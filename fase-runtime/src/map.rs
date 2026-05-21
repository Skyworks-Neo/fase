mod zstd;

use std::path::{Path, PathBuf};

use fase_api::Map;

pub type Error = Box<dyn std::error::Error + Send + Sync + 'static>;
pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Artifact(PathBuf);

pub(crate) struct Identity;

impl Transform for Identity {
    async fn run(self, context: Context) -> Result<Vec<Artifact>> {
        Ok(context.inputs)
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

#[derive(Debug, Clone)]
pub struct Context {
    inputs: Vec<Artifact>,
    output_dir: PathBuf,
    zstd_level: i32,
}

impl Context {
    const DEFAULT_ZSTD_LEVEL: i32 = 3;

    pub fn new(inputs: impl IntoIterator<Item = Artifact>, output_dir: impl Into<PathBuf>) -> Self {
        Self {
            inputs: inputs.into_iter().collect(),
            output_dir: output_dir.into(),
            zstd_level: Self::DEFAULT_ZSTD_LEVEL,
        }
    }

    pub fn level(mut self, level: i32) -> Self {
        self.zstd_level = level;
        self
    }

    pub(crate) fn inputs(&self) -> &[Artifact] {
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

        let outputs = apply(
            Map::Zstd,
            Context::new([Artifact::from(input.as_path())], &output_dir),
        )
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
