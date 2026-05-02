use super::*;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Package {
    pub labels: LabelMap,
}

impl AnyResource for Package {
    const KIND: &'static str = "Package";
}
