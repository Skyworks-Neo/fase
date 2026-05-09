use super::*;

/// Values used to expand one logical act into multiple concrete cases.
pub type Matrix = BTreeMap<Var, Vec<Var>>;
/// Variable bindings passed from a build step into an act.
pub type Bindings = BTreeMap<Var, Expr>;

#[derive(Debug, Clone, Serialize, Deserialize)]
/// A reusable build action.
///
/// An `Act` describes the recipe for transforming inputs into outputs. Its
/// labels are used to find the act from a `Build`, but labels are not part of
/// the act content hash.
pub struct Act {
    /// Metadata used when a build references this act by label.
    pub labels: LabelMap,
    /// Inputs consumed by this act.
    pub inputs: Vec<Input>,
    /// Transformation applied to the inputs.
    pub map: Map,
    /// Optional variable matrix used to expand this act.
    #[serde(default)]
    pub matrix: Matrix,
    /// Outputs declared by this act.
    #[serde(default)]
    pub outputs: Vec<Output>,
}

impl ResourceKind for Act {
    const KIND: &'static str = "Act";
}

impl HashContent for Act {
    fn hash_content(&self, state: &mut sha2::Sha256) {
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
/// A build step that references an act and binds values for it.
pub struct Step {
    /// Step identifier within a build.
    pub id: Var,
    /// Label selector used to choose the act for this step.
    pub act: ActRef,
    /// Values passed to the selected act.
    #[serde(default)]
    pub with: Bindings,
    /// Step identifiers that must finish before this step can run.
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
/// A label selector for an act.
pub struct ActRef(pub LabelMap);

impl HashContent for ActRef {
    fn hash_content(&self, state: &mut sha2::Sha256) {
        self.0.hash_content(state);
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
/// A string expression that can expand `${name}` variables.
pub struct Expr(Box<str>);

impl Expr {
    pub fn eval(&self, vars: &BTreeMap<Var, String>) -> Result<cel::Value, ExprError> {
        // TODO: Cache compiled CEL programs for expressions that are evaluated repeatedly.
        let program = cel::Program::compile(&self.0).map_err(|err| ExprError::CelParse {
            expr: self.0.clone(),
            source: err.to_string(),
        })?;
        let mut context = cel::Context::default();
        let vars_map = vars
            .iter()
            .map(|(name, value)| (name.to_string(), value.clone()))
            .collect::<BTreeMap<_, _>>();
        context
            .add_variable("vars", vars_map)
            .map_err(|err| ExprError::CelVariable {
                name: Var::new("vars").expect("static variable name is valid"),
                source: err.to_string(),
            })?;
        for (name, value) in vars {
            context
                .add_variable(name.to_string(), value.as_str())
                .map_err(|err| ExprError::CelVariable {
                    name: name.clone(),
                    source: err.to_string(),
                })?;
        }
        program
            .execute(&context)
            .map_err(|err| ExprError::CelExecution {
                expr: self.0.clone(),
                source: err.to_string(),
            })
    }

    pub fn eval_string(&self, vars: &BTreeMap<Var, String>) -> Result<String, ExprError> {
        match self.eval(vars)? {
            cel::Value::String(value) => Ok(value.as_ref().clone()),
            value => Err(ExprError::ExpectedString {
                expr: self.0.clone(),
                typ: value.type_of().to_string(),
            }),
        }
    }

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

#[derive(Debug)]
pub enum ExprError {
    UnclosedVariable { expr: Box<str> },
    InvalidVariable { name: Box<str> },
    UnknownVariable { name: Var },
    CelParse { expr: Box<str>, source: String },
    CelVariable { name: Var, source: String },
    CelExecution { expr: Box<str>, source: String },
    ExpectedString { expr: Box<str>, typ: String },
}

impl std::fmt::Display for ExprError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExprError::UnclosedVariable { expr } => write!(f, "unclosed variable in {expr}"),
            ExprError::InvalidVariable { name } => write!(f, "{name} is not a valid variable name"),
            ExprError::UnknownVariable { name } => write!(f, "{name} is not defined"),
            ExprError::CelParse { expr, source } => {
                write!(f, "failed to parse CEL expression {expr}: {source}")
            }
            ExprError::CelVariable { name, source } => {
                write!(f, "failed to add CEL variable {name}: {source}")
            }
            ExprError::CelExecution { expr, source } => {
                write!(f, "failed to execute CEL expression {expr}: {source}")
            }
            ExprError::ExpectedString { expr, typ } => {
                write!(f, "CEL expression {expr} returned {typ}, expected string")
            }
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
/// An input source consumed by an act.
pub enum Input {
    /// Fetch input from an HTTP URL expression.
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
/// The transformation performed by an act.
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
/// An output declared by an act.
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
