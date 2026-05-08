use super::*;

use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
/// A kustomization-like resource collection.
///
/// `Kustomize` points at resource files or directories and describes labels to
/// apply to collected resources.
pub struct Kustomize {
    /// Paths to resource files or directories.
    pub resources: Vec<PathBuf>,
    /// Labels added to all resources collected by this Kustomize.
    #[serde(default)]
    pub labels: Vec<LabelMap>,
}

impl ResourceKind for Kustomize {
    const KIND: &'static str = "Kustomize";
}

impl HashContent for Kustomize {
    fn hash_content(&self, state: &mut sha2::Sha256) {
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
