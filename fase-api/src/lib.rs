#![doc = include_str!("../README.md")]

mod act;
mod build;
mod install;
mod kustomize;
mod package;
mod realize;
mod sha;
mod var;

#[cfg(test)]
mod test;

use serde::{Deserialize, Serialize};
use sha2::Digest;

pub use url::Url;

use std::collections::BTreeMap;

pub use act::{Act, ActRef, Bindings, Expr, ExprError, Matrix, Step};
pub use build::Build;
pub use install::Install;
pub use kustomize::Kustomize;
pub use package::Package;
pub use realize::{Realize, RealizeStep};
pub use sha::{Sha, ShaSum};
pub use var::{Var, VarError};

use sha::{HashContent, hash_field, hash_len, hash_str};

#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(transparent)]
/// Labels attached to resources or used as selectors.
///
/// `LabelMap` keeps keys sorted so serialization-independent operations such as
/// hashing see a stable order.
pub struct LabelMap {
    inner: BTreeMap<Var, Var>,
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
pub enum Resource {
    /// A reusable build action.
    Act(Act),
    /// A selected or produced package.
    Package(Package),
    /// A kustomization-like resource collection.
    Kustomize(Kustomize),
    /// A package installation request.
    Install(Install),
    /// User-authored build intent.
    Build(Build),
    /// Concrete build graph produced from a `Build`.
    Realize(Realize),
}

impl<'de> serde::Deserialize<'de> for Resource {
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
                (Act::KIND, Act::API_VERSION) => <Act>::deserialize(value)
                    .map(Resource::Act)
                    .map_err(<D::Error as serde::de::Error>::custom),
                (Package::KIND, Package::API_VERSION) => <Package>::deserialize(value)
                    .map(Resource::Package)
                    .map_err(<D::Error as serde::de::Error>::custom),
                (Install::KIND, Install::API_VERSION) => <Install>::deserialize(value)
                    .map(Resource::Install)
                    .map_err(<D::Error as serde::de::Error>::custom),
                (Kustomize::KIND, Kustomize::API_VERSION) => <Kustomize>::deserialize(value)
                    .map(Resource::Kustomize)
                    .map_err(<D::Error as serde::de::Error>::custom),
                (Build::KIND, Build::API_VERSION) => <Build>::deserialize(value)
                    .map(Resource::Build)
                    .map_err(<D::Error as serde::de::Error>::custom),
                (Realize::KIND, Realize::API_VERSION) => <Realize>::deserialize(value)
                    .map(Resource::Realize)
                    .map_err(<D::Error as serde::de::Error>::custom),
                (kind, ver) => Err(<D::Error as serde::de::Error>::custom(format!(
                    "kind={kind} apiVersion={ver} is not supported"
                ))),
            }
        }
        // try to deserialize as Kustomize
        else {
            <Kustomize>::deserialize(value)
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

impl Serialize for Resource {
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
