use super::*;

/// Values used to expand one logical act into multiple concrete cases.
pub type Matrix<K> = BTreeMap<K, Vec<K>>;
/// Variable bindings passed from a build step into an act.
pub type Bindings<K, E> = BTreeMap<K, E>;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(bound(
    deserialize = "K: Ord + Deserialize<'de>, E: Deserialize<'de>",
    serialize = "K: Serialize, E: Serialize"
))]
/// A reusable build action.
///
/// An `Act` describes the recipe for transforming inputs into outputs. Its
/// labels are used to find the act from a `Build`, but labels are not part of
/// the act content hash.
pub struct Act<K, E> {
    /// Metadata used when a build references this act by label.
    pub labels: LabelMap<K>,
    /// Inputs consumed by this act.
    pub inputs: Vec<Input<E>>,
    /// Transformation applied to the inputs.
    pub map: Map,
    /// Optional variable matrix used to expand this act.
    #[serde(default)]
    pub matrix: Matrix<K>,
    /// Outputs declared by this act.
    #[serde(default)]
    pub outputs: Vec<Output>,
}

impl<K, E> ResourceKind for Act<K, E> {
    const KIND: &'static str = "Act";
}

impl<K, E> HashContent for Act<K, E>
where
    K: HashContent,
    E: HashContent,
{
    fn hash_content(&self, state: &mut Sha512) {
        state.field("inputs");
        self.inputs.hash_content(state);
        state.field("map");
        self.map.hash_content(state);
        state.field("matrix");
        self.matrix.hash_content(state);
        state.field("outputs");
        self.outputs.hash_content(state);
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(bound(
    deserialize = "K: Ord + Deserialize<'de>, E: Deserialize<'de>",
    serialize = "K: Serialize, E: Serialize"
))]
/// A build step that references an act and binds values for it.
pub struct Step<K, E> {
    /// Step identifier within a build.
    pub id: K,
    /// Label selector used to choose the act for this step.
    pub act: ActRef<K>,
    /// Values passed to the selected act.
    #[serde(default)]
    pub with: Bindings<K, E>,
    /// Step identifiers that must finish before this step can run.
    #[serde(default)]
    pub needs: Vec<K>,
}

impl<K, E> HashContent for Step<K, E>
where
    K: HashContent,
    E: HashContent,
{
    fn hash_content(&self, state: &mut Sha512) {
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(bound(deserialize = "K: Ord + Deserialize<'de>", serialize = "K: Serialize"))]
#[serde(transparent)]
/// A label selector for an act.
pub struct ActRef<K>(pub LabelMap<K>);

impl<K> HashContent for ActRef<K>
where
    K: HashContent,
{
    fn hash_content(&self, state: &mut Sha512) {
        self.0.hash_content(state);
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(bound(deserialize = "E: Deserialize<'de>", serialize = "E: Serialize"))]
#[serde(rename_all = "camelCase")]
#[serde(tag = "typ")]
/// An input source consumed by an act.
pub enum Input<E> {
    /// Fetch input from an HTTP URL expression.
    Http { url: E },
}

impl<E> HashContent for Input<E>
where
    E: HashContent,
{
    fn hash_content(&self, state: &mut Sha512) {
        match self {
            Input::Http { url } => {
                state.text("http");
                state.field("url");
                url.hash_content(state);
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(tag = "typ")]
/// The transformation performed by an act.
pub enum Map {
    /// do noting to input, pass it to output directly.
    Identity,
    /// run a shell script to transform input to output.
    Run,
    /// compress with zstd.
    Zstd,
    /// fetch from HTTP endpoint
    Http,
}

impl HashContent for Map {
    fn hash_content(&self, state: &mut Sha512) {
        state.text(self.name());
    }
}

impl Map {
    pub(crate) fn name(&self) -> &'static str {
        match self {
            Map::Identity => "identity",
            Map::Run => "run",
            Map::Zstd => "zstd",
            Map::Http => "http",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(tag = "typ")]
/// An output declared by an act.
pub enum Output {}

impl HashContent for Output {
    fn hash_content(&self, _state: &mut Sha512) {
        match *self {}
    }
}
