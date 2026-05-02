use super::*;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Act {
    pub labels: LabelMap,
    pub inputs: Vec<Input>,
    pub map: Map,
    pub outputs: Vec<Output>,
}

impl AnyResource for Act {
    const KIND: &'static str = "Act";
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Input {}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Map {
    /// do noting to input, pass it to output directly.
    Identity,
    /// run a shell script to transform input to output.
    Run,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Output {}
