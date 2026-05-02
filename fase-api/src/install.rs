use super::*;

#[derive(Debug, Clone, Deserialize)]
pub struct Install {
    /// Label selectors that candidate packages must match.
    pub must: Vec<LabelMap>,
    /// Label selectors that increase a matching package's priority.
    pub prefer: Vec<LabelMap>,
}

impl AnyResource for Install {
    const KIND: &'static str = "Install";
}
