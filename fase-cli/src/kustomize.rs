use super::*;

use std::{error::Error, io};

use compio::{
    BufResult,
    fs::{self, metadata, stdout},
    io::AsyncWriteExt,
};
use serde::Deserialize;

use fase_api::{Label, LabelMap, Resource};

type CliResource = Resource<Label, String>;

struct Frame {
    path: PathBuf,
    labels: Vec<LabelMap<Label>>,
}

enum Work {
    Path(Frame),
    Resource {
        resource: CliResource,
        labels: Vec<LabelMap<Label>>,
    },
}

pub async fn run(path: PathBuf) -> Result<(), Box<dyn Error>> {
    let resources = render(path).await?;
    let output = render_yaml(&resources)?;

    let mut stdout = stdout();
    let BufResult(result, _) = stdout.write_all(output.into_bytes()).await;
    result?;

    Ok(())
}

async fn render(path: PathBuf) -> Result<Vec<CliResource>, Box<dyn Error>> {
    let mut rendered = Vec::new();
    let mut work = vec![Work::Path(Frame {
        path,
        labels: Vec::new(),
    })];

    while let Some(item) = work.pop() {
        match item {
            Work::Path(frame) => {
                let path = resolve_resource_path(frame.path).await?;
                let base = path.parent().unwrap_or_else(|| Path::new("."));
                let contents = read_to_string(&path).await?;
                let resources = parse_resources(&path, &contents)?;
                for resource in resources.into_iter().rev() {
                    match resource {
                        Resource::Kustomize(kustomize) => {
                            let mut labels = frame.labels.clone();
                            labels.extend(kustomize.labels);
                            for path in kustomize.resources.into_iter().rev() {
                                work.push(Work::Path(Frame {
                                    path: base.join(path),
                                    labels: labels.clone(),
                                }));
                            }
                        }
                        resource => work.push(Work::Resource {
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
                for labels in &labels {
                    resource.merge_labels(labels);
                }
                rendered.push(resource);
            }
        }
    }

    Ok(rendered)
}

async fn resolve_resource_path(path: PathBuf) -> Result<PathBuf, Box<dyn Error>> {
    let meta = metadata(&path).await?;
    if !meta.is_dir() {
        return Ok(path);
    }

    for file_name in ["kustomization.yml", "kustomization.yaml", "Kustomization"] {
        let candidate = path.join(file_name);
        match metadata(&candidate).await {
            Ok(meta) if meta.is_file() => return Ok(candidate),
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(Box::new(error)),
        }
    }

    Err(format!("no kustomization file found in {}", path.display()).into())
}

async fn read_to_string(path: &Path) -> Result<String, Box<dyn Error>> {
    let bytes = fs::read(path).await?;
    String::from_utf8(bytes).map_err(Into::into)
}

fn parse_resources(path: &Path, contents: &str) -> Result<Vec<CliResource>, Box<dyn Error>> {
    let mut resources = Vec::new();
    for document in serde_yml::Deserializer::from_str(contents) {
        let resource = Option::<CliResource>::deserialize(document)
            .map_err(|error| format!("failed to parse {}: {error}", path.display()))?;
        if let Some(resource) = resource {
            resources.push(resource);
        }
    }
    Ok(resources)
}

fn render_yaml(resources: &[CliResource]) -> Result<String, serde_yml::Error> {
    let mut output = String::new();
    for (index, resource) in resources.iter().enumerate() {
        if index > 0 {
            output.push_str("---\n");
        }
        output.push_str(&serde_yml::to_string(resource)?);
    }
    Ok(output)
}
