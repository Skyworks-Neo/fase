use fase_api::{
    Artifact, ArtifactClaim, ArtifactReference, LabelBinding, LabelOperator, LabelSelector, Labels,
    ObjectReference, Phase, Recipe, Request, ResolutionDiagnostic, ResolvedInput,
    ResolvedInputOrigin, ResolvedInputs, ResolvedOutput, ResolvedOutputs, ResolvedRecipeStatus,
    Task, TaskStatus, Variables, valid_label_value, valid_name,
};
use kube::ResourceExt;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone)]
struct Choice {
    labels: Labels,
    kind: fase_api::ArtifactKind,
    content_digest: Option<String>,
    reference: Option<ArtifactReference>,
    claim_ref: Option<ArtifactReference>,
    producer: Option<(String, String)>,
}

#[derive(Debug)]
pub struct Resolution {
    pub recipes: Vec<ResolvedRecipeStatus>,
    pub artifact_ref: Option<ArtifactReference>,
    pub content_digest: Option<String>,
    pub claim_ref: Option<ArtifactReference>,
    pub target_output: Option<String>,
    pub warnings: Vec<String>,
}

#[derive(Debug)]
pub struct ResolutionFailure {
    pub reason: String,
    pub message: String,
    pub diagnostics: Vec<ResolutionDiagnostic>,
}

impl From<String> for ResolutionFailure {
    fn from(message: String) -> Self {
        Self {
            reason: "InvalidSelector".into(),
            message,
            diagnostics: Vec::new(),
        }
    }
}

impl From<&str> for ResolutionFailure {
    fn from(message: &str) -> Self {
        message.to_owned().into()
    }
}

struct Solver<'a> {
    request: &'a Request,
    recipes: &'a [Recipe],
    tasks: &'a [Task],
    artifacts: &'a [Artifact],
    claims: &'a [ArtifactClaim],
    graph: Vec<ResolvedRecipeStatus>,
    stack: Vec<String>,
    warnings: Vec<String>,
}

pub fn resolve(
    request: &Request,
    recipes: &[Recipe],
    tasks: &[Task],
    artifacts: &[Artifact],
    claims: &[ArtifactClaim],
) -> Result<Resolution, ResolutionFailure> {
    request.spec.artifact_selector.validate()?;
    if let Some(selector) = &request.spec.recipe_selector {
        selector.validate()?;
    }
    if request.spec.artifact_selector.is_empty() {
        return Err("artifactSelector is empty".into());
    }
    let mut solver = Solver {
        request,
        recipes,
        tasks,
        artifacts,
        claims,
        graph: Vec::new(),
        stack: Vec::new(),
        warnings: Vec::new(),
    };
    let choice = solver.choose(
        &request.spec.artifact_selector,
        true,
        &request.spec.variables,
    )?;
    Ok(Resolution {
        recipes: solver.graph,
        artifact_ref: choice.reference,
        content_digest: choice.content_digest,
        claim_ref: choice.claim_ref,
        target_output: choice.producer.map(|p| p.1),
        warnings: solver.warnings,
    })
}

