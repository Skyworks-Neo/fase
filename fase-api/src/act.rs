use super::*;

pub type Matrix = BTreeMap<Var, Vec<Var>>;
pub type Bindings = BTreeMap<Var, Expr>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Act {
    pub labels: LabelMap,
    pub inputs: Vec<Input>,
    pub map: Map,
    #[serde(default)]
    pub matrix: Matrix,
    #[serde(default)]
    pub outputs: Vec<Output>,
}

impl ResourceKind for Act {
    const KIND: &'static str = "Act";
}

impl HashContent for Act {
    fn hash_content(&self, state: &mut sha2::Sha256) {
        hash_field(state, "labels");
        self.labels.hash_content(state);
        hash_field(state, "inputs");
        hash_len(state, self.inputs.len());
        for input in &self.inputs {
            input.hash_content(state);
        }
        hash_field(state, "map");
        self.map.hash_content(state);
        hash_field(state, "matrix");
        self.matrix.hash_content(state);
        hash_field(state, "outputs");
        hash_len(state, self.outputs.len());
        for output in &self.outputs {
            output.hash_content(state);
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Step {
    pub id: Var,
    pub act: ActRef,
    #[serde(default)]
    pub with: Bindings,
    #[serde(default)]
    pub needs: Vec<Var>,
}

impl HashContent for Step {
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ActRef(pub LabelMap);

impl HashContent for ActRef {
    fn hash_content(&self, state: &mut sha2::Sha256) {
        self.0.hash_content(state);
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Expr(Box<str>);

impl Expr {
    pub fn expand(&self, vars: &BTreeMap<Var, String>) -> Result<String, ExprError> {
        let mut expanded = String::new();
        let mut rest = self.0.as_ref();
        while let Some(start) = rest.find("${") {
            expanded.push_str(&rest[..start]);
            rest = &rest[start + 2..];
            let Some(end) = rest.find('}') else {
                return Err(ExprError::UnclosedVariable {
                    expr: self.0.clone(),
                });
            };
            let name = Var::new(&rest[..end]).map_err(|_| ExprError::InvalidVariable {
                name: rest[..end].into(),
            })?;
            let value = vars
                .get(&name)
                .ok_or_else(|| ExprError::UnknownVariable { name: name.clone() })?;
            expanded.push_str(value);
            rest = &rest[end + 1..];
        }
        expanded.push_str(rest);
        Ok(expanded)
    }
}

// impl Serialize for Expr {
//     fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
//     where
//         S: serde::Serializer,
//     {
//         serializer.serialize_str(&self.0)
//     }
// }

// impl<'de> Deserialize<'de> for Expr {
//     fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
//     where
//         D: serde::Deserializer<'de>,
//     {
//         Ok(Self(<Box<str>>::deserialize(deserializer)?))
//     }
// }

#[derive(Debug)]
pub enum ExprError {
    UnclosedVariable { expr: Box<str> },
    InvalidVariable { name: Box<str> },
    UnknownVariable { name: Var },
}

impl std::fmt::Display for ExprError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExprError::UnclosedVariable { expr } => write!(f, "unclosed variable in {expr}"),
            ExprError::InvalidVariable { name } => write!(f, "{name} is not a valid variable name"),
            ExprError::UnknownVariable { name } => write!(f, "{name} is not defined"),
        }
    }
}

impl std::error::Error for ExprError {}

impl HashContent for Expr {
    fn hash_content(&self, state: &mut sha2::Sha256) {
        hash_str(state, &self.0);
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(tag = "typ")]
pub enum Input {
    Http { url: Expr },
}

impl HashContent for Input {
    fn hash_content(&self, state: &mut sha2::Sha256) {
        match self {
            Input::Http { url } => {
                hash_str(state, "http");
                hash_field(state, "url");
                url.hash_content(state);
            }
        }
    }
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

impl HashContent for Map {
    fn hash_content(&self, state: &mut sha2::Sha256) {
        hash_str(
            state,
            match self {
                Map::Identity => "identity",
                Map::Run => "run",
                Map::Zstd => "zstd",
            },
        );
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(tag = "typ")]
pub enum Output {}

impl HashContent for Output {
    fn hash_content(&self, _state: &mut sha2::Sha256) {
        match *self {}
    }
}

impl HashContent for Matrix {
    fn hash_content(&self, state: &mut sha2::Sha256) {
        hash_len(state, self.len());
        for (key, values) in self {
            key.hash_content(state);
            hash_len(state, values.len());
            for value in values {
                value.hash_content(state);
            }
        }
    }
}

impl HashContent for Bindings {
    fn hash_content(&self, state: &mut sha2::Sha256) {
        hash_len(state, self.len());
        for (key, value) in self {
            key.hash_content(state);
            value.hash_content(state);
        }
    }
}
