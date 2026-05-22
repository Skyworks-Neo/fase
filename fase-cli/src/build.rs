use super::*;

use std::{collections::BTreeMap, env, error::Error as StdError, fs, io};

use cyper::Client;
use fase_api::{
    Act, ActRef, Build as ApiBuild, Input as ApiInput, Label, LabelMap, Resource, Sha, Step,
};
use fase_runtime::{Artifact, Context, Input as RuntimeInput, apply};
use url::Url;

type Error = Box<dyn StdError + Send + Sync>;
type Result<T> = std::result::Result<T, Error>;
type CliAct = Act<Label, String>;
type CliBuild = ApiBuild<Label, String>;
type CliStep = Step<Label, String>;

pub async fn run(path: PathBuf) -> Result<()> {
    let resources = kustomize::collect(path).await?;
    let acts = acts(&resources);
    let builds = builds(&resources);

    if builds.is_empty() {
        return Err(error("no Build resource found"));
    }

    for build in builds {
        let root = build_root(build)?;
        let outputs = execute(build, &acts, &root).await?;
        for (step, artifacts) in outputs {
            for artifact in artifacts {
                println!("{step}\t{}", artifact.path().display());
            }
        }
    }

    Ok(())
}

async fn execute(
    build: &CliBuild,
    acts: &[&CliAct],
    root: &Path,
) -> Result<BTreeMap<String, Vec<Artifact>>> {
    let mut done = BTreeMap::<String, Vec<Artifact>>::new();
    let mut pending = build.steps.iter().collect::<Vec<_>>();

    while !pending.is_empty() {
        let Some(index) = pending.iter().position(|step| ready(step, &done)) else {
            return Err(error("build graph has missing or cyclic step dependencies"));
        };
        let step = pending.remove(index);
        let act = select_act(acts, &step.act)?;
        let outputs = execute_step(step, act, root, &done).await?;
        done.insert(step.id.as_ref().to_owned(), outputs);
    }

    Ok(done)
}

async fn execute_step(
    step: &CliStep,
    act: &CliAct,
    root: &Path,
    done: &BTreeMap<String, Vec<Artifact>>,
) -> Result<Vec<Artifact>> {
    let mut outputs = Vec::new();
    let base = root.join(segment(step.id.as_ref()));

    for (index, matrix) in matrix(act.matrix.iter()).into_iter().enumerate() {
        let vars = bindings(step.with.iter(), matrix);
        let case = base.join(index.to_string());
        let sources = acquire_inputs(act, &vars, &case.join("inputs"), done, step).await?;
        let inputs = sources
            .iter()
            .map(|artifact| RuntimeInput::path(artifact.path()))
            .collect::<Vec<_>>();
        outputs.extend(apply(act.map.clone(), Context::new(inputs, case.join("outputs"))).await?);
    }

    Ok(outputs)
}

async fn acquire_inputs(
    act: &CliAct,
    vars: &BTreeMap<String, String>,
    dir: &Path,
    done: &BTreeMap<String, Vec<Artifact>>,
    step: &CliStep,
) -> Result<Vec<Artifact>> {
    if act.inputs.is_empty() {
        return Ok(step
            .needs
            .iter()
            .filter_map(|need| done.get(need.as_ref()))
            .flat_map(|artifacts| artifacts.iter().cloned())
            .collect());
    }

    let mut artifacts = Vec::with_capacity(act.inputs.len());
    for input in &act.inputs {
        artifacts.push(acquire_input(input, vars, dir).await?);
    }
    Ok(artifacts)
}

async fn acquire_input(
    input: &ApiInput<String>,
    vars: &BTreeMap<String, String>,
    dir: &Path,
) -> Result<Artifact> {
    fs::create_dir_all(dir)?;

    match input {
        ApiInput::Env { name } => {
            let name = expand(name, vars)?;
            let value = env::var(&name)?;
            let output = dir.join(segment(&name));
            fs::write(&output, value)?;
            Ok(Artifact::from(output))
        }
        ApiInput::File { path } => {
            let path = expand(path, vars)?;
            let input = Path::new(&path);
            let output = dir.join(file_name(input)?);
            fs::copy(input, &output)?;
            Ok(Artifact::from(output))
        }
        ApiInput::Http { url } => {
            let url = expand(url, vars)?;
            let client = Client::new()?;
            let response = client.get(&url)?.send().await?;
            let status = response.status();
            let final_url = response.url().clone();
            if !status.is_success() {
                return Err(error(format!("get {final_url} failed with {status}")));
            }

            let output = dir.join(url_name(&final_url));
            fs::write(&output, response.bytes().await?)?;
            Ok(Artifact::from(output))
        }
        ApiInput::Dir { path } => {
            let path = expand(path, vars)?;
            let input = Path::new(&path);
            let output = dir.join(file_name(input)?);
            copy_dir(input, &output)?;
            Ok(Artifact::from(output))
        }
    }
}

