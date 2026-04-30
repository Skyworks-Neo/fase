use super::*;

use std::path::PathBuf;

#[derive(Debug, Clone, Deserialize)]
pub struct Kustomize {
    pub resources: Vec<PathBuf>,
}

impl AnyResource for Kustomize {
    const API_VERSION: &'static str = "v1alpha1";
    const KIND: &'static str = "Kustomize";
}
