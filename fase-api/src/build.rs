use super::*;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Build {
    pub labels: LabelMap,
    pub sources: LabelMap,
}

impl AnyResource for Build {
    const KIND: &'static str = "Build";
}
