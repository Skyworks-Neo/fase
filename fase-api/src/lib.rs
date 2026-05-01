mod build;
mod install;
mod kustomize;
mod package;
mod source;
mod var;

#[cfg(test)]
mod test;

use serde::{Deserialize, Serialize};

use std::collections::BTreeMap;

pub use build::Build;
pub use install::Install;
pub use kustomize::Kustomize;
pub use package::Package;
pub use source::Source;
pub use var::{Var, VarError};

#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(transparent)]
pub struct LabelMap {
    inner: BTreeMap<Var, Var>,
}

trait AnyResource {
    const API_VERSION: &'static str = "v1alpha1";
    const KIND: &'static str;
}

#[derive(Debug, Clone)]
pub enum Resource {
    Package(Package),
    Source(Source),
    Kustomize(Kustomize),
    Install(Install),
    Build(Build),
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
                (Package::KIND, Package::API_VERSION) => <Package>::deserialize(value)
                    .map(Resource::Package)
                    .map_err(<D::Error as serde::de::Error>::custom),
                (Source::KIND, Source::API_VERSION) => <Source>::deserialize(value)
                    .map(Resource::Source)
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
