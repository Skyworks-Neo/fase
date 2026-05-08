use super::*;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Package {
    pub labels: LabelMap,
}

impl ResourceKind for Package {
    const KIND: &'static str = "Package";
}

impl HashContent for Package {
    fn hash_content(&self, _state: &mut sha2::Sha256) {}
}
