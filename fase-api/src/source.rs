use super::*;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Source {
    pub labels: LabelMap,
}

impl AnyResource for Source {
    const KIND: &'static str = "Source";
}
