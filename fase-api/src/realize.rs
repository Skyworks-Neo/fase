use super::*;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Realize {
    pub labels: LabelMap,
    pub build: ShaSum,
    pub steps: Vec<RealizeStep>,
}

impl ResourceKind for Realize {
    const KIND: &'static str = "Realize";
}

impl HashContent for Realize {
    fn hash_content(&self, state: &mut sha2::Sha256) {
        hash_field(state, "build");
        self.build.hash_content(state);
        hash_field(state, "steps");
        hash_len(state, self.steps.len());
        for step in &self.steps {
            step.hash_content(state);
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RealizeStep {
    pub id: Var,
    pub act: ShaSum,
    #[serde(default)]
    pub with: Bindings,
    #[serde(default)]
    pub needs: Vec<Var>,
}

impl HashContent for RealizeStep {
    fn hash_content(&self, state: &mut sha2::Sha256) {
        hash_field(state, "id");
        self.id.hash_content(state);
        hash_field(state, "act");
        self.act.hash_content(state);
        hash_field(state, "with");
        self.with.hash_content(state);
        hash_field(state, "needs");
        hash_len(state, self.needs.len());
        for need in &self.needs {
            need.hash_content(state);
        }
    }
}