impl Solver<'_> {
    fn choose(
        &mut self,
        selector: &LabelSelector,
        root: bool,
        scope: &Variables,
    ) -> Result<Choice, ResolutionFailure> {
        selector.validate()?;
        let mut existing: Vec<_> = self
            .claims
            .iter()
            .filter(|a| a.metadata.deletion_timestamp.is_none())
            .filter(|a| {
                self.artifacts.iter().any(|artifact| {
                    artifact.metadata.deletion_timestamp.is_none()
                        && artifact.name_any() == a.spec.artifact_ref.name
                })
            })
            .filter(|a| {
                a.metadata
                    .labels
                    .as_ref()
                    .is_some_and(|l| fase_api::valid_labels(l) && selector.matches(l))
            })
            .collect();
        existing.sort_by_key(|a| a.name_any());
        if !root || (self.request.spec.recipe_selector.is_none() && self.request.spec.rerun == 0) {
            if existing.len() > 1 {
                self.warnings.push(format!(
                    "multiple ArtifactClaims match: {}",
                    existing
                        .iter()
                        .map(|a| a.name_any())
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }
            if let Some(a) = existing.first() {
                let artifact = self
                    .artifacts
                    .iter()
                    .find(|item| item.name_any() == a.spec.artifact_ref.name)
                    .ok_or("ArtifactClaim references a missing Artifact")?;
                return Ok(Choice {
                    labels: a.metadata.labels.clone().unwrap_or_default(),
                    kind: artifact.spec.kind,
                    content_digest: Some(artifact.spec.content_digest.clone()),
                    reference: Some(a.spec.artifact_ref.clone()),
                    claim_ref: Some(ArtifactReference { name: a.name_any() }),
                    producer: None,
                });
            }
        }
        let recipened: Vec<_> = self
            .graph
            .iter()
            .flat_map(|recipe| {
                recipe
                    .definition
                    .outputs
                    .artifacts
                    .iter()
                    .filter_map(move |output| {
                        let labels = frozen_labels(recipe, output).ok()?;
                        if !selector.matches(&labels) {
                            return None;
                        }
                        let task = recipe
                            .tasks
                            .iter()
                            .find(|s| Some(&s.name) == output.from.task.as_ref())?;
                        let port = task
                            .definition
                            .outputs
                            .artifacts
                            .iter()
                            .find(|p| Some(&p.name) == output.from.artifact.as_ref())?;
                        Some(Choice {
                            labels,
                            kind: port.kind,
                            content_digest: None,
                            reference: None,
                            claim_ref: None,
                            producer: Some((recipe.name.clone(), output.name.clone())),
                        })
                    })
            })
            .collect();
        if recipened.len() > 1 {
            self.warnings.push(format!(
                "multiple resolved outputs match: {}",
                recipened
                    .iter()
                    .filter_map(|choice| choice.producer.as_ref())
                    .map(|(recipe, output)| format!("{recipe}/{output}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        if let Some(choice) = recipened.into_iter().next() {
            return Ok(choice);
        }
        let mut viable = Vec::new();
        let mut errors = Vec::new();
        for recipe in self
            .recipes
            .iter()
            .filter(|p| p.metadata.deletion_timestamp.is_none())
            .filter(|p| {
                !root
                    || self
                        .request
                        .spec
                        .recipe_selector
                        .as_ref()
                        .is_none_or(|selector| {
                            selector.matches(p.metadata.labels.as_ref().unwrap_or(&Labels::new()))
                        })
            })
        {
            for output in &recipe.spec.outputs.artifacts {
                if !may_match(output, selector, scope) {
                    continue;
                }
                let candidate = format!("{}/{}", recipe.name_any(), output.name);
                let mut branch = Solver {
                    request: self.request,
                    recipes: self.recipes,
                    tasks: self.tasks,
                    artifacts: self.artifacts,
                    claims: self.claims,
                    graph: self.graph.clone(),
                    stack: self.stack.clone(),
                    warnings: Vec::new(),
                };
                match branch.expand(recipe, output, selector, scope) {
                    Ok(choice) => viable.push((candidate, choice, branch.graph, branch.warnings)),
                    Err(message) => errors.push(ResolutionDiagnostic {
                        candidate,
                        reason: diagnostic_reason(&message).into(),
                        message,
                    }),
                }
            }
        }
        match viable.len() {
            n if n > 0 => {
                if n > 1 {
                    self.warnings.push(format!(
                        "multiple Recipe outputs match: {}",
                        viable
                            .iter()
                            .map(|item| item.0.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    ));
                }
                let (_, choice, graph, warnings) = viable.remove(0);
                self.graph = graph;
                self.warnings.extend(warnings);
                Ok(choice)
            }
            _ => {
                let reason = if !errors.is_empty() {
                    if errors.iter().all(|item| item.reason == "MissingParameter") {
                        "MissingParameter"
                    } else if errors.iter().all(|item| item.reason == "WaitingForTask") {
                        "WaitingForTask"
                    } else if errors
                        .iter()
                        .all(|item| item.reason == "WaitingForArtifact")
                    {
                        "WaitingForArtifact"
                    } else {
                        "NoViableRecipe"
                    }
                } else if root {
                    "WaitingForRecipe"
                } else {
                    "WaitingForArtifact"
                };
                let message = if errors.is_empty() {
                    format!("no ArtifactClaim or Recipe output satisfies selector ({reason})")
                } else {
                    format!("{} Recipe output candidates were rejected", errors.len())
                };
                Err(ResolutionFailure {
                    reason: reason.into(),
                    message,
                    diagnostics: errors,
                })
            }
        }
    }

    fn expand(
        &mut self,
        recipe: &Recipe,
        output: &fase_api::RecipeOutput,
        demand: &LabelSelector,
        scope: &Variables,
    ) -> Result<Choice, String> {
        let output_names: BTreeSet<_> = recipe
            .spec
            .outputs
            .artifacts
            .iter()
            .map(|o| &o.name)
            .collect();
        if output_names.len() != recipe.spec.outputs.artifacts.len()
            || recipe.spec.outputs.artifacts.is_empty()
            || output_names.iter().any(|name| !valid_name(name))
        {
            return Err("invalid or duplicate Recipe output".into());
        }
        let identity = format!("{}/{}", recipe.name_any(), output.name);
        if self.stack.contains(&identity) {
            return Err(format!("DependencyCycle: {identity}"));
        }
        if self.stack.len() >= 32 {
            return Err("DependencyCycle: depth limit".into());
        }
        self.stack.push(identity);
        let mut variables = Variables::new();
        let mut declared = BTreeSet::new();
        for var in &recipe.spec.inputs.variables {
            if !valid_name(&var.name) || !declared.insert(var.name.clone()) {
                return Err(format!("invalid Recipe variable {}", var.name));
            }
            if let Some(v) = scope
                .get(&var.name)
                .or_else(|| demand.match_labels.get(&var.name))
            {
                variables.insert(var.name.clone(), v.clone());
            }
        }
        let mut inputs = Vec::new();
        let mut input_choices = BTreeMap::new();
        for input in &recipe.spec.inputs.artifacts {
            if !valid_name(&input.name) || input_choices.contains_key(&input.name) {
                return Err(format!("invalid Recipe input {}", input.name));
            }
            let mut selector = input.artifact_selector.clone();
            for (key, binding) in &output.labels {
                if let LabelBinding::Bound(source) = binding
                    && let Some(from) = &source.from_input
                    && from.input == input.name
                    && let Some(expected) = forced_value(demand, key)
                {
                    match selector
                        .match_labels
                        .insert(from.label.clone(), expected.clone())
                    {
                        Some(previous) if previous != expected => {
                            return Err(format!(
                                "UnsatisfiedDependency: conflicting {} label",
                                from.label
                            ));
                        }
                        _ => {}
                    }
                }
            }
            let chosen = self
                .choose(&selector, false, &variables)
                .map_err(|error| format!("{}: {}", error.reason, error.message))?;
            inputs.push(ResolvedInput {
                name: input.name.clone(),
                from: ResolvedInputOrigin {
                    resolved_recipe: chosen.producer.as_ref().map(|p| p.0.clone()),
                    artifact: chosen.producer.as_ref().map(|p| p.1.clone()),
                },
                labels: chosen.labels.clone(),
                kind: chosen.kind.clone(),
                content_digest: chosen.content_digest.clone(),
                artifact_ref: chosen.reference.clone(),
                claim_ref: chosen.claim_ref.clone(),
            });
            input_choices.insert(input.name.clone(), chosen);
        }
        for var in &recipe.spec.inputs.variables {
            if variables.contains_key(&var.name) {
                continue;
            }
            let mut values = input_choices
                .values()
                .filter_map(|input| input.labels.get(&var.name));
            if let Some(value) = values.next() {
                if values.any(|other| other != value) {
                    return Err(format!(
                        "conflicting input labels for Recipe variable {}",
                        var.name
                    ));
                }
                variables.insert(var.name.clone(), value.clone());
            } else if var.required {
                return Err(format!("missing Recipe variable {}", var.name));
            }
        }
        let mut labels = Labels::new();
        for (key, binding) in &output.labels {
            let value = match binding {
                LabelBinding::Literal(v) => v.value.clone(),
                LabelBinding::Bound(source) => match (&source.from_variable, &source.from_input) {
                    (Some(v), None) => variables
                        .get(v)
                        .ok_or_else(|| format!("unknown Recipe variable {v}"))?
                        .clone(),
                    (None, Some(input)) => input_choices
                        .get(&input.input)
                        .and_then(|c: &Choice| c.labels.get(&input.label))
                        .ok_or_else(|| {
                            format!("missing input label {}.{}", input.input, input.label)
                        })?
                        .clone(),
                    _ => return Err(format!("invalid label binding {key}")),
                },
            };
            if !valid_label_value(&value) {
                return Err(format!("invalid output label {key}"));
            }
            labels.insert(key.clone(), value);
        }
        if !fase_api::valid_labels(&labels) {
            return Err("invalid Recipe output labels".into());
        }
        if !demand.matches(&labels) {
            return Err("UnsatisfiedDependency: output labels do not match selector".into());
        }
        let mut pending: BTreeMap<String, &fase_api::RecipeTask> = recipe
            .spec
            .tasks
            .iter()
            .map(|s| (s.name.clone(), s))
            .collect();
        if pending.len() != recipe.spec.tasks.len() || pending.is_empty() {
            return Err("duplicate or empty Recipe tasks".into());
        }
        let mut statuses = Vec::new();
        while !pending.is_empty() {
            let before = pending.len();
            let names: Vec<_> = pending.keys().cloned().collect();
            for name in names {
                let call = pending[&name];
                let inputs: BTreeSet<_> = call.inputs.artifacts.iter().map(|i| &i.name).collect();
                let outputs: BTreeSet<_> = call.outputs.iter().map(|o| &o.name).collect();
                if inputs.len() != call.inputs.artifacts.len()
                    || outputs.len() != call.outputs.len()
                {
                    return Err(format!("duplicate Task binding in {name}"));
                }
                let deps: Vec<_> = call
                    .inputs
                    .artifacts
                    .iter()
                    .filter_map(|b| b.from.task.as_ref())
                    .collect();
                if deps
                    .iter()
                    .any(|d| !statuses.iter().any(|s: &TaskStatus| &s.name == *d))
                {
                    continue;
                }
                call.task_selector.validate()?;
                if call.task_selector.is_empty() {
                    return Err(format!("task {name} has empty selector"));
                }
                let mut matches: Vec<_> = self
                    .tasks
                    .iter()
                    .filter(|s| s.metadata.deletion_timestamp.is_none())
                    .filter(|s| {
                        call.task_selector
                            .matches(s.metadata.labels.as_ref().unwrap_or(&Labels::new()))
                    })
                    .collect();
                matches.sort_by_key(|task| task.name_any());
                if matches.is_empty() {
                    return Err(format!(
                        "UnsatisfiedDependency: task {name} selector matched no Tasks"
                    ));
                }
                if matches.len() > 1 {
                    self.warnings.push(format!(
                        "multiple Tasks match {name}: {}",
                        matches
                            .iter()
                            .map(|task| task.name_any())
                            .collect::<Vec<_>>()
                            .join(", ")
                    ));
                }
                let task = matches[0];
                task.spec.validate()?;
                let mut env = Variables::new();
                for v in &task.spec.inputs.variables {
                    let b = call.variables.get(&v.name);
                    let value = match b {
                        Some(b) => match (&b.value, &b.from_variable, &b.from_input) {
                            (Some(v), None, None) => Some(v.clone()),
                            (None, Some(from), None) => Some(
                                variables
                                    .get(from)
                                    .ok_or_else(|| format!("missing Recipe variable {from}"))?
                                    .clone(),
                            ),
                            (None, None, Some(input)) => Some(
                                input_choices
                                    .get(&input.input)
                                    .and_then(|choice| choice.labels.get(&input.label))
                                    .ok_or_else(|| {
                                        format!(
                                            "missing input label {}.{}",
                                            input.input, input.label
                                        )
                                    })?
                                    .clone(),
                            ),
                            _ => return Err(format!("invalid binding for {}", v.name)),
                        },
                        None if v.required => return Err(format!("missing variable {}", v.name)),
                        None => None,
                    };
                    if let Some(value) = value
                        && env.insert(v.env.clone(), value).is_some()
                    {
                        return Err("duplicate variable env".into());
                    }
                }
                if call.variables.len()
                    != task
                        .spec
                        .inputs
                        .variables
                        .iter()
                        .filter(|v| call.variables.contains_key(&v.name))
                        .count()
                {
                    return Err(format!("undeclared variable binding in {name}"));
                }
                if call.inputs.artifacts.len() != task.spec.inputs.artifacts.len() {
                    return Err(format!("input count mismatch in {name}"));
                }
                for port in &task.spec.inputs.artifacts {
                    let b = call
                        .inputs
                        .artifacts
                        .iter()
                        .find(|b| b.name == port.name)
                        .ok_or_else(|| format!("missing input {}", port.name))?;
                    let origin = &b.from;
                    let kind = match (&origin.recipe_input, &origin.task, &origin.artifact) {
                        (Some(i), None, None) => input_choices
                            .get(i)
                            .ok_or_else(|| format!("unknown Recipe input {i}"))?
                            .kind
                            .clone(),
                        (None, Some(s), Some(a)) => {
                            let producer = statuses
                                .iter()
                                .find(|item: &&TaskStatus| &item.name == s)
                                .ok_or_else(|| format!("unknown previous Task {s}"))?;
                            if !recipe
                                .spec
                                .tasks
                                .iter()
                                .find(|call| &call.name == s)
                                .is_some_and(|call| call.outputs.iter().any(|out| &out.name == a))
                            {
                                return Err(format!("Task output {s}.{a} is not exposed"));
                            }
                            producer
                                .definition
                                .outputs
                                .artifacts
                                .iter()
                                .find(|p| &p.name == a)
                                .map(|p| p.kind)
                                .ok_or_else(|| format!("unknown Task output {s}.{a}"))?
                        }
                        _ => return Err("invalid Task artifact origin".into()),
                    };
                    if kind != port.kind {
                        return Err(format!("type mismatch in {name}.{}", port.name));
                    }
                }
                for out in &call.outputs {
                    if !task
                        .spec
                        .outputs
                        .artifacts
                        .iter()
                        .any(|p| p.name == out.name)
                    {
                        return Err(format!("unknown output {}", out.name));
                    }
                }
                statuses.push(TaskStatus {
                    name: name.clone(),
                    phase: Phase::Pending,
                    attempt: 0,
                    task_ref: ObjectReference {
                        api_version: "skyw.top/v1beta1".into(),
                        kind: "Task".into(),
                        name: task.name_any(),
                        uid: task.metadata.uid.clone().ok_or("Task UID missing")?,
                        generation: task.metadata.generation.unwrap_or(1),
                    },
                    definition: task.spec.clone(),
                    variables: env,
                    build_key: None,
                    job_ref: None,
                    outputs: BTreeMap::new(),
                    output_digests: BTreeMap::new(),
                    claims: BTreeMap::new(),
                    message: None,
                });
                pending.remove(&name);
            }
            if before == pending.len() {
                return Err("DependencyCycle: Task inputs form a cycle".into());
            }
        }
        for out in &recipe.spec.outputs.artifacts {
            let from = &out.from;
            let task = from
                .task
                .as_ref()
                .ok_or("Recipe output must reference Task")?;
            let artifact = from
                .artifact
                .as_ref()
                .ok_or("Recipe output missing artifact")?;
            let status = statuses
                .iter()
                .find(|s| &s.name == task)
                .ok_or("unknown output Task")?;
            if !recipe
                .spec
                .tasks
                .iter()
                .find(|s| &s.name == task)
                .is_some_and(|s| s.outputs.iter().any(|o| &o.name == artifact))
            {
                return Err("Recipe output is not exposed by Task call".into());
            }
            if !status
                .definition
                .outputs
                .artifacts
                .iter()
                .any(|p| &p.name == artifact)
            {
                return Err("unknown Recipe output artifact".into());
            }
        }
        let source_task = output.from.task.as_ref().ok_or("invalid output Task")?;
        let source_artifact = output
            .from
            .artifact
            .as_ref()
            .ok_or("invalid output artifact")?;
        let port = statuses
            .iter()
            .find(|s| &s.name == source_task)
            .and_then(|s| {
                s.definition
                    .outputs
                    .artifacts
                    .iter()
                    .find(|p| &p.name == source_artifact)
            })
            .ok_or("unknown output port")?;
        let kind = port.kind;
        let recipe_name = unique_name(&recipe.name_any(), &self.graph);
        self.graph.push(ResolvedRecipeStatus {
            name: recipe_name.clone(),
            recipe_ref: ObjectReference {
                api_version: "skyw.top/v1beta1".into(),
                kind: "Recipe".into(),
                name: recipe.name_any(),
                uid: recipe.metadata.uid.clone().ok_or("Recipe UID missing")?,
                generation: recipe.metadata.generation.unwrap_or(1),
            },
            definition: recipe.spec.clone(),
            variables,
            phase: Phase::Pending,
            inputs: ResolvedInputs { artifacts: inputs },
            tasks: statuses,
            outputs: ResolvedOutputs {
                artifacts: recipe
                    .spec
                    .outputs
                    .artifacts
                    .iter()
                    .map(|o| (o.name.clone(), ResolvedOutput::default()))
                    .collect(),
            },
        });
        self.stack.pop();
        Ok(Choice {
            labels,
            kind,
            content_digest: None,
            reference: None,
            claim_ref: None,
            producer: Some((recipe_name, output.name.clone())),
        })
    }
}

fn diagnostic_reason(message: &str) -> &'static str {
    if message.starts_with("MissingParameter:")
        || message.starts_with("missing Recipe variable")
        || message.starts_with("missing variable")
        || message.starts_with("missing input label")
        || message.starts_with("unknown Recipe variable")
    {
        "MissingParameter"
    } else if message.starts_with("WaitingForTask:")
        || message.starts_with("UnsatisfiedDependency: task")
    {
        "WaitingForTask"
    } else if message.starts_with("WaitingForArtifact:") {
        "WaitingForArtifact"
    } else if message.starts_with("DependencyCycle") {
        "DependencyCycle"
    } else if message.starts_with("UnsatisfiedDependency") {
        "UnsatisfiedDependency"
    } else {
        "InvalidDefinition"
    }
}

fn frozen_labels(
    recipe: &ResolvedRecipeStatus,
    output: &fase_api::RecipeOutput,
) -> Result<Labels, String> {
    let mut labels = Labels::new();
    for (key, binding) in &output.labels {
        let value = match binding {
            LabelBinding::Literal(value) => value.value.clone(),
            LabelBinding::Bound(source) => match (&source.from_variable, &source.from_input) {
                (Some(name), None) => recipe
                    .variables
                    .get(name)
                    .ok_or("missing Recipe variable")?
                    .clone(),
                (None, Some(input)) => recipe
                    .inputs
                    .artifacts
                    .iter()
                    .find(|i| i.name == input.input)
                    .and_then(|i| i.labels.get(&input.label))
                    .ok_or("missing input label")?
                    .clone(),
                _ => return Err("invalid label binding".into()),
            },
        };
        labels.insert(key.clone(), value);
    }
    if !fase_api::valid_labels(&labels) {
        return Err("invalid output labels".into());
    }
    Ok(labels)
}

fn unique_name(base: &str, graph: &[ResolvedRecipeStatus]) -> String {
    if !graph.iter().any(|p| p.name == base) {
        return base.into();
    }
    for i in 2.. {
        let name = format!("{base}-{i}");
        if !graph.iter().any(|p| p.name == name) {
            return name;
        }
    }
    unreachable!()
}
fn forced_value(selector: &LabelSelector, key: &str) -> Option<String> {
    selector.match_labels.get(key).cloned().or_else(|| {
        selector
            .match_expressions
            .iter()
            .find(|e| e.key == key && e.operator == LabelOperator::In && e.values.len() == 1)
            .map(|e| e.values[0].clone())
    })
}
fn may_match(out: &fase_api::RecipeOutput, demand: &LabelSelector, variables: &Variables) -> bool {
    for (key, binding) in &out.labels {
        let known = match binding {
            LabelBinding::Literal(v) => Some(&v.value),
            LabelBinding::Bound(s) => s.from_variable.as_ref().and_then(|v| variables.get(v)),
        };
        if let Some(value) = known {
            if demand
                .match_labels
                .get(key)
                .is_some_and(|expected| expected != value)
            {
                return false;
            }
            if demand.match_expressions.iter().any(|e| {
                e.key == *key
                    && match e.operator {
                        LabelOperator::In => !e.values.contains(value),
                        LabelOperator::NotIn => e.values.contains(value),
                        LabelOperator::Exists => false,
                        LabelOperator::DoesNotExist => true,
                    }
            }) {
                return false;
            }
        }
    }
    !demand
        .match_labels
        .keys()
        .any(|k| !out.labels.contains_key(k))
        && !demand.match_expressions.iter().any(|e| {
            e.operator == LabelOperator::Exists && !out.labels.contains_key(&e.key)
                || e.operator == LabelOperator::DoesNotExist && out.labels.contains_key(&e.key)
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use fase_api::{
        ArtifactClaimSpec, ArtifactSpec, ObjectReference, Phase, ProducerReference,
        StorageReference,
    };
    use serde::Deserialize;

    fn example() -> (Vec<Task>, Vec<Recipe>, Request) {
        let mut tasks = Vec::new();
        let mut recipes = Vec::new();
        let mut request = None;
        for doc in serde_yml::Deserializer::from_str(include_str!("../../examples/hello.yaml")) {
            let value = serde_json::Value::deserialize(doc).unwrap();
            match value["kind"].as_str().unwrap() {
                "Task" => tasks.push(serde_json::from_value::<Task>(value).unwrap()),
                "Recipe" => recipes.push(serde_json::from_value::<Recipe>(value).unwrap()),
                "Request" => request = Some(serde_json::from_value::<Request>(value).unwrap()),
                other => panic!("unexpected resource {other}"),
            }
        }
        for (i, task) in tasks.iter_mut().enumerate() {
            task.metadata.uid = Some(format!("task-{i}"));
        }
        for (i, recipe) in recipes.iter_mut().enumerate() {
            recipe.metadata.uid = Some(format!("recipe-{i}"));
        }
        (tasks, recipes, request.unwrap())
    }

    #[test]
    fn expands_dependencies_and_propagates_requested_version() {
        let (tasks, recipes, request) = example();
        let graph = resolve(&request, &recipes, &tasks, &[], &[]).unwrap();
        assert_eq!(
            graph
                .recipes
                .iter()
                .map(|p| p.name.as_str())
                .collect::<Vec<_>>(),
            vec!["hello-source", "hello-package"]
        );
        assert_eq!(
            graph.recipes[1].inputs.artifacts[0].labels["version"],
            "1.0"
        );
        assert_eq!(
            graph.recipes[1].inputs.artifacts[0]
                .from
                .resolved_recipe
                .as_deref(),
            Some("hello-source")
        );
        assert_eq!(graph.target_output.as_deref(), Some("package"));
        assert_eq!(
            graph.recipes[0].tasks[0].variables["FASE_VAR_VERSION"],
            "1.0"
        );
        assert_eq!(graph.recipes[1].tasks[0].phase, Phase::Pending);
    }

    #[test]
    fn existing_claim_binds_without_source_recipe_and_multiple_claims_warn() {
        let (tasks, recipes, request) = example();
        let first = Artifact::new(
            "art-first",
            ArtifactSpec {
                content_digest: format!("sha256:{}", "0".repeat(64)),
                size_bytes: 0,
                kind: fase_api::ArtifactKind::Tree,
                storage_ref: StorageReference {
                    key: "objects/art-first".into(),
                },
            },
        );
        let object_ref = ObjectReference {
            api_version: "skyw.top/v1beta1".into(),
            kind: "Recipe".into(),
            name: "test".into(),
            uid: "uid".into(),
            generation: 1,
        };
        let mut claim = ArtifactClaim::new(
            "claim-first",
            ArtifactClaimSpec {
                artifact_ref: ArtifactReference {
                    name: "art-first".into(),
                },
                build_key: "sha256:test".into(),
                producer: ProducerReference {
                    recipe_ref: object_ref.clone(),
                    run_ref: object_ref,
                },
            },
        );
        claim.metadata.labels = Some(Labels::from([
            ("name".into(), "hello-source".into()),
            ("version".into(), "1.0".into()),
        ]));
        let graph = resolve(
            &request,
            &recipes,
            &tasks,
            &[first.clone()],
            &[claim.clone()],
        )
        .unwrap();
        assert_eq!(graph.recipes.len(), 1);
        assert_eq!(
            graph.recipes[0].inputs.artifacts[0]
                .artifact_ref
                .as_ref()
                .unwrap()
                .name,
            "art-first"
        );
        let mut second = claim.clone();
        second.metadata.name = Some("claim-second".into());
        let graph = resolve(&request, &recipes, &tasks, &[first], &[claim, second]).unwrap();
        assert!(
            graph
                .warnings
                .iter()
                .any(|w| w.contains("multiple ArtifactClaims"))
        );
    }

    #[test]
    fn rerun_builds_target_again_when_a_claim_already_exists() {
        let (tasks, recipes, mut request) = example();
        request.spec.rerun = 1;
        let artifact = Artifact::new(
            "art-old",
            ArtifactSpec {
                content_digest: format!("sha256:{}", "0".repeat(64)),
                size_bytes: 0,
                kind: fase_api::ArtifactKind::Tree,
                storage_ref: StorageReference {
                    key: "objects/art-old".into(),
                },
            },
        );
        let object_ref = ObjectReference {
            api_version: "skyw.top/v1beta1".into(),
            kind: "Recipe".into(),
            name: "old".into(),
            uid: "uid".into(),
            generation: 1,
        };
        let mut claim = ArtifactClaim::new(
            "claim-old",
            ArtifactClaimSpec {
                artifact_ref: ArtifactReference {
                    name: "art-old".into(),
                },
                build_key: "sha256:old".into(),
                producer: ProducerReference {
                    recipe_ref: object_ref.clone(),
                    run_ref: object_ref,
                },
            },
        );
        claim.metadata.labels = Some(Labels::from([
            ("name".into(), "hello-package".into()),
            ("version".into(), "1.0".into()),
            ("arch".into(), "amd64".into()),
        ]));
        let graph = resolve(&request, &recipes, &tasks, &[artifact], &[claim]).unwrap();
        assert!(graph.artifact_ref.is_none());
        assert_eq!(
            graph.recipes.last().unwrap().recipe_ref.name,
            "hello-package"
        );
    }

    #[test]
    fn missing_task_has_candidate_diagnostic_and_can_wait_for_definition() {
        let (tasks, recipes, mut request) = example();
        request
            .spec
            .artifact_selector
            .match_labels
            .insert("name".into(), "hello-source".into());
        request.spec.artifact_selector.match_labels.remove("arch");
        let tasks = tasks
            .into_iter()
            .filter(|task| task.name_any() != "make-source")
            .collect::<Vec<_>>();
        let error = resolve(&request, &recipes, &tasks, &[], &[]).unwrap_err();
        assert_eq!(error.reason, "WaitingForTask");
        assert_eq!(error.diagnostics[0].candidate, "hello-source/source");
        assert_eq!(error.diagnostics[0].reason, "WaitingForTask");

        let (tasks, recipes, request) = example();
        let tasks = tasks
            .into_iter()
            .filter(|task| task.name_any() != "make-source")
            .collect::<Vec<_>>();
        let error = resolve(&request, &recipes, &tasks, &[], &[]).unwrap_err();
        assert_eq!(error.reason, "WaitingForTask");
        assert_eq!(error.diagnostics[0].candidate, "hello-package/package");
    }

    #[test]
    fn two_inputs_reuse_one_resolved_producer() {
        let (tasks, mut recipes, request) = example();
        let mut second = recipes[1].spec.inputs.artifacts[0].clone();
        second.name = "also-source".into();
        recipes[1].spec.inputs.artifacts.push(second);
        let graph = resolve(&request, &recipes, &tasks, &[], &[]).unwrap();
        assert_eq!(graph.recipes.len(), 2);
        let inputs = &graph.recipes[1].inputs.artifacts;
        assert_eq!(
            inputs[0].from.resolved_recipe,
            inputs[1].from.resolved_recipe
        );
    }
}
