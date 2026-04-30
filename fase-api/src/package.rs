use super::*;

#[derive(Debug, Clone, Deserialize)]
pub struct Package {
    pub labels: Label,
}

impl AnyResource for Package {
    const API_VERSION: &'static str = "v1alpha1";
    const KIND: &'static str = "Package";
}
