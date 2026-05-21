use super::*;

use std::{error::Error as StdError, io};

use compio::{
    BufResult,
    fs::{self, metadata, stdout},
    io::AsyncWriteExt,
};
use serde::Deserialize;

use fase_api::{Label, LabelMap, Resource};

type CliResource = Resource<Label, String>;
type Labels = Vec<LabelMap<Label>>;
type Error = Box<dyn StdError>;
type Result<T> = std::result::Result<T, Error>;

struct Frame {
    path: PathBuf,
    labels: Labels,
}

impl Frame {
    fn new(path: PathBuf) -> Self {
        Self {
            path,
            labels: Vec::new(),
        }
    }

    fn scoped(&self, labels: Labels) -> Labels {
        let mut scope = self.labels.clone();
        scope.extend(labels);
        scope
    }
}

enum Work {
    Path(Frame),
    Resource {
        resource: CliResource,
        labels: Labels,
    },
}

struct Renderer {
    work: Vec<Work>,
}

impl Renderer {
    fn new(path: PathBuf) -> Self {
        Self {
            work: vec![Work::Path(Frame::new(path))],
        }
    }

    async fn collect(mut self) -> Result<Vec<CliResource>> {
        let mut rendered = Vec::new();

        while let Some(item) = self.work.pop() {
            match item {
                Work::Path(frame) => {
                    let path = manifest(&frame.path).await?;
                    let base = path.parent().unwrap_or_else(|| Path::new("."));
                    let contents = read_utf8(&path).await?;
                    let resources = parse(&path, &contents)?;
                    for resource in resources.into_iter().rev() {
                        match resource {
                            Resource::Kustomize(kustomize) => {
                                let labels = frame.scoped(kustomize.labels);
                                for path in kustomize.resources.into_iter().rev() {
                                    self.work.push(Work::Path(Frame {
                                        path: base.join(path),
                                        labels: labels.clone(),
                                    }));
                                }
                            }
                            resource => self.work.push(Work::Resource {
                                resource,
                                labels: frame.labels.clone(),
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

pub async fn run(path: PathBuf) -> Result<()> {
    let resources = Renderer::new(path).collect().await?;
    let output = render(&resources)?;

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

fn parse(path: &Path, contents: &str) -> Result<Vec<CliResource>> {
    let mut resources = Vec::new();
    for document in serde_yml::Deserializer::from_str(contents) {
        let resource = Option::<CliResource>::deserialize(document)
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
