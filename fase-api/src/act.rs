use super::*;

pub type Matrix = BTreeMap<Var, Vec<Var>>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Act {
    pub inputs: Vec<Input>,
    pub map: Map,
    #[serde(default)]
    pub matrix: Matrix,
    #[serde(default)]
    pub outputs: Vec<Output>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(tag = "typ")]
pub enum Input {
    Http { url: Url },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(tag = "typ")]
pub enum Map {
    /// do noting to input, pass it to output directly.
    Identity,
    /// run a shell script to transform input to output.
    Run,
    /// compress with zstd.
    Zstd,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(tag = "typ")]
pub enum Output {}
