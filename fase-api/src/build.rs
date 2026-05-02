use super::*;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Build {
    pub labels: LabelMap,
}

impl AnyResource for Build {
    const KIND: &'static str = "Build";
}
