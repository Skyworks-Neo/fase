use super::*;

use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
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
        hash_field(state, "resources");
        hash_len(state, self.resources.len());
        for resource in &self.resources {
            hash_str(state, &resource.to_string_lossy());
        }

        hash_field(state, "labels");
        hash_len(state, self.labels.len());
        for labels in &self.labels {
            labels.hash_content(state);
        }
    }
}
