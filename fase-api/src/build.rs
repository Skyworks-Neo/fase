use super::*;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Build {
    pub labels: LabelMap,
    pub steps: Vec<Step>,
}

impl ResourceKind for Build {
    const KIND: &'static str = "Build";
}

impl HashContent for Build {
    fn hash_content(&self, state: &mut sha2::Sha256) {
        hash_field(state, "steps");
        hash_len(state, self.steps.len());
        for step in &self.steps {
            step.hash_content(state);
        }
    }
}
