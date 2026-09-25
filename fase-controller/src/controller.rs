use crate::{
    job::{JobSettings, job_name, resources},
    resolve::resolve,
    transfer::{InputItem, JobManifest, OutputItem, OutputResult},
};
use fase_api::{
    Artifact, ArtifactReference, Condition, LabelBinding, Labels, Phase, Plan, Request,
    RequestStatus, ResolvedPlanStatus, Step, valid_labels,
};
use k8s_openapi::api::{batch::v1::Job, core::v1::ConfigMap};
use kube::{
    Api, Client, ResourceExt,
    api::{ListParams, Patch, PatchParams, PostParams},
};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
    time::Duration,
};

#[derive(Clone)]
pub struct Context {
    pub client: Client,
    pub settings: JobSettings,
}

pub async fn run() -> Result<(), String> {
    let context = Arc::new(Context {
        client: Client::try_default().await.map_err(|e| e.to_string())?,
        settings: JobSettings::from_env()?,
    });
    let namespace = std::env::var("FASE_NAMESPACE")
        .or_else(|_| std::env::var("POD_NAMESPACE"))
        .map_err(|_| "FASE_NAMESPACE or POD_NAMESPACE is required".to_string())?;
    tokio::spawn(crate::generator::run(
        context.client.clone(),
        namespace.clone(),
    ));
    loop {
        let requests: Api<Request> = Api::namespaced(context.client.clone(), &namespace);
        match requests.list(&ListParams::default()).await {
            Ok(list) => {
                for request in list {
                    if let Err(error) = reconcile(&context, &request).await {
                        tracing::error!(request=%request.name_any(),%error,"reconciliation failed");
                    }
                }
            }
            Err(error) => tracing::error!(%error,"failed to list Requests"),
        }
        tokio::time::sleep(Duration::from_secs(3)).await;
    }
}

