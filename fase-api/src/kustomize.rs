use super::*;

use std::path::PathBuf;

#[derive(Debug, Clone, Deserialize)]
pub struct Kustomize {
    pub resources: Vec<PathBuf>,
}

impl AnyResource for Kustomize {
    const KIND: &'static str = "Kustomize";
}
