use super::*;

use std::{collections::BTreeMap, error::Error as StdError, io};

use compio::{
    BufResult,
    fs::{self, metadata, stdout},
    io::AsyncWriteExt,
};
use serde::Deserialize;

use fase_api::{ApiResource, Label, LabelMap, Resource};
use fase_runtime::Runtime;

pub type CliResource = Resource<Label, String>;
type RawResource = Resource<String, String>;
type Labels = Vec<LabelMap<Label>>;
pub type Error = Box<dyn StdError + Send + Sync>;
pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
pub enum Output {
    Count,
    Yaml,
}

struct Frame {
    path: PathBuf,
    labels: Labels,
    ancestors: Vec<PathBuf>,
}

enum Work {
    Path(Frame),
    Resource {
        resource: CliResource,
        labels: Labels,
    },
}

struct Renderer {
    runtime: Runtime,
    work: Vec<Work>,
}

impl Renderer {
    fn new(runtime: &Runtime, path: PathBuf) -> Self {
        Self {
            runtime: runtime.clone(),
            work: vec![Work::Path(Frame {
                path,
                labels: Vec::new(),
                ancestors: Vec::new(),
            })],
        }
    }

    async fn collect(mut self) -> Result<Vec<CliResource>> {
        let mut rendered = Vec::new();

        while let Some(item) = self.work.pop() {
            match item {
                Work::Path(frame) => {
                    let Frame {
                        path,
                        labels,
                        ancestors,
                    } = frame;
                    let path = manifest(&path).await?;
                    let key = std::fs::canonicalize(&path)?;
                    if let Some(start) = ancestors.iter().position(|parent| parent == &key) {
                        let cycle = ancestors[start..]
                            .iter()
                            .chain(std::iter::once(&key))
                            .map(|path| path.display().to_string())
                            .collect::<Vec<_>>()
                            .join(" -> ");
                        return Err(Box::new(io::Error::new(
                            io::ErrorKind::InvalidData,
                            format!("recursive kustomize resources: {cycle}"),
                        )));
                    }

                    let mut ancestors = ancestors;
                    ancestors.push(key);
                    let base = path.parent().unwrap_or_else(|| Path::new("."));
                    let contents = read_utf8(&path).await?;
                    let resources = parse(&path, &contents)?;
                    for resource in resources.into_iter().rev() {
                        let resource = self.runtime.intern(resource);
                        match resource {
                            Resource::Kustomize(kustomize) => {
                                let mut labels = labels.clone();
                                labels.extend(kustomize.labels);
                                for path in kustomize.resources.into_iter().rev() {
                                    self.work.push(Work::Path(Frame {
                                        path: base.join(path),
                                        labels: labels.clone(),
                                        ancestors: ancestors.clone(),
                                    }));
                                }
                            }
                            resource => self.work.push(Work::Resource {
                                resource,
                                labels: labels.clone(),
                            }),
                        }
                    }
                }
                Work::Resource {
                    mut resource,
                    labels,
                } => {
                    for labels in labels {
                        resource.merge_labels(&labels);
                    }
                    rendered.push(resource);
                }
            }
        }
        Ok(rendered)
    }
}

pub async fn collect(runtime: &Runtime, path: PathBuf) -> Result<Vec<CliResource>> {
    Renderer::new(runtime, path).collect().await
}

pub async fn run(runtime: &Runtime, path: PathBuf, output: Output) -> Result<()> {
    let resources = collect(runtime, path).await?;
    let output = match output {
        Output::Count => count(&resources),
        Output::Yaml => render(&resources)?,
    };

    let mut stdout = stdout();
    let BufResult(result, _) = stdout.write_all(output.into_bytes()).await;
    result?;

    Ok(())
}

async fn manifest(path: &Path) -> Result<PathBuf> {
    let meta = metadata(path).await?;
    if !meta.is_dir() {
        return Ok(path.to_owned());
    }

    let candidate = path.join("kustomize.yaml");
    match metadata(&candidate).await {
        Ok(meta) if meta.is_file() => Ok(candidate),
        Ok(_) => Err(Box::new(io::Error::new(
            io::ErrorKind::NotFound,
            format!("expect file at {}", path.display()),
        ))),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Err(Box::new(io::Error::new(
            io::ErrorKind::NotFound,
            format!("no kustomize file found in {}", path.display()),
        ))),
        Err(error) => Err(Box::new(error)),
    }
}

async fn read_utf8(path: &Path) -> Result<String> {
    Ok(String::from_utf8(fs::read(path).await?)?)
}

fn parse(path: &Path, contents: &str) -> Result<Vec<RawResource>> {
    let mut resources = Vec::new();
    for document in serde_yml::Deserializer::from_str(contents) {
        let resource = Option::<RawResource>::deserialize(document)
            .map_err(|error| parse_error(path, error))?;
        if let Some(resource) = resource {
            resources.push(resource);
        }
    }
    Ok(resources)
}

fn parse_error(path: &Path, error: serde_yml::Error) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!("failed to parse {}: {error}", path.display()),
    )
}

fn count(resources: &[CliResource]) -> String {
    let mut counts = BTreeMap::<ApiResource, usize>::new();

    for resource in resources {
        *counts.entry(resource.into()).or_default() += 1;
    }

    let mut output = "apiVersion\tkind\tcount\n".to_owned();
    for (resource, count) in counts {
        output.push_str(&format!(
            "{}\t{}\t{}\n",
            resource.api_version, resource.kind, count
        ));
    }
    output
}

fn render(resources: &[CliResource]) -> Result<String> {
    let mut output = String::new();
    for (index, resource) in resources.iter().enumerate() {
        if index > 0 {
            output.push_str("---\n");
        }
        output.push_str(&serde_yml::to_string(resource)?);
    }
    Ok(output)
}

#[cfg(test)]
mod test {
    use super::*;

    #[compio::test]
    async fn recursive_kustomize() {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("fase-cli-kustomize-cycle-{nonce}"));
        let a = root.join("a");
        let b = root.join("b");
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        std::fs::write(a.join("kustomize.yaml"), "resources:\n  - ../b\n").unwrap();
        std::fs::write(b.join("kustomize.yaml"), "resources:\n  - ../a\n").unwrap();

        let error = collect(&Runtime::new(), a).await.unwrap_err();
        assert!(
            error.to_string().contains("recursive kustomize resources"),
            "{error}"
        );

        std::fs::remove_dir_all(root).unwrap();
    }
}
