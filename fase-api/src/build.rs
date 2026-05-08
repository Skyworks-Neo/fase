use super::*;

#[derive(Debug, Clone, Serialize, Deserialize)]
/// User-authored build intent.
///
/// A `Build` references acts by labels and describes the step graph the user
/// wants to run. It is not pinned to concrete act hashes until it is realized.
pub struct Build {
    /// Metadata used to select or organize this build.
    pub labels: LabelMap,
    /// Ordered step declarations for the build graph.
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
