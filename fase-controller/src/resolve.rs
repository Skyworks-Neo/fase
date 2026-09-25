use fase_api::{
    Artifact, ArtifactReference, LabelBinding, LabelOperator, LabelSelector, Labels, Phase, Plan,
    PlanReference, Request, ResolvedInput, ResolvedInputOrigin, ResolvedInputs, ResolvedOutput,
    ResolvedOutputs, ResolvedPlanStatus, Step, StepStatus, Variables, spec_digest,
    valid_label_value, valid_name,
};
use kube::ResourceExt;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone)]
struct Choice {
    labels: Labels,
    artifact_type: String,
    reference: Option<ArtifactReference>,
    producer: Option<(String, String)>,
}

#[derive(Debug)]
pub struct Resolution {
    pub plans: Vec<ResolvedPlanStatus>,
    pub artifact_ref: Option<ArtifactReference>,
    pub target_output: Option<String>,
}

struct Solver<'a> {
    request: &'a Request,
    plans: &'a [Plan],
    steps: &'a [Step],
    artifacts: &'a [Artifact],
    graph: Vec<ResolvedPlanStatus>,
    stack: Vec<String>,
}

pub fn resolve(
    request: &Request,
    plans: &[Plan],
    steps: &[Step],
    artifacts: &[Artifact],
) -> Result<Resolution, String> {
    request.spec.artifact_selector.validate()?;
    if request.spec.artifact_selector.is_empty() {
        return Err("artifactSelector is empty".into());
    }
    let mut solver = Solver {
        request,
        plans,
        steps,
        artifacts,
        graph: Vec::new(),
        stack: Vec::new(),
    };
    let choice = solver.choose(&request.spec.artifact_selector)?;
    Ok(Resolution {
        plans: solver.graph,
        artifact_ref: choice.reference,
        target_output: choice.producer.map(|p| p.1),
    })
}

