use super::*;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Package {
    pub labels: LabelMap,
}

impl ResourceKind for Package {
    const KIND: &'static str = "Package";
}

impl HashContent for Package {
    fn hash_content(&self, state: &mut sha2::Sha256) {
        hash_field(state, "labels");
        self.labels.hash_content(state);
    }
}
