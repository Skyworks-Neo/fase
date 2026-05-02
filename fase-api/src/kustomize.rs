use super::*;

use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Kustomize {
    pub resources: Vec<PathBuf>,
    #[serde(default)]
    /// Labels added to all resources collected by this Kustomize.
    pub labels: Vec<LabelMap>,
}

impl AnyResource for Kustomize {
    const KIND: &'static str = "Kustomize";
}
