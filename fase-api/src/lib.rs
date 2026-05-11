#![doc = include_str!("../README.md")]

mod act;
mod build;
mod install;
mod kustomize;
mod label;
mod package;
mod realize;
mod sha;

#[cfg(test)]
mod test;

pub use sha2::Sha512;

use serde::{Deserialize, Serialize};
use sha2::Digest;

use std::collections::BTreeMap;

pub use act::{Act, ActRef, Bindings, Input, Map, Matrix, Output, Step};
pub use build::Build;
pub use install::Install;
pub use kustomize::Kustomize;
pub use label::{Label, LabelPool};
pub use lasso::Spur;
pub use package::Package;
pub use realize::{Realize, RealizeStep};
pub use sha::{HashContent, Sha, ShaSum, hash_field, hash_len, hash_str};

#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(bound(deserialize = "K: Ord + Deserialize<'de>", serialize = "K: Serialize"))]
#[serde(transparent)]
/// Labels attached to resources or used as selectors.
///
/// `LabelMap` keeps keys sorted so serialization-independent operations such as
/// hashing see a stable order.
pub struct LabelMap<K> {
    inner: BTreeMap<K, K>,
}

trait ResourceKind {
    const API_VERSION: &'static str = "v1alpha1";
    const KIND: &'static str;
}

#[derive(Debug, Clone)]
/// Any supported Fase resource.
///
/// This enum is the format-dispatch type used when deserializing documents with
/// `apiVersion` and `kind` headers.
pub enum Resource<K, E> {
    /// A reusable build action.
    Act(Act<K, E>),
    /// A selected or produced package.
    Package(Package<K>),
    /// A kustomization-like resource collection.
    Kustomize(Kustomize<K>),
    /// A package installation request.
    Install(Install<K>),
    /// User-authored build intent.
    Build(Build<K, E>),
    /// Concrete build graph produced from a `Build`.
    Realize(Realize<K, E>),
}

impl<'de, K, E> serde::Deserialize<'de> for Resource<K, E>
where
    K: Ord + serde::Deserialize<'de>,
    E: serde::Deserialize<'de>,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(serde::Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Header {
            api_version: String,
            kind: String,
        }
        let value = <serde_value::Value as serde::Deserialize>::deserialize(deserializer)?;
        if let Ok(header) = <Header as serde::Deserialize>::deserialize(value.clone())
            .map_err(<D::Error as serde::de::Error>::custom)
        {
            match (header.kind.as_str(), header.api_version.as_str()) {
                (Act::<K, E>::KIND, Act::<K, E>::API_VERSION) => <Act<K, E>>::deserialize(value)
                    .map(Resource::Act)
                    .map_err(<D::Error as serde::de::Error>::custom),
                (Package::<K>::KIND, Package::<K>::API_VERSION) => <Package<K>>::deserialize(value)
                    .map(Resource::Package)
                    .map_err(<D::Error as serde::de::Error>::custom),
                (Install::<K>::KIND, Install::<K>::API_VERSION) => <Install<K>>::deserialize(value)
                    .map(Resource::Install)
                    .map_err(<D::Error as serde::de::Error>::custom),
                (Kustomize::<K>::KIND, Kustomize::<K>::API_VERSION) => {
                    <Kustomize<K>>::deserialize(value)
                        .map(Resource::Kustomize)
                        .map_err(<D::Error as serde::de::Error>::custom)
                }
                (Build::<K, E>::KIND, Build::<K, E>::API_VERSION) => {
                    <Build<K, E>>::deserialize(value)
                        .map(Resource::Build)
                        .map_err(<D::Error as serde::de::Error>::custom)
                }
                (Realize::<K, E>::KIND, Realize::<K, E>::API_VERSION) => {
                    <Realize<K, E>>::deserialize(value)
                        .map(Resource::Realize)
                        .map_err(<D::Error as serde::de::Error>::custom)
                }
                (kind, ver) => Err(<D::Error as serde::de::Error>::custom(format!(
                    "kind={kind} apiVersion={ver} is not supported"
                ))),
            }
        }
        // try to deserialize as Kustomize
        else {
            <Kustomize<K>>::deserialize(value)
                .map(Resource::Kustomize)
                .map_err(<D::Error as serde::de::Error>::custom)
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ResourceEnvelope<'a, T: ?Sized> {
    api_version: &'static str,
    kind: &'static str,
    #[serde(flatten)]
    body: &'a T,
}

fn serialize_with_header<R, S>(resource: &R, serializer: S) -> Result<S::Ok, S::Error>
where
    R: ResourceKind + Serialize,
    S: serde::Serializer,
{
    ResourceEnvelope {
        api_version: R::API_VERSION,
        kind: R::KIND,
        body: resource,
    }
    .serialize(serializer)
}

impl<K, E> Serialize for Resource<K, E>
where
    K: Serialize,
    E: Serialize,
{
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self {
            Resource::Act(resource) => serialize_with_header(resource, serializer),
            Resource::Package(resource) => serialize_with_header(resource, serializer),
            Resource::Kustomize(resource) => serialize_with_header(resource, serializer),
            Resource::Install(resource) => serialize_with_header(resource, serializer),
            Resource::Build(resource) => serialize_with_header(resource, serializer),
            Resource::Realize(resource) => serialize_with_header(resource, serializer),
        }
    }
}
