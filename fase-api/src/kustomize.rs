use super::*;

use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(bound(deserialize = "K: Ord + Deserialize<'de>", serialize = "K: Serialize"))]
/// A kustomization-like resource collection.
///
/// `Kustomize` points at resource files or directories and describes labels to
/// apply to collected resources.
pub struct Kustomize<K> {
    /// Paths to resource files or directories.
    pub resources: Vec<PathBuf>,
    /// Labels added to all resources collected by this Kustomize.
    #[serde(default)]
    pub labels: Vec<LabelMap<K>>,
}

impl<K> ResourceKind for Kustomize<K> {
    const KIND: &'static str = "Kustomize";
}

impl<K> HashContent for Kustomize<K>
where
    K: HashContent,
{
    fn hash_content(&self, state: &mut Sha512) {
        state.field("resources");
        self.resources.hash_content(state);
        state.field("labels");
        self.labels.hash_content(state);
    }
}
