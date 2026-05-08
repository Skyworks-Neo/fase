use super::*;

#[derive(Debug, Clone, Serialize, Deserialize)]
/// A package selected or produced by Fase.
///
/// A `Package` is currently identified only by labels. Labels are metadata used
/// for selection and do not contribute to the package content hash.
pub struct Package {
    /// Metadata used to select this package.
    pub labels: LabelMap,
}

impl ResourceKind for Package {
    const KIND: &'static str = "Package";
}

impl HashContent for Package {
    fn hash_content(&self, _state: &mut sha2::Sha256) {}
}
