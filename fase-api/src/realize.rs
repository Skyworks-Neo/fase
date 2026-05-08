use super::*;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
/// A concrete build graph produced from a `Build`.
///
/// `Realize` records which acts were selected and which bindings will be used.
/// It is a resolved plan, not an execution cache record; layer cache keys belong
/// to a separate cache/layer model.
pub struct Realize {
    /// Metadata used to select or organize this realized graph.
    pub labels: LabelMap,
    /// Content hash of the source `Build`.
    pub build: ShaSum,
    /// Concrete steps after act selection and binding expansion.
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
/// A concrete step in a realized build graph.
pub struct RealizeStep {
    /// Step identifier within the realized graph.
    pub id: Var,
    /// Content hash of the selected act.
    pub act: ShaSum,
    /// Values passed to the selected act.
    #[serde(default)]
    pub with: Bindings,
    /// Step identifiers that must finish before this step can run.
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
