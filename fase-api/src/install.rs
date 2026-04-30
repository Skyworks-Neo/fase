use super::*;

#[derive(Debug, Clone, Deserialize)]
pub struct Install {
    pub wants: Vec<Label>,
}

impl AnyResource for Install {
    const API_VERSION: &'static str = "v1alpha1";
    const KIND: &'static str = "Install";
}