pub async fn reconcile(context: &Context, request: &Request) -> Result<(), String> {
    if request.metadata.deletion_timestamp.is_some() {
        return Ok(());
    }
    let mut status = request.status.clone().unwrap_or_default();
    if matches!(status.phase, Some(Phase::Succeeded | Phase::Failed)) {
        return Ok(());
    }
    let digest = fase_api::spec_digest(&request.spec).map_err(|e| e.to_string())?;
    if let Some(frozen) = &status.request_digest {
        if frozen != &digest {
            fail(
                &mut status,
                "RequestChanged",
                "Request spec changed after resolution",
            );
            return patch_status(context, request, &status).await;
        }
    } else {
        let namespace = request.namespace().ok_or("Request namespace missing")?;
        let plans = Api::<Plan>::namespaced(context.client.clone(), &namespace)
            .list(&ListParams::default())
            .await
            .map_err(|e| e.to_string())?
            .items;
        let steps = Api::<Step>::namespaced(context.client.clone(), &namespace)
            .list(&ListParams::default())
            .await
            .map_err(|e| e.to_string())?
            .items;
        let artifacts = Api::<Artifact>::namespaced(context.client.clone(), &namespace)
            .list(&ListParams::default())
            .await
            .map_err(|e| e.to_string())?
            .items;
        match resolve(request, &plans, &steps, &artifacts) {
            Ok(graph) => {
                status.request_digest = Some(digest);
                status.resolved_plans = graph.plans;
                status.artifact_ref = graph.artifact_ref;
                status.target_output = graph.target_output;
                status.phase = Some(if status.artifact_ref.is_some() {
                    Phase::Succeeded
                } else {
                    Phase::Pending
                });
                status.conditions.push(condition(
                    "Resolved",
                    "True",
                    "PlanInputsBound",
                    "Plan, Step, variables and input bindings are fixed",
                ));
            }
            Err(error) if error.starts_with("UnsatisfiedDependency") => {
                if status.phase == Some(Phase::Pending)
                    && status
                        .conditions
                        .iter()
                        .any(|c| c.reason == "WaitingForArtifact" && c.message == error)
                {
                    return Ok(());
                }
                status.phase = Some(Phase::Pending);
                status.conditions =
                    vec![condition("Resolved", "False", "WaitingForArtifact", &error)];
            }
            Err(error) => fail(&mut status, "ResolutionFailed", &error),
        }
        return patch_status(context, request, &status).await;
    }
    let namespace = request.namespace().ok_or("Request namespace missing")?;
    for plan_index in 0..status.resolved_plans.len() {
        if status.resolved_plans[plan_index].phase == Phase::Succeeded {
            continue;
        }
        if !bind_inputs(&mut status, plan_index) {
            break;
        }
        let plan = status.resolved_plans[plan_index].clone();
        for step_index in 0..plan.steps.len() {
            let step = &plan.steps[step_index];
            if step.phase == Phase::Succeeded {
                continue;
            }
            if step.phase == Phase::Running || step.phase == Phase::Verifying {
                let name = step
                    .job_ref
                    .as_ref()
                    .ok_or("running Step has no Job reference")?
                    .name
                    .clone();
                let jobs: Api<Job> = Api::namespaced(context.client.clone(), &namespace);
                let Some(job) = jobs.get_opt(&name).await.map_err(|e| e.to_string())? else {
                    fail(
                        &mut status,
                        "JobMissing",
                        &format!("Job {name} disappeared"),
                    );
                    return patch_status(context, request, &status).await;
                };
                if job.status.as_ref().and_then(|s| s.succeeded).unwrap_or(0) > 0 {
                    let result = match read_result(context, &namespace, &name).await? {
                        Some(result) => result,
                        None => {
                            if step.phase == Phase::Verifying {
                                return Ok(());
                            }
                            status.resolved_plans[plan_index].steps[step_index].phase =
                                Phase::Verifying;
                            status.phase = Some(Phase::Verifying);
                            return patch_status(context, request, &status).await;
                        }
                    };
                    if let Err(error) =
                        validate_result(context, &namespace, &plan, step_index, &result).await
                    {
                        fail(&mut status, "InvalidResult", &error);
                        return patch_status(context, request, &status).await;
                    }
                    let current = &mut status.resolved_plans[plan_index].steps[step_index];
                    current.outputs = result
                        .outputs
                        .into_iter()
                        .map(|r| (r.name, r.artifact_ref))
                        .collect();
                    current.phase = Phase::Succeeded;
                    status.phase = Some(Phase::Running);
                } else if job.status.as_ref().and_then(|s| s.failed).unwrap_or(0) > 0 {
                    let current = &mut status.resolved_plans[plan_index].steps[step_index];
                    if current.attempt >= 2 {
                        current.phase = Phase::Failed;
                        status.resolved_plans[plan_index].phase = Phase::Failed;
                        fail(
                            &mut status,
                            "StepFailed",
                            &format!("Job {name} failed after 3 attempts"),
                        );
                    } else {
                        current.phase = Phase::Pending;
                        current.attempt += 1;
                        current.job_ref = None;
                    }
                } else {
                    if step.phase == Phase::Running {
                        return Ok(());
                    }
                    status.resolved_plans[plan_index].steps[step_index].phase = Phase::Running;
                }
                return patch_status(context, request, &status).await;
            }
            if step.phase == Phase::Failed {
                fail(
                    &mut status,
                    "StepFailed",
                    &format!("Step {} failed", step.name),
                );
                return patch_status(context, request, &status).await;
            }
            let call = plan
                .definition
                .steps
                .iter()
                .find(|c| c.name == step.name)
                .ok_or("frozen Plan missing Step")?;
            if call.inputs.artifacts.iter().any(|b| {
                b.from.step.as_ref().is_some_and(|dep| {
                    !plan
                        .steps
                        .iter()
                        .find(|s| &s.name == dep)
                        .is_some_and(|s| s.phase == Phase::Succeeded)
                })
            }) {
                continue;
            }
            let (manifest, injected_env) =
                manifest_for(context, request, &status, plan_index, step_index).await?;
            let name = job_name(
                request,
                &plan.name,
                step,
                step.attempt,
                &context.settings,
                &manifest,
            )?;
            let (config, job) = resources(
                request,
                step,
                &manifest,
                &name,
                &context.settings,
                injected_env,
            )?;
            create_if_missing(
                &Api::<ConfigMap>::namespaced(context.client.clone(), &namespace),
                &config,
            )
            .await?;
            create_if_missing(
                &Api::<Job>::namespaced(context.client.clone(), &namespace),
                &job,
            )
            .await?;
            let current = &mut status.resolved_plans[plan_index].steps[step_index];
            current.phase = Phase::Running;
            current.job_ref = Some(ArtifactReference { name });
            status.resolved_plans[plan_index].phase = Phase::Running;
            status.phase = Some(Phase::Running);
            return patch_status(context, request, &status).await;
        }
        if status.resolved_plans[plan_index]
            .steps
            .iter()
            .all(|s| s.phase == Phase::Succeeded)
        {
            let is_root = plan_index + 1 == status.resolved_plans.len();
            let plan = &mut status.resolved_plans[plan_index];
            for output in &plan.definition.outputs.artifacts {
                let step = output.from.step.as_ref().ok_or("invalid Plan output")?;
                let artifact = output.from.artifact.as_ref().ok_or("invalid Plan output")?;
                let reference = plan
                    .steps
                    .iter()
                    .find(|s| &s.name == step)
                    .and_then(|s| s.outputs.get(artifact))
                    .ok_or("missing committed output")?;
                plan.outputs
                    .artifacts
                    .get_mut(&output.name)
                    .ok_or("missing Plan output")?
                    .artifact_ref = Some(reference.clone());
            }
            plan.phase = Phase::Succeeded;
            if is_root {
                let target = status
                    .target_output
                    .as_ref()
                    .ok_or("target output missing")?;
                status.artifact_ref = plan
                    .outputs
                    .artifacts
                    .get(target)
                    .and_then(|o| o.artifact_ref.clone());
                status.phase = Some(Phase::Succeeded);
                status.conditions.push(condition(
                    "Completed",
                    "True",
                    "ArtifactCommitted",
                    "requested Artifact was committed",
                ));
            }
            return patch_status(context, request, &status).await;
        }
        break;
    }
    Ok(())
}

