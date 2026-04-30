use super::*;

#[derive(Debug, Clone, Deserialize)]
pub struct Source {
    pub labels: Label,
}

impl AnyResource for Source {
    const API_VERSION: &'static str = "v1alpha1";
    const KIND: &'static str = "Source";
}
