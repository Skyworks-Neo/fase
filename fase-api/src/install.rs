use super::*;

#[derive(Debug, Clone, Serialize, Deserialize)]
/// A package installation request.
///
/// `Install` describes selectors for choosing packages. Unlike resource
/// metadata labels, these selector fields are part of install content.
pub struct Install {
    /// Label selectors that candidate packages must match.
    pub must: Vec<LabelMap>,
    /// Label selectors that increase a matching package's priority.
    pub prefer: Vec<LabelMap>,
}

impl ResourceKind for Install {
    const KIND: &'static str = "Install";
}

impl HashContent for Install {
    fn hash_content(&self, state: &mut sha2::Sha256) {
        hash_field(state, "must");
        hash_len(state, self.must.len());
        for labels in &self.must {
            labels.hash_content(state);
        }

        hash_field(state, "prefer");
        hash_len(state, self.prefer.len());
        for labels in &self.prefer {
            labels.hash_content(state);
        }
    }
}
