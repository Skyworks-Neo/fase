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

pub use act::{Act, ActRef, Bindings, Input, Map, Matrix, Output, Step};
pub use build::Build;
pub use install::Install;
pub use kustomize::Kustomize;
pub use label::{Label, LabelPool};
pub use lasso::Spur;
pub use package::Package;
pub use realize::{Realize, RealizeStep};
pub use sha::{HashContent, Sha, ShaParseError, ShaSum};

use serde::{Deserialize, Serialize, de::Error as DeError};
use sha2::Digest;

use std::collections::BTreeMap;

use sha::HashWrite;

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

impl<K> LabelMap<K> {
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = (&K, &K)> {
        self.inner.iter()
    }
}

impl<K> LabelMap<K>
where
    K: Clone + Ord,
{
    pub fn merge(&mut self, other: &Self) {
        self.inner.extend(
            other
                .iter()
                .map(|(key, value)| (key.clone(), value.clone())),
        );
    }
}

trait ResourceKind {
    const API_VERSION: &'static str = "v1alpha1";
    const KIND: &'static str;

    fn matches(kind: &str, api_version: &str) -> bool {
        kind == Self::KIND && api_version == Self::API_VERSION
    }
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

impl<K, E> Resource<K, E> {
    pub fn labels_mut(&mut self) -> Option<&mut LabelMap<K>> {
        match self {
            Resource::Act(resource) => Some(&mut resource.labels),
            Resource::Package(resource) => Some(&mut resource.labels),
            Resource::Build(resource) => Some(&mut resource.labels),
            Resource::Realize(resource) => Some(&mut resource.labels),
            Resource::Kustomize(_) | Resource::Install(_) => None,
        }
    }

    pub fn merge_labels(&mut self, labels: &LabelMap<K>)
    where
        K: Clone + Ord,
    {
        if let Some(resource_labels) = self.labels_mut() {
            resource_labels.merge(labels);
        }
    }

    fn decode<'de, D, T, F>(value: serde_value::Value, wrap: F) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
        T: Deserialize<'de>,
        F: FnOnce(T) -> Self,
    {
        T::deserialize(value).map(wrap).map_err(D::Error::custom)
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Header {
    api_version: String,
    kind: String,
}

impl Header {
    fn is<R: ResourceKind>(&self) -> bool {
        R::matches(&self.kind, &self.api_version)
    }
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
        let value = <serde_value::Value as serde::Deserialize>::deserialize(deserializer)?;

        let Ok(header) = Header::deserialize(value.clone()) else {
            return Self::decode::<D, Kustomize<K>, _>(value, Self::Kustomize);
        };

        if header.is::<Act<K, E>>() {
            Self::decode::<D, Act<K, E>, _>(value, Self::Act)
        } else if header.is::<Package<K>>() {
            Self::decode::<D, Package<K>, _>(value, Self::Package)
        } else if header.is::<Install<K>>() {
            Self::decode::<D, Install<K>, _>(value, Self::Install)
        } else if header.is::<Kustomize<K>>() {
            Self::decode::<D, Kustomize<K>, _>(value, Self::Kustomize)
        } else if header.is::<Build<K, E>>() {
            Self::decode::<D, Build<K, E>, _>(value, Self::Build)
        } else if header.is::<Realize<K, E>>() {
            Self::decode::<D, Realize<K, E>, _>(value, Self::Realize)
        } else {
            Err(D::Error::custom(format!(
                "kind={} apiVersion={} is not supported",
                header.kind, header.api_version
            )))
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Envelope<'a, T: ?Sized> {
    api_version: &'static str,
    kind: &'static str,
    #[serde(flatten)]
    body: &'a T,
}

fn envelope<R, S>(resource: &R, serializer: S) -> Result<S::Ok, S::Error>
where
    R: ResourceKind + Serialize,
    S: serde::Serializer,
{
    Envelope {
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
            Resource::Act(resource) => envelope(resource, serializer),
            Resource::Package(resource) => envelope(resource, serializer),
            Resource::Kustomize(resource) => envelope(resource, serializer),
            Resource::Install(resource) => envelope(resource, serializer),
            Resource::Build(resource) => envelope(resource, serializer),
            Resource::Realize(resource) => envelope(resource, serializer),
        }
    }
}
