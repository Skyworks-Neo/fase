use super::*;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Build {
    pub labels: LabelMap,
    pub steps: Vec<Step>,
}

impl AnyResource for Build {
    const KIND: &'static str = "Build";
}
