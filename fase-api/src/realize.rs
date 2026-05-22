use super::*;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(bound(
    deserialize = "K: Ord + Deserialize<'de>, E: Deserialize<'de>",
    serialize = "K: Serialize, E: Serialize"
))]
/// A concrete build graph produced from a `Build`.
///
/// `Realize` records which acts were selected and which bindings will be used.
/// It is a resolved plan, not an execution cache record; layer cache keys belong
/// to a separate cache/layer model.
pub struct Realize<K, E> {
    /// Metadata used to select or organize this realized graph.
    pub labels: LabelMap<K>,
    /// Content hash of the source `Build`.
    pub build: ShaSum,
    /// Concrete steps after act selection and binding expansion.
    pub steps: Vec<RealizeStep<K, E>>,
}

impl<K, E> ResourceKind for Realize<K, E> {
    const KIND: &'static str = "Realize";
}

impl<K, E> HashContent for Realize<K, E>
where
    K: HashContent,
    E: HashContent,
{
    fn hash_content(&self, state: &mut Sha256) {
        state.field("build");
        self.build.hash_content(state);
        state.field("steps");
        self.steps.hash_content(state);
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(bound(
    deserialize = "K: Ord + Deserialize<'de>, E: Deserialize<'de>",
    serialize = "K: Serialize, E: Serialize"
))]
/// A concrete step in a realized build graph.
pub struct RealizeStep<K, E> {
    /// Step identifier within the realized graph.
    pub id: K,
    /// Content hash of the selected act.
    pub act: ShaSum,
    /// Values passed to the selected act.
    #[serde(default)]
    pub with: Bindings<K, E>,
    /// Step identifiers that must finish before this step can run.
    #[serde(default)]
    pub needs: Vec<K>,
}

impl<K, E> HashContent for RealizeStep<K, E>
where
    K: HashContent,
    E: HashContent,
{
    fn hash_content(&self, state: &mut Sha256) {
        state.field("id");
        self.id.hash_content(state);
        state.field("act");
        self.act.hash_content(state);
        state.field("with");
        self.with.hash_content(state);
        state.field("needs");
        self.needs.hash_content(state);
    }
}
