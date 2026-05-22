use super::*;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(bound(deserialize = "K: Ord + Deserialize<'de>", serialize = "K: Serialize"))]
/// A package installation request.
///
/// `Install` describes selectors for choosing packages. Unlike resource
/// metadata labels, these selector fields are part of install content.
pub struct Install<K> {
    /// Label selectors that candidate packages must match.
    pub must: Vec<LabelMap<K>>,
    /// Label selectors that increase a matching package's priority.
    pub prefer: Vec<LabelMap<K>>,
}

impl<K> ResourceKind for Install<K> {
    const KIND: &'static str = "Install";
}

impl<K> HashContent for Install<K>
where
    K: HashContent,
{
    fn hash_content(&self, state: &mut Sha256) {
        state.field("must");
        self.must.hash_content(state);
        state.field("prefer");
        self.prefer.hash_content(state);
    }
}
