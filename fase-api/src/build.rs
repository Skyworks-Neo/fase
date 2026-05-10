use super::*;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(bound(
    deserialize = "K: Ord + Deserialize<'de>, E: Deserialize<'de>",
    serialize = "K: Serialize, E: Serialize"
))]
/// User-authored build intent.
///
/// A `Build` references acts by labels and describes the step graph the user
/// wants to run. It is not pinned to concrete act hashes until it is realized.
pub struct Build<K, E> {
    /// Metadata used to select or organize this build.
    pub labels: LabelMap<K>,
    /// Ordered step declarations for the build graph.
    pub steps: Vec<Step<K, E>>,
}

impl<K, E> ResourceKind for Build<K, E> {
    const KIND: &'static str = "Build";
}

impl<K, E> HashContent for Build<K, E>
where
    K: HashContent,
    E: HashContent,
{
    fn hash_content(&self, state: &mut Sha512) {
        hash_field(state, "steps");
        hash_len(state, self.steps.len());
        for step in &self.steps {
            step.hash_content(state);
        }
    }
}
