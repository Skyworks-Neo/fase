use super::*;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(bound(deserialize = "K: Ord + Deserialize<'de>", serialize = "K: Serialize"))]
/// A package selected or produced by Fase.
///
/// A `Package` is currently identified only by labels. Labels are metadata used
/// for selection and do not contribute to the package content hash.
pub struct Package<K> {
    /// Metadata used to select this package.
    pub labels: LabelMap<K>,
}

impl<K> ResourceKind for Package<K> {
    const KIND: &'static str = "Package";
}

impl<K> HashContent for Package<K> {
    fn hash_content(&self, _state: &mut Sha256) {}
}
