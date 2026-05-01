use super::*;

#[derive(Debug, Clone, Deserialize)]
pub struct Build {
    pub labels: LabelMap,
    pub sources: LabelMap,
}

impl AnyResource for Build {
    const KIND: &'static str = "Build";
}
