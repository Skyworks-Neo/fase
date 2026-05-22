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

pub use sha2::Sha256;

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

/// The stable type identity of a Fase API resource.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ApiResource {
    /// API version used in serialized resource documents.
    pub api_version: &'static str,
    /// Resource kind used in serialized resource documents.
    pub kind: &'static str,
}

impl<R> From<&R> for ApiResource
where
    R: ResourceKind,
{
    fn from(_: &R) -> Self {
        Self {
            api_version: R::API_VERSION,
            kind: R::KIND,
        }
    }
}

trait ResourceKind {
    const API_VERSION: &'static str = "v1alpha1";
    const KIND: &'static str;

    fn matches(kind: &str, api_version: &str) -> bool {
        kind == Self::KIND && api_version == Self::API_VERSION
    }
}

/// Labels attached to resources or used as selectors.
///
/// `LabelMap` keeps keys sorted so serialization-independent operations such as
/// hashing see a stable order.
///
/// `K` is the label key and label value representation. Raw YAML can use an
/// owned string type, while the CLI uses [`Label`] to intern repeated names.
#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(bound(deserialize = "K: Ord + Deserialize<'de>", serialize = "K: Serialize"))]
#[serde(transparent)]
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

    pub fn map<T, F>(self, mut f: F) -> LabelMap<T>
    where
        T: Ord,
        F: FnMut(K) -> T,
    {
        LabelMap {
            inner: self
                .inner
                .into_iter()
                .map(|(key, value)| {
                    let key = f(key);
                    let value = f(value);
                    (key, value)
                })
                .collect(),
        }
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

/// Any supported Fase resource.
///
/// This enum is the format-dispatch type used when deserializing documents with
/// `apiVersion` and `kind` headers.
///
/// `K` is the identifier type used for resource labels, selectors, matrix keys,
/// step IDs, and dependency names. Use a string-like type when preserving raw
/// documents, or [`Label`] when repeated names should be interned.
///
/// `E` is the expression/value type used in places that may be expanded at build
/// time, such as input paths, URLs, environment variable names, and step
/// bindings. Keeping it separate from `K` lets callers choose compact labels
/// while still storing expressions as ordinary strings.
#[derive(Debug, Clone)]
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

impl<K, E> From<&Resource<K, E>> for ApiResource {
    fn from(resource: &Resource<K, E>) -> Self {
        match resource {
            Resource::Act(resource) => resource.into(),
            Resource::Package(resource) => resource.into(),
            Resource::Kustomize(resource) => resource.into(),
            Resource::Install(resource) => resource.into(),
            Resource::Build(resource) => resource.into(),
            Resource::Realize(resource) => resource.into(),
        }
    }
}

impl<K, E> Resource<K, E> {
    /// Convert the resource key type while leaving expression values unchanged.
    pub fn map<T, F>(self, mut f: F) -> Resource<T, E>
    where
        T: Ord,
        F: FnMut(K) -> T,
    {
        match self {
            Resource::Act(resource) => Resource::Act(Act {
                labels: resource.labels.map(&mut f),
                inputs: resource.inputs,
                map: resource.map,
                matrix: resource
                    .matrix
                    .into_iter()
                    .map(|(key, values)| {
                        let key = f(key);
                        let values = values.into_iter().map(&mut f).collect();
                        (key, values)
                    })
                    .collect(),
                outputs: resource.outputs,
            }),
            Resource::Package(resource) => Resource::Package(Package {
                labels: resource.labels.map(&mut f),
            }),
            Resource::Kustomize(resource) => Resource::Kustomize(Kustomize {
                resources: resource.resources,
                labels: resource
                    .labels
                    .into_iter()
                    .map(|labels| labels.map(&mut f))
                    .collect(),
            }),
            Resource::Install(resource) => Resource::Install(Install {
                must: resource
                    .must
                    .into_iter()
                    .map(|labels| labels.map(&mut f))
                    .collect(),
                prefer: resource
                    .prefer
                    .into_iter()
                    .map(|labels| labels.map(&mut f))
                    .collect(),
            }),
            Resource::Build(resource) => Resource::Build(Build {
                labels: resource.labels.map(&mut f),
                steps: resource
                    .steps
                    .into_iter()
                    .map(|step| Step {
                        id: f(step.id),
                        act: ActRef(step.act.0.map(&mut f)),
                        with: step
                            .with
                            .into_iter()
                            .map(|(key, value)| (f(key), value))
                            .collect(),
                        needs: step.needs.into_iter().map(&mut f).collect(),
                    })
                    .collect(),
            }),
            Resource::Realize(resource) => Resource::Realize(Realize {
                labels: resource.labels.map(&mut f),
                build: resource.build,
                steps: resource
                    .steps
                    .into_iter()
                    .map(|step| RealizeStep {
                        id: f(step.id),
                        act: step.act,
                        with: step
                            .with
                            .into_iter()
                            .map(|(key, value)| (f(key), value))
                            .collect(),
                        needs: step.needs.into_iter().map(&mut f).collect(),
                    })
                    .collect(),
            }),
        }
    }

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