fn bind_inputs(status: &mut RequestStatus, index: usize) -> bool {
    let updates: Vec<_> = status.resolved_plans[index]
        .inputs
        .artifacts
        .iter()
        .map(|input| {
            if input.artifact_ref.is_some() {
                return Some(input.artifact_ref.clone().unwrap());
            }
            let source = input.from.resolved_plan.as_ref()?;
            let output = input.from.artifact.as_ref()?;
            status
                .resolved_plans
                .iter()
                .find(|p| &p.name == source)?
                .outputs
                .artifacts
                .get(output)?
                .artifact_ref
                .clone()
        })
        .collect();
    if updates.iter().any(Option::is_none) {
        return false;
    }
    for (input, reference) in status.resolved_plans[index]
        .inputs
        .artifacts
        .iter_mut()
        .zip(updates)
    {
        input.artifact_ref = reference;
    }
    true
}

async fn manifest_for(
    context: &Context,
    request: &Request,
    status: &RequestStatus,
    pi: usize,
    si: usize,
) -> Result<(JobManifest, Vec<Value>), String> {
    let namespace = request.namespace().ok_or("namespace missing")?;
    let plan = &status.resolved_plans[pi];
    let step = &plan.steps[si];
    let call = plan
        .definition
        .steps
        .iter()
        .find(|c| c.name == step.name)
        .ok_or("missing Step call")?;
    let artifacts: Api<Artifact> = Api::namespaced(context.client.clone(), &namespace);
    let mut inputs = Vec::new();
    let mut env = Vec::new();
    let mut env_names: BTreeSet<String> = ["FASE_INPUT_ROOT", "FASE_OUTPUT_ROOT", "FASE_RUN_ROOT"]
        .into_iter()
        .map(str::to_string)
        .collect();
    env_names.extend(step.variables.keys().cloned());
    env_names.extend(step.definition.env.iter().map(|e| e.name.clone()));
    for port in &step.definition.inputs.artifacts {
        let binding = call
            .inputs
            .artifacts
            .iter()
            .find(|b| b.name == port.name)
            .ok_or("missing Step input")?;
        let reference = match (
            &binding.from.plan_input,
            &binding.from.step,
            &binding.from.artifact,
        ) {
            (Some(input), None, None) => plan
                .inputs
                .artifacts
                .iter()
                .find(|i| &i.name == input)
                .and_then(|i| i.artifact_ref.clone()),
            (None, Some(source), Some(output)) => plan
                .steps
                .iter()
                .find(|s| &s.name == source)
                .and_then(|s| s.outputs.get(output).cloned()),
            _ => return Err("invalid Step input origin".into()),
        }
        .ok_or("input Artifact is not committed")?;
        let artifact = artifacts
            .get(&reference.name)
            .await
            .map_err(|e| e.to_string())?;
        let labels = artifact.metadata.labels.clone().unwrap_or_default();
        if let Some(expected) = plan
            .inputs
            .artifacts
            .iter()
            .find(|i| binding.from.plan_input.as_ref() == Some(&i.name))
            && labels != expected.labels
        {
            return Err("input Artifact labels changed after resolution".into());
        }
        for (key, value) in &labels {
            let name = format!("INPUTS_{}_LABELS_{}", normalize(&port.name), normalize(key));
            if !env_names.insert(name.clone()) {
                return Err(format!("input label env collision: {name}"));
            }
            env.push(json!({"name":name,"value":value}));
        }
        inputs.push(InputItem {
            path: port.path.clone(),
            format: port.format,
            artifact_name: reference.name,
            artifact_spec: artifact.spec,
        });
    }
    let outputs = step
        .definition
        .outputs
        .artifacts
        .iter()
        .filter(|port| call.outputs.iter().any(|o| o.name == port.name))
        .map(|port| {
            let final_output = plan.definition.outputs.artifacts.iter().find(|o| {
                o.from.step.as_deref() == Some(&step.name)
                    && o.from.artifact.as_deref() == Some(&port.name)
            });
            let (name, labels) = match final_output {
                Some(o) => (o.name.clone(), evaluate_labels(plan, &o.labels)?),
                None => (
                    port.name.clone(),
                    BTreeMap::from([("skyw.top/internal".into(), "true".into())]),
                ),
            };
            Ok(OutputItem {
                name: port.name.clone(),
                path: port.path.clone(),
                format: port.format,
                artifact_name: name,
                artifact_type: port
                    .artifact_type
                    .clone()
                    .unwrap_or_else(|| format_name(port.format).into()),
                labels,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok((
        JobManifest {
            inputs,
            outputs,
            namespace,
            request_name: request.name_any(),
            request_uid: request.metadata.uid.clone().ok_or("Request UID missing")?,
        },
        env,
    ))
}

fn evaluate_labels(
    plan: &ResolvedPlanStatus,
    bindings: &BTreeMap<String, LabelBinding>,
) -> Result<Labels, String> {
    let mut labels = Labels::new();
    for (key, binding) in bindings {
        let value = match binding {
            LabelBinding::Literal(v) => v.value.clone(),
            LabelBinding::Bound(source) => match (&source.from_variable, &source.from_input) {
                (Some(v), None) => plan
                    .variables
                    .get(v)
                    .ok_or("unknown output variable")?
                    .clone(),
                (None, Some(i)) => plan
                    .inputs
                    .artifacts
                    .iter()
                    .find(|a| a.name == i.input)
                    .and_then(|a| a.labels.get(&i.label))
                    .ok_or("unknown output input label")?
                    .clone(),
                _ => return Err("invalid output label binding".into()),
            },
        };
        labels.insert(key.clone(), value);
    }
    if !valid_labels(&labels) {
        return Err("invalid Plan output labels".into());
    }
    Ok(labels)
}
fn normalize(value: &str) -> String {
    value
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .collect()
}
fn format_name(format: fase_api::ArtifactFormat) -> &'static str {
    match format {
        fase_api::ArtifactFormat::File => "file",
        fase_api::ArtifactFormat::Directory => "directory",
        fase_api::ArtifactFormat::Archive => "tar.zst",
    }
}

async fn read_result(
    context: &Context,
    namespace: &str,
    job: &str,
) -> Result<Option<OutputResult>, String> {
    let api: Api<ConfigMap> = Api::namespaced(context.client.clone(), namespace);
    let Some(result) = api
        .get_opt(&format!("{job}-result"))
        .await
        .map_err(|e| e.to_string())?
    else {
        return Ok(None);
    };
    serde_json::from_str(
        result
            .data
            .as_ref()
            .and_then(|d| d.get("result.json"))
            .ok_or("result.json missing")?,
    )
    .map(Some)
    .map_err(|e| e.to_string())
}

async fn validate_result(
    context: &Context,
    namespace: &str,
    plan: &ResolvedPlanStatus,
    si: usize,
    result: &OutputResult,
) -> Result<(), String> {
    let step = &plan.steps[si];
    let call = plan
        .definition
        .steps
        .iter()
        .find(|c| c.name == step.name)
        .ok_or("missing Step call")?;
    if call.outputs.len() != result.outputs.len() {
        return Err("output count mismatch".into());
    }
    let artifacts: Api<Artifact> = Api::namespaced(context.client.clone(), namespace);
    for receipt in &result.outputs {
        let port = step
            .definition
            .outputs
            .artifacts
            .iter()
            .find(|p| p.name == receipt.name)
            .ok_or("unknown output receipt")?;
        if !call.outputs.iter().any(|o| o.name == port.name)
            || receipt.format != port.format
            || receipt.artifact_ref.name
                != crate::storage::artifact_id(&receipt.sha256, receipt.format)
            || receipt.size < 0
        {
            return Err("invalid output receipt".into());
        }
        let artifact = artifacts
            .get(&receipt.artifact_ref.name)
            .await
            .map_err(|e| e.to_string())?;
        if artifact.spec.artifact_type
            != port
                .artifact_type
                .clone()
                .unwrap_or_else(|| format_name(port.format).into())
        {
            return Err("output type mismatch".into());
        }
        for output in &plan.definition.outputs.artifacts {
            if output.from.step.as_deref() == Some(&step.name)
                && output.from.artifact.as_deref() == Some(&port.name)
            {
                let labels = evaluate_labels(plan, &output.labels)?;
                if artifact.metadata.labels.as_ref() != Some(&labels) {
                    return Err("output labels violate Plan contract".into());
                }
            }
        }
    }
    Ok(())
}
fn fail(status: &mut RequestStatus, reason: &str, message: &str) {
    status.phase = Some(Phase::Failed);
    status
        .conditions
        .push(condition("Completed", "False", reason, message));
}
fn condition(kind: &str, status: &str, reason: &str, message: &str) -> Condition {
    Condition {
        r#type: kind.into(),
        status: status.into(),
        reason: reason.into(),
        message: message.into(),
        last_transition_time: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
    }
}
async fn patch_status(
    context: &Context,
    request: &Request,
    status: &RequestStatus,
) -> Result<(), String> {
    let api: Api<Request> = Api::namespaced(
        context.client.clone(),
        &request.namespace().ok_or("namespace missing")?,
    );
    api.patch_status(
        &request.name_any(),
        &PatchParams::default(),
        &Patch::Merge(
            json!({"metadata":{"resourceVersion":request.resource_version()},"status":status}),
        ),
    )
    .await
    .map_err(|e| e.to_string())?;
    Ok(())
}
async fn create_if_missing<K>(api: &Api<K>, value: &K) -> Result<(), String>
where
    K: kube::Resource<DynamicType = ()>
        + Clone
        + serde::Serialize
        + serde::de::DeserializeOwned
        + std::fmt::Debug,
{
    match api.create(&PostParams::default(), value).await {
        Ok(_) => Ok(()),
        Err(kube::Error::Api(e)) if e.code == 409 => {
            let existing = api
                .get(&value.name_any())
                .await
                .map_err(|e| e.to_string())?;
            if equivalent_resource(&existing, value)? {
                Ok(())
            } else {
                Err(format!("existing {} conflicts", value.name_any()))
            }
        }
        Err(e) => Err(e.to_string()),
    }
}
fn equivalent_resource<K: serde::Serialize>(existing: &K, desired: &K) -> Result<bool, String> {
    let mut a = serde_json::to_value(existing).map_err(|e| e.to_string())?;
    let mut b = serde_json::to_value(desired).map_err(|e| e.to_string())?;
    for v in [&mut a, &mut b] {
        if let Some(o) = v.as_object_mut() {
            o.remove("status");
        }
        if let Some(m) = v.get_mut("metadata").and_then(Value::as_object_mut) {
            for k in [
                "resourceVersion",
                "uid",
                "creationTimestamp",
                "deletionTimestamp",
                "managedFields",
                "generation",
                "selfLink",
            ] {
                m.remove(k);
            }
        }
    }
    Ok(a == b)
}
