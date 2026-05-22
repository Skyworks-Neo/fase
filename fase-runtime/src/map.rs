mod zstd;

use std::path::{Path, PathBuf};

use fase_api::Map;

pub type Error = Box<dyn std::error::Error + Send + Sync + 'static>;
pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Artifact(PathBuf);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Input(String);

pub(crate) struct Identity;

impl Transform for Identity {
    async fn run(self, context: Context) -> Result<Vec<Artifact>> {
        Ok(context.inputs.into_iter().map(Artifact::from).collect())
    }
}

impl Artifact {
    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl From<PathBuf> for Artifact {
    fn from(path: PathBuf) -> Self {
        Self(path)
    }
}

impl From<&Path> for Artifact {
    fn from(path: &Path) -> Self {
        Self(path.to_owned())
    }
}

impl AsRef<Path> for Artifact {
    fn as_ref(&self) -> &Path {
        self.path()
    }
}

impl From<Input> for Artifact {
    fn from(input: Input) -> Self {
        Self(input.0.into())
    }
}

impl Input {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn path(path: impl AsRef<Path>) -> Self {
        Self(path.as_ref().to_string_lossy().into_owned())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn as_path(&self) -> &Path {
        Path::new(&self.0)
    }
}

impl From<String> for Input {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl From<&str> for Input {
    fn from(value: &str) -> Self {
        Self(value.to_owned())
    }
}

impl AsRef<str> for Input {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

#[derive(Debug, Clone)]
pub struct Context {
    inputs: Vec<Input>,
    output_dir: PathBuf,
    zstd_level: i32,
}

impl Context {
    const DEFAULT_ZSTD_LEVEL: i32 = 3;

    pub fn new<I, T>(inputs: I, output_dir: impl Into<PathBuf>) -> Self
    where
        I: IntoIterator<Item = T>,
        T: Into<Input>,
    {
        Self {
            inputs: inputs.into_iter().map(Into::into).collect(),
            output_dir: output_dir.into(),
            zstd_level: Self::DEFAULT_ZSTD_LEVEL,
        }
    }

    pub fn level(mut self, level: i32) -> Self {
        self.zstd_level = level;
        self
    }

    pub(crate) fn inputs(&self) -> &[Input] {
        &self.inputs
    }

    pub(crate) fn output_dir(&self) -> &Path {
        &self.output_dir
    }

    pub(crate) fn zstd_level(&self) -> i32 {
        self.zstd_level
    }
}

trait Transform {
    async fn run(self, context: Context) -> Result<Vec<Artifact>>;
}

trait Execute {
    async fn execute(self, context: Context) -> Result<Vec<Artifact>>;
}

impl Execute for Map {
    async fn execute(self, context: Context) -> Result<Vec<Artifact>> {
        match self {
            Map::Identity => Identity.run(context).await,
            Map::Zstd => zstd::Zstd.run(context).await,
            Map::Run => Err(Box::new(std::io::Error::other(
                "run map is not implemented",
            ))),
        }
    }
}

pub async fn apply(map: Map, context: Context) -> Result<Vec<Artifact>> {
    map.execute(context).await
}

#[cfg(test)]
mod test {
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    #[compio::test]
    async fn zstd() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("fase-runtime-zstd-{nonce}"));
        let input_dir = root.join("input");
        let output_dir = root.join("output");
        std::fs::create_dir_all(&input_dir).unwrap();

        let input = input_dir.join("hello.txt");
        let content = b"hello from fase runtime";
        std::fs::write(&input, content).unwrap();

        let outputs = apply(Map::Zstd, Context::new([Input::path(&input)], &output_dir))
            .await
            .unwrap();

        assert_eq!(outputs.len(), 1);
        assert_eq!(outputs[0].path(), output_dir.join("hello.txt.zst"));

        let compressed = std::fs::read(outputs[0].path()).unwrap();
        let decompressed = ::zstd::bulk::decompress(&compressed, 1024).unwrap();
        assert_eq!(decompressed, content);

        std::fs::remove_dir_all(root).unwrap();
    }
}