fn acts(resources: &[kustomize::CliResource]) -> Vec<&CliAct> {
    resources
        .iter()
        .filter_map(|resource| match resource {
            Resource::Act(act) => Some(act),
            _ => None,
        })
        .collect()
}

fn builds(resources: &[kustomize::CliResource]) -> Vec<&CliBuild> {
    resources
        .iter()
        .filter_map(|resource| match resource {
            Resource::Build(build) => Some(build),
            _ => None,
        })
        .collect()
}

fn select_act<'a>(acts: &'a [&CliAct], selector: &ActRef<Label>) -> Result<&'a CliAct> {
    let selected = acts
        .iter()
        .copied()
        .filter(|act| labels_match(&act.labels, &selector.0))
        .collect::<Vec<_>>();

    match selected.as_slice() {
        [act] => Ok(act),
        [] => Err(error(format!(
            "no Act matches selector {}",
            labels(&selector.0)
        ))),
        _ => Err(error(format!(
            "multiple Acts match selector {}",
            labels(&selector.0)
        ))),
    }
}

fn labels_match(labels: &LabelMap<Label>, selector: &LabelMap<Label>) -> bool {
    selector
        .iter()
        .all(|(key, value)| labels.iter().any(|(k, v)| k == key && v == value))
}

fn ready(step: &CliStep, done: &BTreeMap<String, Vec<Artifact>>) -> bool {
    step.needs
        .iter()
        .all(|need| done.contains_key(need.as_ref()))
}

fn bindings<'a>(
    with: impl IntoIterator<Item = (&'a Label, &'a String)>,
    matrix: BTreeMap<String, String>,
) -> BTreeMap<String, String> {
    let mut vars = matrix;
    vars.extend(
        with.into_iter()
            .map(|(key, value)| (key.as_ref().to_owned(), value.clone())),
    );
    vars
}

fn matrix<'a>(
    matrix: impl IntoIterator<Item = (&'a Label, &'a Vec<Label>)>,
) -> Vec<BTreeMap<String, String>> {
    let mut cases = vec![BTreeMap::new()];
    for (key, values) in matrix {
        let mut next = Vec::new();
        for case in &cases {
            for value in values {
                let mut case = case.clone();
                case.insert(key.as_ref().to_owned(), value.as_ref().to_owned());
                next.push(case);
            }
        }
        cases = next;
    }
    cases
}

fn expand(value: &str, vars: &BTreeMap<String, String>) -> Result<String> {
    let mut output = String::new();
    let mut rest = value;

    while let Some(start) = rest.find("${") {
        output.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let Some(end) = after.find('}') else {
            return Err(error(format!("unclosed expression in {value}")));
        };
        let name = &after[..end];
        let Some(value) = vars.get(name) else {
            return Err(error(format!("missing binding for {name}")));
        };
        output.push_str(value);
        rest = &after[end + 1..];
    }

    output.push_str(rest);
    Ok(output)
}

fn build_root(build: &CliBuild) -> Result<PathBuf> {
    let sum = build.sha256().to_string();
    let (prefix, rest) = sum.split_at(2);

    Ok(cache_home()?
        .join("fase")
        .join("build")
        .join(prefix)
        .join(rest))
}

fn cache_home() -> Result<PathBuf> {
    if let Some(value) = env::var_os("XDG_CACHE_HOME") {
        let path = PathBuf::from(value);
        if path.is_absolute() {
            return Ok(path);
        }
    }

    env::var_os("HOME")
        .map(|home| PathBuf::from(home).join(".cache"))
        .ok_or_else(|| error("HOME is not set and XDG_CACHE_HOME is unavailable"))
}

fn copy_dir(input: &Path, output: &Path) -> Result<()> {
    if output.exists() {
        fs::remove_dir_all(output)?;
    }
    fs::create_dir_all(output)?;

    for entry in fs::read_dir(input)? {
        let entry = entry?;
        let path = entry.path();
        let target = output.join(entry.file_name());
        if path.is_dir() {
            copy_dir(&path, &target)?;
        } else {
            fs::copy(&path, target)?;
        }
    }

    Ok(())
}

fn file_name(path: &Path) -> Result<&std::ffi::OsStr> {
    path.file_name()
        .ok_or_else(|| error(format!("input path has no file name: {}", path.display())))
}

fn url_name(url: &Url) -> &str {
    url.path_segments()
        .and_then(|segments| segments.rev().find(|segment| !segment.is_empty()))
        .filter(|name| !name.is_empty())
        .unwrap_or("index")
}

fn labels(labels: &LabelMap<Label>) -> String {
    labels
        .iter()
        .map(|(key, value)| format!("{}={}", key.as_ref(), value.as_ref()))
        .collect::<Vec<_>>()
        .join(",")
}

fn segment(value: &str) -> String {
    let value = value
        .chars()
        .map(|ch| match ch {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '.' | '_' | '-' => ch,
            _ => '_',
        })
        .collect::<String>();

    match value.is_empty() {
        true => "_".to_owned(),
        false => value,
    }
}

fn error(message: impl Into<String>) -> Error {
    Box::new(io::Error::other(message.into()))
}