impl Solver<'_> {
    fn choose(&mut self, selector: &LabelSelector) -> Result<Choice, String> {
        selector.validate()?;
        let existing: Vec<_> = self
            .artifacts
            .iter()
            .filter(|a| a.metadata.deletion_timestamp.is_none())
            .filter(|a| {
                a.metadata
                    .labels
                    .as_ref()
                    .is_some_and(|l| selector.matches(l))
            })
            .collect();
        if existing.len() > 1 {
            return Err(format!(
                "AmbiguousArtifact: {} artifacts match",
                existing.len()
            ));
        }
        if let Some(a) = existing.first() {
            if fase_api::artifact_format(&a.spec.artifact_type).is_none() {
                return Err(format!("Artifact {} has unsupported type", a.name_any()));
            }
            return Ok(Choice {
                labels: a.metadata.labels.clone().unwrap_or_default(),
                artifact_type: a.spec.artifact_type.clone(),
                reference: Some(ArtifactReference { name: a.name_any() }),
                producer: None,
            });
        }
        let planned: Vec<_> = self
            .graph
            .iter()
            .flat_map(|plan| {
                plan.definition
                    .outputs
                    .artifacts
                    .iter()
                    .filter_map(move |output| {
                        let labels = frozen_labels(plan, output).ok()?;
                        if !selector.matches(&labels) {
                            return None;
                        }
                        let step = plan
                            .steps
                            .iter()
                            .find(|s| Some(&s.name) == output.from.step.as_ref())?;
                        let port = step
                            .definition
                            .outputs
                            .artifacts
                            .iter()
                            .find(|p| Some(&p.name) == output.from.artifact.as_ref())?;
                        Some(Choice {
                            labels,
                            artifact_type: port_type(port),
                            reference: None,
                            producer: Some((plan.name.clone(), output.name.clone())),
                        })
                    })
            })
            .collect();
        if planned.len() > 1 {
            return Err(format!(
                "AmbiguousProducer: {} resolved outputs match",
                planned.len()
            ));
        }
        if let Some(choice) = planned.into_iter().next() {
            return Ok(choice);
        }
        let mut viable = Vec::new();
        let mut errors = Vec::new();
        for plan in self
            .plans
            .iter()
            .filter(|p| p.metadata.deletion_timestamp.is_none())
        {
            for output in &plan.spec.outputs.artifacts {
                if !may_match(output, selector, &self.request.spec.variables) {
                    continue;
                }
                let mut branch = Solver {
                    request: self.request,
                    plans: self.plans,
                    steps: self.steps,
                    artifacts: self.artifacts,
                    graph: self.graph.clone(),
                    stack: self.stack.clone(),
                };
                match branch.expand(plan, output, selector) {
                    Ok(choice) => viable.push((choice, branch.graph)),
                    Err(error) => errors.push(error),
                }
            }
        }
        match viable.len() {
            1 => {
                let (choice, graph) = viable.remove(0);
                self.graph = graph;
                Ok(choice)
            }
            n if n > 1 => Err(format!("AmbiguousProducer: {n} Plan outputs match")),
            _ => Err(errors
                .iter()
                .find(|e| !e.starts_with("UnsatisfiedDependency"))
                .or_else(|| errors.first())
                .cloned()
                .unwrap_or_else(|| {
                    "UnsatisfiedDependency: no Artifact or Plan output satisfies selector".into()
                })),
        }
    }

    fn expand(
        &mut self,
        plan: &Plan,
        output: &fase_api::PlanOutput,
        demand: &LabelSelector,
    ) -> Result<Choice, String> {
        let output_names: BTreeSet<_> = plan
            .spec
            .outputs
            .artifacts
            .iter()
            .map(|o| &o.name)
            .collect();
        if output_names.len() != plan.spec.outputs.artifacts.len()
            || plan.spec.outputs.artifacts.is_empty()
            || output_names.iter().any(|name| !valid_name(name))
        {
            return Err("invalid or duplicate Plan output".into());
        }
        let identity = format!("{}/{}", plan.name_any(), output.name);
        if self.stack.contains(&identity) {
            return Err(format!("DependencyCycle: {identity}"));
        }
        if self.stack.len() >= 32 {
            return Err("DependencyCycle: depth limit".into());
        }
        self.stack.push(identity);
        let mut variables = Variables::new();
        let mut declared = BTreeSet::new();
        for var in &plan.spec.inputs.variables {
            if !valid_name(&var.name) || !declared.insert(var.name.clone()) {
                return Err(format!("invalid Plan variable {}", var.name));
            }
            if let Some(v) = self.request.spec.variables.get(&var.name) {
                variables.insert(var.name.clone(), v.clone());
            } else if var.required {
                return Err(format!("missing Plan variable {}", var.name));
            }
        }
        let mut inputs = Vec::new();
        let mut input_choices = BTreeMap::new();
        for input in &plan.spec.inputs.artifacts {
            if !valid_name(&input.name) || input_choices.contains_key(&input.name) {
                return Err(format!("invalid Plan input {}", input.name));
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
            let chosen = self.choose(&selector)?;
            inputs.push(ResolvedInput {
                name: input.name.clone(),
                from: ResolvedInputOrigin {
                    resolved_plan: chosen.producer.as_ref().map(|p| p.0.clone()),
                    artifact: chosen.producer.as_ref().map(|p| p.1.clone()),
                },
                labels: chosen.labels.clone(),
                artifact_type: chosen.artifact_type.clone(),
                artifact_ref: chosen.reference.clone(),
            });
            input_choices.insert(input.name.clone(), chosen);
        }
        let mut labels = Labels::new();
        for (key, binding) in &output.labels {
            let value = match binding {
                LabelBinding::Literal(v) => v.clone(),
                LabelBinding::Bound(source) => match (&source.from_variable, &source.from_input) {
                    (Some(v), None) => variables
                        .get(v)
                        .ok_or_else(|| format!("unknown Plan variable {v}"))?
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
            return Err("invalid Plan output labels".into());
        }
        if !demand.matches(&labels) {
            return Err("UnsatisfiedDependency: output labels do not match selector".into());
        }
        let mut pending: BTreeMap<String, &fase_api::PlanStep> = plan
            .spec
            .steps
            .iter()
            .map(|s| (s.name.clone(), s))
            .collect();
        if pending.len() != plan.spec.steps.len() || pending.is_empty() {
            return Err("duplicate or empty Plan steps".into());
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
                    return Err(format!("duplicate Step binding in {name}"));
                }
                let deps: Vec<_> = call
                    .inputs
                    .artifacts
                    .iter()
                    .filter_map(|b| b.from.step.as_ref())
                    .collect();
                if deps
                    .iter()
                    .any(|d| !statuses.iter().any(|s: &StepStatus| &s.name == *d))
                {
                    continue;
                }
                call.step_selector.validate()?;
                if call.step_selector.is_empty() {
                    return Err(format!("step {name} has empty selector"));
                }
                let matches: Vec<_> = self
                    .steps
                    .iter()
                    .filter(|s| s.metadata.deletion_timestamp.is_none())
                    .filter(|s| {
                        call.step_selector
                            .matches(s.metadata.labels.as_ref().unwrap_or(&Labels::new()))
                    })
                    .collect();
                if matches.len() != 1 {
                    return Err(format!(
                        "step {name} selector matched {} Steps",
                        matches.len()
                    ));
                }
                let step = matches[0];
                step.spec.validate()?;
                let mut env = Variables::new();
                for v in &step.spec.inputs.variables {
                    let b = call.variables.get(&v.name);
                    let value = match b {
                        Some(b) => match (&b.value, &b.from_variable) {
                            (Some(v), None) => Some(v.clone()),
                            (None, Some(from)) => Some(
                                variables
                                    .get(from)
                                    .ok_or_else(|| format!("missing Plan variable {from}"))?
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
                    != step
                        .spec
                        .inputs
                        .variables
                        .iter()
                        .filter(|v| call.variables.contains_key(&v.name))
                        .count()
                {
                    return Err(format!("undeclared variable binding in {name}"));
                }
                if call.inputs.artifacts.len() != step.spec.inputs.artifacts.len() {
                    return Err(format!("input count mismatch in {name}"));
                }
                for port in &step.spec.inputs.artifacts {
                    let b = call
                        .inputs
                        .artifacts
                        .iter()
                        .find(|b| b.name == port.name)
                        .ok_or_else(|| format!("missing input {}", port.name))?;
                    let origin = &b.from;
                    let artifact_type = match (&origin.plan_input, &origin.step, &origin.artifact) {
                        (Some(i), None, None) => input_choices
                            .get(i)
                            .ok_or_else(|| format!("unknown Plan input {i}"))?
                            .artifact_type
                            .clone(),
                        (None, Some(s), Some(a)) => {
                            let producer = statuses
                                .iter()
                                .find(|item: &&StepStatus| &item.name == s)
                                .ok_or_else(|| format!("unknown previous Step {s}"))?;
                            if !plan
                                .spec
                                .steps
                                .iter()
                                .find(|call| &call.name == s)
                                .is_some_and(|call| call.outputs.iter().any(|out| &out.name == a))
                            {
                                return Err(format!("Step output {s}.{a} is not exposed"));
                            }
                            producer
                                .definition
                                .outputs
                                .artifacts
                                .iter()
                                .find(|p| &p.name == a)
                                .map(port_type)
                                .ok_or_else(|| format!("unknown Step output {s}.{a}"))?
                        }
                        _ => return Err("invalid Step artifact origin".into()),
                    };
                    if fase_api::artifact_format(&artifact_type) != Some(port.format)
                        || port
                            .artifact_type
                            .as_ref()
                            .is_some_and(|t| t != &artifact_type)
                    {
                        return Err(format!("type mismatch in {name}.{}", port.name));
                    }
                }
                for out in &call.outputs {
                    if !step
                        .spec
                        .outputs
                        .artifacts
                        .iter()
                        .any(|p| p.name == out.name)
                    {
                        return Err(format!("unknown output {}", out.name));
                    }
                }
                statuses.push(StepStatus {
                    name: name.clone(),
                    phase: Phase::Pending,
                    attempt: 0,
                    step_ref: PlanReference {
                        name: step.name_any(),
                        uid: step.metadata.uid.clone().ok_or("Step UID missing")?,
                        digest: spec_digest(&step.spec).map_err(|e| e.to_string())?,
                    },
                    definition: step.spec.clone(),
                    variables: env,
                    job_ref: None,
                    outputs: BTreeMap::new(),
                    message: None,
                });
                pending.remove(&name);
            }
            if before == pending.len() {
                return Err("DependencyCycle: Step inputs form a cycle".into());
            }
        }
        for out in &plan.spec.outputs.artifacts {
            let from = &out.from;
            let step = from
                .step
                .as_ref()
                .ok_or("Plan output must reference Step")?;
            let artifact = from
                .artifact
                .as_ref()
                .ok_or("Plan output missing artifact")?;
            let status = statuses
                .iter()
                .find(|s| &s.name == step)
                .ok_or("unknown output Step")?;
            if !plan
                .spec
                .steps
                .iter()
                .find(|s| &s.name == step)
                .is_some_and(|s| s.outputs.iter().any(|o| &o.name == artifact))
            {
                return Err("Plan output is not exposed by Step call".into());
            }
            if !status
                .definition
                .outputs
                .artifacts
                .iter()
                .any(|p| &p.name == artifact)
            {
                return Err("unknown Plan output artifact".into());
            }
        }
        let source_step = output.from.step.as_ref().ok_or("invalid output Step")?;
        let source_artifact = output
            .from
            .artifact
            .as_ref()
            .ok_or("invalid output artifact")?;
        let port = statuses
            .iter()
            .find(|s| &s.name == source_step)
            .and_then(|s| {
                s.definition
                    .outputs
                    .artifacts
                    .iter()
                    .find(|p| &p.name == source_artifact)
            })
            .ok_or("unknown output port")?;
        let artifact_type = port_type(port);
        let plan_name = unique_name(&plan.name_any(), &self.graph);
        self.graph.push(ResolvedPlanStatus {
            name: plan_name.clone(),
            plan_ref: PlanReference {
                name: plan.name_any(),
                uid: plan.metadata.uid.clone().ok_or("Plan UID missing")?,
                digest: spec_digest(&plan.spec).map_err(|e| e.to_string())?,
            },
            definition: plan.spec.clone(),
            variables,
            phase: Phase::Pending,
            inputs: ResolvedInputs { artifacts: inputs },
            steps: statuses,
            outputs: ResolvedOutputs {
                artifacts: plan
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
            artifact_type,
            reference: None,
            producer: Some((plan_name, output.name.clone())),
        })
    }
}

fn frozen_labels(
    plan: &ResolvedPlanStatus,
    output: &fase_api::PlanOutput,
) -> Result<Labels, String> {
    let mut labels = Labels::new();
    for (key, binding) in &output.labels {
        let value = match binding {
            LabelBinding::Literal(value) => value.clone(),
            LabelBinding::Bound(source) => match (&source.from_variable, &source.from_input) {
                (Some(name), None) => plan
                    .variables
                    .get(name)
                    .ok_or("missing Plan variable")?
                    .clone(),
                (None, Some(input)) => plan
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

fn port_type(port: &fase_api::ArtifactPort) -> String {
    port.artifact_type.clone().unwrap_or_else(|| {
        match port.format {
            fase_api::ArtifactFormat::File => "file",
            fase_api::ArtifactFormat::Directory => "directory",
            fase_api::ArtifactFormat::Archive => "tar.zst",
        }
        .into()
    })
}
fn unique_name(base: &str, graph: &[ResolvedPlanStatus]) -> String {
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
fn may_match(out: &fase_api::PlanOutput, demand: &LabelSelector, variables: &Variables) -> bool {
    for (key, binding) in &out.labels {
        let known = match binding {
            LabelBinding::Literal(v) => Some(v),
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
    use fase_api::{ArtifactSpec, Phase};
    use serde::Deserialize;

    fn example() -> (Vec<Step>, Vec<Plan>, Request) {
        let mut steps = Vec::new();
        let mut plans = Vec::new();
        let mut request = None;
        for doc in serde_yml::Deserializer::from_str(include_str!("../../examples/hello.yaml")) {
            let value = serde_json::Value::deserialize(doc).unwrap();
            match value["kind"].as_str().unwrap() {
                "Step" => steps.push(serde_json::from_value::<Step>(value).unwrap()),
                "Plan" => plans.push(serde_json::from_value::<Plan>(value).unwrap()),
                "Request" => request = Some(serde_json::from_value::<Request>(value).unwrap()),
                other => panic!("unexpected resource {other}"),
            }
        }
        for (i, step) in steps.iter_mut().enumerate() {
            step.metadata.uid = Some(format!("step-{i}"));
        }
        for (i, plan) in plans.iter_mut().enumerate() {
            plan.metadata.uid = Some(format!("plan-{i}"));
        }
        (steps, plans, request.unwrap())
    }

    #[test]
    fn expands_dependencies_and_propagates_requested_version() {
        let (steps, plans, request) = example();
        let graph = resolve(&request, &plans, &steps, &[]).unwrap();
        assert_eq!(
            graph
                .plans
                .iter()
                .map(|p| p.name.as_str())
                .collect::<Vec<_>>(),
            vec!["hello-source", "hello-package"]
        );
        assert_eq!(graph.plans[1].inputs.artifacts[0].labels["version"], "1.0");
        assert_eq!(
            graph.plans[1].inputs.artifacts[0]
                .from
                .resolved_plan
                .as_deref(),
            Some("hello-source")
        );
        assert_eq!(graph.target_output.as_deref(), Some("package"));
        assert_eq!(graph.plans[0].steps[0].variables["FASE_VAR_VERSION"], "1.0");
        assert_eq!(graph.plans[1].steps[0].phase, Phase::Pending);
    }

    #[test]
    fn existing_artifact_binds_without_source_plan_and_ambiguity_fails() {
        let (steps, plans, request) = example();
        let mut first = Artifact::new(
            "art-first",
            ArtifactSpec {
                name: "source".into(),
                artifact_type: "directory".into(),
            },
        );
        first.metadata.labels = Some(Labels::from([
            ("name".into(), "hello-source".into()),
            ("version".into(), "1.0".into()),
        ]));
        let graph = resolve(&request, &plans, &steps, &[first.clone()]).unwrap();
        assert_eq!(graph.plans.len(), 1);
        assert_eq!(
            graph.plans[0].inputs.artifacts[0]
                .artifact_ref
                .as_ref()
                .unwrap()
                .name,
            "art-first"
        );
        let mut second = first.clone();
        second.metadata.name = Some("art-second".into());
        assert!(
            resolve(&request, &plans, &steps, &[first, second])
                .unwrap_err()
                .contains("AmbiguousArtifact")
        );
    }

    #[test]
    fn two_inputs_reuse_one_resolved_producer() {
        let (steps, mut plans, request) = example();
        let mut second = plans[1].spec.inputs.artifacts[0].clone();
        second.name = "also-source".into();
        plans[1].spec.inputs.artifacts.push(second);
        let graph = resolve(&request, &plans, &steps, &[]).unwrap();
        assert_eq!(graph.plans.len(), 2);
        let inputs = &graph.plans[1].inputs.artifacts;
        assert_eq!(inputs[0].from.resolved_plan, inputs[1].from.resolved_plan);
    }
}
