use crate::{
    job::{JobSettings, job_name, resources},
    resolve::resolve,
    transfer::{InputItem, JobManifest, OutputItem, OutputResult},
};
use fase_api::{
    Artifact, ArtifactClaim, ArtifactReference, Condition, FailureClass, LabelBinding, Labels,
    ObjectReference, Phase, ProducerReference, Recipe, RecipeSpec, Request, RequestStatus,
    ResolvedRecipeStatus, Run, RunSpec, RunStatus, Task, TaskSpec, Variables, valid_labels,
};
use k8s_openapi::api::{
    batch::v1::Job,
    core::v1::{ConfigMap, Pod},
};
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
                    if let Err(error) = reconcile_request(&context, &request).await {
                        tracing::error!(request=%request.name_any(),%error,"reconciliation failed");
                    }
                }
            }
            Err(error) => tracing::error!(%error,"failed to list Requests"),
        }
        let runs: Api<Run> = Api::namespaced(context.client.clone(), &namespace);
        match runs.list(&ListParams::default()).await {
            Ok(list) => {
                for run in list {
                    if let Err(error) = reconcile_run(&context, &run).await {
                        tracing::error!(run=%run.name_any(),%error,"Run reconciliation failed");
                    }
                }
            }
            Err(error) => tracing::error!(%error,"failed to list Runs"),
        }
        tokio::time::sleep(Duration::from_secs(3)).await;
    }
}

async fn reconcile_request(context: &Context, request: &Request) -> Result<(), String> {
    if request.metadata.deletion_timestamp.is_some() {
        return Ok(());
    }
    let namespace = request.namespace().ok_or("Request namespace missing")?;
    let uid = request.metadata.uid.clone().ok_or("Request UID missing")?;
    let generation = request.metadata.generation.unwrap_or(1);
    let name = format!(
        "run-{}",
        &fase_api::sha256_hex(format!("{uid}:{generation}").as_bytes())[..32]
    );
    let mut run = Run::new(
        &name,
        RunSpec {
            request_ref: ObjectReference {
                api_version: "skyw.top/v1beta1".into(),
                kind: "Request".into(),
                name: request.name_any(),
                uid,
                generation,
            },
        },
    );
    run.metadata.namespace = Some(namespace.clone());
    let runs: Api<Run> = Api::namespaced(context.client.clone(), &namespace);
    create_if_missing(&runs, &run).await?;
    let current = runs.get(&name).await.map_err(|e| e.to_string())?;
    let current_status = current.status.unwrap_or_default();
    let next = RequestStatus {
        phase: current_status.phase.or(Some(Phase::Pending)),
        run_ref: Some(ArtifactReference { name }),
        claim_ref: current_status.claim_ref,
        artifact_ref: current_status.artifact_ref,
        conditions: current_status.conditions,
    };
    if serde_json::to_value(request.status.as_ref().unwrap_or(&RequestStatus::default()))
        .map_err(|e| e.to_string())?
        != serde_json::to_value(&next).map_err(|e| e.to_string())?
    {
        let api: Api<Request> = Api::namespaced(context.client.clone(), &namespace);
        api.patch_status(
            &request.name_any(),
            &PatchParams::default(),
            &Patch::Merge(
                json!({"metadata":{"resourceVersion":request.resource_version()},"status":next}),
            ),
        )
        .await
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

async fn hydrate(context: &Context, namespace: &str, status: &mut RunStatus) -> Result<(), String> {
    let recipes: Api<Recipe> = Api::namespaced(context.client.clone(), namespace);
    let tasks: Api<Task> = Api::namespaced(context.client.clone(), namespace);
    for item in &mut status.resolved_recipes {
        let recipe = recipes
            .get(&item.recipe_ref.name)
            .await
            .map_err(|e| e.to_string())?;
        if recipe.metadata.uid.as_deref() != Some(&item.recipe_ref.uid)
            || recipe.metadata.generation.unwrap_or(1) != item.recipe_ref.generation
        {
            return Err(format!(
                "Recipe {} changed after resolution",
                item.recipe_ref.name
            ));
        }
        item.definition = recipe.spec;
        for task in &mut item.tasks {
            let definition = tasks
                .get(&task.task_ref.name)
                .await
                .map_err(|e| e.to_string())?;
            if definition.metadata.uid.as_deref() != Some(&task.task_ref.uid)
                || definition.metadata.generation.unwrap_or(1) != task.task_ref.generation
            {
                return Err(format!(
                    "Task {} changed after resolution",
                    task.task_ref.name
                ));
            }
            task.definition = definition.spec;
        }
    }
    Ok(())
}

pub async fn reconcile_run(context: &Context, run: &Run) -> Result<(), String> {
    if run.metadata.deletion_timestamp.is_some() {
        return Ok(());
    }
    let namespace = run.namespace().ok_or("Run namespace missing")?;
    let request: Request = Api::<Request>::namespaced(context.client.clone(), &namespace)
        .get(&run.spec.request_ref.name)
        .await
        .map_err(|e| e.to_string())?;
    if request.metadata.uid.as_deref() != Some(&run.spec.request_ref.uid) {
        return Err("Run Request reference has changed identity".into());
    }
    let mut status = run.status.clone().unwrap_or_default();
    if matches!(status.phase, Some(Phase::Succeeded | Phase::Failed)) {
        return Ok(());
    }
    if request.metadata.generation.unwrap_or(1) != run.spec.request_ref.generation {
        fail(
            &mut status,
            "RequestChanged",
            "Request generation changed after Run creation",
        );
        return patch_status(context, run, &status).await;
    }
    if let Err(error) = hydrate(context, &namespace, &mut status).await {
        fail(&mut status, "DefinitionChanged", &error);
        return patch_status(context, run, &status).await;
    }
    let digest = fase_api::spec_digest(&request.spec).map_err(|e| e.to_string())?;
    if let Some(frozen) = &status.request_digest {
        if frozen != &digest {
            fail(
                &mut status,
                "RequestChanged",
                "Request spec changed after resolution",
            );
            return patch_status(context, run, &status).await;
        }
    } else {
        let namespace = request.namespace().ok_or("Request namespace missing")?;
        let recipes = Api::<Recipe>::namespaced(context.client.clone(), &namespace)
            .list(&ListParams::default())
            .await
            .map_err(|e| e.to_string())?
            .items;
        let tasks = Api::<Task>::namespaced(context.client.clone(), &namespace)
            .list(&ListParams::default())
            .await
            .map_err(|e| e.to_string())?
            .items;
        let artifacts = Api::<Artifact>::namespaced(context.client.clone(), &namespace)
            .list(&ListParams::default())
            .await
            .map_err(|e| e.to_string())?
            .items;
        let claims = Api::<ArtifactClaim>::namespaced(context.client.clone(), &namespace)
            .list(&ListParams::default())
            .await
            .map_err(|e| e.to_string())?
            .items;
        match resolve(&request, &recipes, &tasks, &artifacts, &claims) {
            Ok(graph) => {
                for warning in &graph.warnings {
                    emit_warning(context, run, warning).await?;
                }
                status.request_digest = Some(digest);
                status.diagnostics.clear();
                status.resolved_recipes = graph.recipes;
                status.artifact_ref = graph.artifact_ref;
                status.content_digest = graph.content_digest;
                status.claim_ref = graph.claim_ref;
                status.target_output = graph.target_output;
                status.phase = Some(if status.artifact_ref.is_some() {
                    Phase::Succeeded
                } else {
                    Phase::Pending
                });
                status.conditions.push(condition(
                    "Resolved",
                    "True",
                    "RecipeInputsBound",
                    "Recipe, Task, variables and input bindings are fixed",
                ));
            }
            Err(error)
                if matches!(
                    error.reason.as_str(),
                    "WaitingForRecipe" | "WaitingForArtifact" | "WaitingForTask"
                ) =>
            {
                if status.phase == Some(Phase::Pending)
                    && status
                        .conditions
                        .iter()
                        .any(|c| c.reason == error.reason && c.message == error.message)
                    && status.diagnostics == error.diagnostics
                {
                    return Ok(());
                }
                status.phase = Some(Phase::Pending);
                status.diagnostics = error.diagnostics;
                status.conditions = vec![condition(
                    "Resolved",
                    "False",
                    &error.reason,
                    &error.message,
                )];
            }
            Err(error) => {
                status.diagnostics = error.diagnostics;
                fail(&mut status, &error.reason, &error.message);
            }
        }
        return patch_status(context, run, &status).await;
    }
    let namespace = request.namespace().ok_or("Request namespace missing")?;
    for recipe_index in 0..status.resolved_recipes.len() {
        if status.resolved_recipes[recipe_index].phase == Phase::Succeeded {
            continue;
        }
        if !bind_inputs(&mut status, recipe_index) {
            break;
        }
        let recipe = status.resolved_recipes[recipe_index].clone();
        for task_index in 0..recipe.tasks.len() {
            let task = &recipe.tasks[task_index];
            if task.phase == Phase::Succeeded {
                continue;
            }
            if task.phase == Phase::Running || task.phase == Phase::Verifying {
                let name = task
                    .job_ref
                    .as_ref()
                    .ok_or("running Task has no Job reference")?
                    .name
                    .clone();
                let jobs: Api<Job> = Api::namespaced(context.client.clone(), &namespace);
                let Some(job) = jobs.get_opt(&name).await.map_err(|e| e.to_string())? else {
                    fail(
                        &mut status,
                        "JobMissing",
                        &format!("Job {name} disappeared"),
                    );
                    return patch_status(context, run, &status).await;
                };
                if job.status.as_ref().and_then(|s| s.succeeded).unwrap_or(0) > 0 {
                    let result = match read_result(context, &namespace, &name).await? {
                        Some(result) => result,
                        None => {
                            if task.phase == Phase::Verifying {
                                return Ok(());
                            }
                            status.resolved_recipes[recipe_index].tasks[task_index].phase =
                                Phase::Verifying;
                            status.phase = Some(Phase::Verifying);
                            return patch_status(context, run, &status).await;
                        }
                    };
                    if let Err(error) =
                        validate_result(context, &namespace, &recipe, task_index, &result).await
                    {
                        fail(&mut status, "InvalidResult", &error);
                        return patch_status(context, run, &status).await;
                    }
                    let current = &mut status.resolved_recipes[recipe_index].tasks[task_index];
                    current.outputs = result
                        .outputs
                        .iter()
                        .map(|r| (r.name.clone(), r.artifact_ref.clone()))
                        .collect();
                    current.output_digests = result
                        .outputs
                        .iter()
                        .map(|r| (r.name.clone(), format!("sha256:{}", r.sha256)))
                        .collect();
                    current.claims = result
                        .outputs
                        .into_iter()
                        .map(|r| (r.name, r.claim_ref))
                        .collect();
                    current.phase = Phase::Succeeded;
                    status.phase = Some(Phase::Running);
                } else if job.status.as_ref().and_then(|s| s.failed).unwrap_or(0) > 0 {
                    let failure_class = classify_job_failure(context, &namespace, &name).await?;
                    let current = &mut status.resolved_recipes[recipe_index].tasks[task_index];
                    let retry = &current.definition.retry;
                    if current.attempt + 1 >= retry.max_attempts
                        || !retry.retry_on.contains(&failure_class)
                    {
                        current.phase = Phase::Failed;
                        status.resolved_recipes[recipe_index].phase = Phase::Failed;
                        fail(
                            &mut status,
                            "TaskFailed",
                            &format!("Job {name} failed ({failure_class:?})"),
                        );
                    } else {
                        current.phase = Phase::Pending;
                        current.attempt += 1;
                        current.job_ref = None;
                    }
                } else {
                    if task.phase == Phase::Running {
                        return Ok(());
                    }
                    status.resolved_recipes[recipe_index].tasks[task_index].phase = Phase::Running;
                }
                return patch_status(context, run, &status).await;
            }
            if task.phase == Phase::Failed {
                fail(
                    &mut status,
                    "TaskFailed",
                    &format!("Task {} failed", task.name),
                );
                return patch_status(context, run, &status).await;
            }
            let call = recipe
                .definition
                .tasks
                .iter()
                .find(|c| c.name == task.name)
                .ok_or("frozen Recipe missing Task")?;
            if call.inputs.artifacts.iter().any(|b| {
                b.from.task.as_ref().is_some_and(|dep| {
                    !recipe
                        .tasks
                        .iter()
                        .find(|s| &s.name == dep)
                        .is_some_and(|s| s.phase == Phase::Succeeded)
                })
            }) {
                continue;
            }
            let (manifest, injected_env) =
                manifest_for(context, run, &request, &status, recipe_index, task_index).await?;
            if request.spec.rerun == 0
                && let Some((outputs, claims, digests)) =
                    reuse_outputs(context, run, &namespace, &manifest).await?
            {
                let current = &mut status.resolved_recipes[recipe_index].tasks[task_index];
                current.outputs = outputs;
                current.claims = claims;
                current.output_digests = digests;
                current.build_key = Some(manifest.execution_build_key.clone());
                current.phase = Phase::Succeeded;
                status.resolved_recipes[recipe_index].phase = Phase::Running;
                status.phase = Some(Phase::Running);
                return patch_status(context, run, &status).await;
            }
            let name = job_name(
                run,
                &recipe.name,
                task,
                task.attempt,
                &context.settings,
                &manifest,
            )?;
            let (config, job) =
                resources(run, task, &manifest, &name, &context.settings, injected_env)?;
            create_if_missing(
                &Api::<ConfigMap>::namespaced(context.client.clone(), &namespace),
                &config,
            )
            .await?;
            create_job_if_missing(
                &Api::<Job>::namespaced(context.client.clone(), &namespace),
                &job,
            )
            .await?;
            let current = &mut status.resolved_recipes[recipe_index].tasks[task_index];
            current.phase = Phase::Running;
            current.build_key = Some(manifest.execution_build_key.clone());
            current.job_ref = Some(ArtifactReference { name });
            status.resolved_recipes[recipe_index].phase = Phase::Running;
            status.phase = Some(Phase::Running);
            return patch_status(context, run, &status).await;
        }
        if status.resolved_recipes[recipe_index]
            .tasks
            .iter()
            .all(|s| s.phase == Phase::Succeeded)
        {
            let is_root = recipe_index + 1 == status.resolved_recipes.len();
            let recipe = &mut status.resolved_recipes[recipe_index];
            for output in &recipe.definition.outputs.artifacts {
                let task = output.from.task.as_ref().ok_or("invalid Recipe output")?;
                let artifact = output
                    .from
                    .artifact
                    .as_ref()
                    .ok_or("invalid Recipe output")?;
                let reference = recipe
                    .tasks
                    .iter()
                    .find(|s| &s.name == task)
                    .and_then(|s| s.outputs.get(artifact))
                    .ok_or("missing committed output")?;
                recipe
                    .outputs
                    .artifacts
                    .get_mut(&output.name)
                    .ok_or("missing Recipe output")?
                    .artifact_ref = Some(reference.clone());
                recipe
                    .outputs
                    .artifacts
                    .get_mut(&output.name)
                    .unwrap()
                    .claim_ref = recipe
                    .tasks
                    .iter()
                    .find(|t| &t.name == task)
                    .and_then(|t| t.claims.get(artifact))
                    .cloned();
                recipe
                    .outputs
                    .artifacts
                    .get_mut(&output.name)
                    .unwrap()
                    .content_digest = recipe
                    .tasks
                    .iter()
                    .find(|t| &t.name == task)
                    .and_then(|t| t.output_digests.get(artifact))
                    .cloned();
            }
            recipe.phase = Phase::Succeeded;
            if is_root {
                let target = status
                    .target_output
                    .as_ref()
                    .ok_or("target output missing")?;
                status.artifact_ref = recipe
                    .outputs
                    .artifacts
                    .get(target)
                    .and_then(|o| o.artifact_ref.clone());
                status.claim_ref = recipe
                    .outputs
                    .artifacts
                    .get(target)
                    .and_then(|o| o.claim_ref.clone());
                status.content_digest = recipe
                    .outputs
                    .artifacts
                    .get(target)
                    .and_then(|o| o.content_digest.clone());
                status.phase = Some(Phase::Succeeded);
                status.conditions.push(condition(
                    "Completed",
                    "True",
                    "ArtifactCommitted",
                    "requested Artifact was committed",
                ));
            }
            return patch_status(context, run, &status).await;
        }
        break;
    }
    Ok(())
}

async fn reuse_outputs(
    context: &Context,
    run: &Run,
    namespace: &str,
    manifest: &JobManifest,
) -> Result<
    Option<(
        BTreeMap<String, ArtifactReference>,
        BTreeMap<String, ArtifactReference>,
        BTreeMap<String, String>,
    )>,
    String,
> {
    if manifest.outputs.is_empty() {
        return Ok(None);
    }
    let claims: Api<ArtifactClaim> = Api::namespaced(context.client.clone(), namespace);
    let artifacts: Api<Artifact> = Api::namespaced(context.client.clone(), namespace);
    let all = claims
        .list(&ListParams::default())
        .await
        .map_err(|e| e.to_string())?;
    let mut outputs = BTreeMap::new();
    let mut claim_refs = BTreeMap::new();
    let mut digests = BTreeMap::new();
    for output in &manifest.outputs {
        let mut candidates: Vec<_> = all
            .items
            .iter()
            .filter(|claim| {
                claim.metadata.deletion_timestamp.is_none()
                    && claim.metadata.labels.clone().unwrap_or_default() == output.labels
                    && claim.spec.build_key == output.build_key
                    && claim.spec.producer.recipe_ref.uid == output.producer.recipe_ref.uid
            })
            .collect();
        candidates.sort_by_key(|claim| claim.name_any());
        if candidates.len() > 1 {
            emit_warning(
                context,
                run,
                &format!(
                    "multiple cached Claims match {}: {}",
                    output.name,
                    candidates
                        .iter()
                        .map(|c| c.name_any())
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            )
            .await?;
        }
        let Some(claim) = candidates.first() else {
            return Ok(None);
        };
        let Some(artifact) = artifacts
            .get_opt(&claim.spec.artifact_ref.name)
            .await
            .map_err(|e| e.to_string())?
        else {
            return Ok(None);
        };
        if artifact.spec.kind != output.kind {
            return Ok(None);
        }
        outputs.insert(output.name.clone(), claim.spec.artifact_ref.clone());
        claim_refs.insert(
            output.name.clone(),
            ArtifactReference {
                name: claim.name_any(),
            },
        );
        digests.insert(output.name.clone(), artifact.spec.content_digest);
    }
    Ok(Some((outputs, claim_refs, digests)))
}

async fn classify_job_failure(
    context: &Context,
    namespace: &str,
    name: &str,
) -> Result<FailureClass, String> {
    let pods: Api<Pod> = Api::namespaced(context.client.clone(), namespace);
    let pods = pods
        .list(&ListParams::default().labels(&format!("batch.kubernetes.io/job-name={name}")))
        .await
        .map_err(|e| e.to_string())?;
    for pod in pods.items {
        if let Some(status) = pod.status {
            if status.container_statuses.as_ref().is_some_and(|items| {
                items.iter().any(|item| {
                    item.name == "task"
                        && item
                            .state
                            .as_ref()
                            .and_then(|s| s.terminated.as_ref())
                            .is_some_and(|t| t.exit_code != 0)
                })
            }) {
                return Ok(FailureClass::Task);
            }
            if status.container_statuses.as_ref().is_some_and(|items| {
                items.iter().any(|item| {
                    item.name == "fase-output"
                        && item
                            .state
                            .as_ref()
                            .and_then(|s| s.terminated.as_ref())
                            .is_some_and(|t| t.exit_code != 0)
                })
            }) {
                return Ok(FailureClass::Output);
            }
        }
    }
    Ok(FailureClass::Infrastructure)
}

fn bind_inputs(status: &mut RunStatus, index: usize) -> bool {
    let updates: Vec<_> = status.resolved_recipes[index]
        .inputs
        .artifacts
        .iter()
        .map(|input| {
            if let (Some(artifact), Some(claim), Some(digest)) =
                (&input.artifact_ref, &input.claim_ref, &input.content_digest)
            {
                return Some((artifact.clone(), claim.clone(), digest.clone()));
            }
            let source = input.from.resolved_recipe.as_ref()?;
            let output = input.from.artifact.as_ref()?;
            let output = status
                .resolved_recipes
                .iter()
                .find(|p| &p.name == source)?
                .outputs
                .artifacts
                .get(output)?;
            Some((
                output.artifact_ref.clone()?,
                output.claim_ref.clone()?,
                output.content_digest.clone()?,
            ))
        })
        .collect();
    if updates.iter().any(Option::is_none) {
        return false;
    }
    for (input, reference) in status.resolved_recipes[index]
        .inputs
        .artifacts
        .iter_mut()
        .zip(updates)
    {
        let (artifact, claim, digest) = reference.unwrap();
        input.artifact_ref = Some(artifact);
        input.claim_ref = Some(claim);
        input.content_digest = Some(digest);
    }
    true
}

async fn manifest_for(
    context: &Context,
    run: &Run,
    request: &Request,
    status: &RunStatus,
    pi: usize,
    si: usize,
) -> Result<(JobManifest, Vec<Value>), String> {
    let namespace = request.namespace().ok_or("namespace missing")?;
    let recipe = &status.resolved_recipes[pi];
    let task = &recipe.tasks[si];
    let call = recipe
        .definition
        .tasks
        .iter()
        .find(|c| c.name == task.name)
        .ok_or("missing Task call")?;
    let artifacts: Api<Artifact> = Api::namespaced(context.client.clone(), &namespace);
    let claims: Api<ArtifactClaim> = Api::namespaced(context.client.clone(), &namespace);
    let mut inputs = Vec::new();
    let mut input_digests = Vec::new();
    let mut env = Vec::new();
    let mut env_names: BTreeSet<String> = ["FASE_INPUT_ROOT", "FASE_OUTPUT_ROOT", "FASE_RUN_ROOT"]
        .into_iter()
        .map(str::to_string)
        .collect();
    env_names.extend(task.variables.keys().cloned());
    env_names.extend(task.definition.env.iter().map(|e| e.name.clone()));
    for port in &task.definition.inputs.artifacts {
        let binding = call
            .inputs
            .artifacts
            .iter()
            .find(|b| b.name == port.name)
            .ok_or("missing Task input")?;
        let reference = match (
            &binding.from.recipe_input,
            &binding.from.task,
            &binding.from.artifact,
        ) {
            (Some(input), None, None) => recipe
                .inputs
                .artifacts
                .iter()
                .find(|i| &i.name == input)
                .and_then(|i| i.artifact_ref.clone()),
            (None, Some(source), Some(output)) => recipe
                .tasks
                .iter()
                .find(|s| &s.name == source)
                .and_then(|s| s.outputs.get(output).cloned()),
            _ => return Err("invalid Task input origin".into()),
        }
        .ok_or("input Artifact is not committed")?;
        let artifact = artifacts
            .get(&reference.name)
            .await
            .map_err(|e| e.to_string())?;
        let claim_ref = match (
            &binding.from.recipe_input,
            &binding.from.task,
            &binding.from.artifact,
        ) {
            (Some(input), None, None) => recipe
                .inputs
                .artifacts
                .iter()
                .find(|i| &i.name == input)
                .and_then(|i| i.claim_ref.as_ref()),
            (None, Some(source), Some(output)) => recipe
                .tasks
                .iter()
                .find(|t| &t.name == source)
                .and_then(|t| t.claims.get(output)),
            _ => None,
        }
        .ok_or("input ArtifactClaim is not committed")?;
        let claim = claims
            .get(&claim_ref.name)
            .await
            .map_err(|e| e.to_string())?;
        if claim.spec.artifact_ref.name != reference.name {
            return Err("input Claim changed Artifact reference".into());
        }
        let labels = claim.metadata.labels.clone().unwrap_or_default();
        if let Some(expected) = recipe
            .inputs
            .artifacts
            .iter()
            .find(|i| binding.from.recipe_input.as_ref() == Some(&i.name))
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
            kind: port.kind,
            artifact_name: reference.name,
            artifact_spec: artifact.spec,
        });
        input_digests.push((
            port.name.clone(),
            inputs.last().unwrap().artifact_spec.content_digest.clone(),
            labels,
        ));
    }
    let build_key = execution_build_key(
        &recipe.definition,
        &task.definition,
        &task.variables,
        &input_digests,
        &context.settings,
    )?;
    let producer = ProducerReference {
        recipe_ref: ObjectReference {
            api_version: "skyw.top/v1beta1".into(),
            kind: "Recipe".into(),
            name: recipe.recipe_ref.name.clone(),
            uid: recipe.recipe_ref.uid.clone(),
            generation: recipe.recipe_ref.generation,
        },
        run_ref: ObjectReference {
            api_version: "skyw.top/v1beta1".into(),
            kind: "Run".into(),
            name: run.name_any(),
            uid: run.metadata.uid.clone().ok_or("Run UID missing")?,
            generation: run.metadata.generation.unwrap_or(1),
        },
    };
    let outputs = task
        .definition
        .outputs
        .artifacts
        .iter()
        .filter(|port| call.outputs.iter().any(|o| o.name == port.name))
        .map(|port| {
            let final_output = recipe.definition.outputs.artifacts.iter().find(|o| {
                o.from.task.as_deref() == Some(&task.name)
                    && o.from.artifact.as_deref() == Some(&port.name)
            });
            let labels = match final_output {
                Some(o) => evaluate_labels(recipe, &o.labels)?,
                None => Labels::new(),
            };
            Ok(OutputItem {
                name: port.name.clone(),
                path: port.path.clone(),
                kind: port.kind,
                labels,
                build_key: output_build_key(&build_key, &port.name),
                producer: producer.clone(),
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok((
        JobManifest {
            inputs,
            outputs,
            execution_build_key: build_key,
            namespace,
            run_name: run.name_any(),
            run_uid: run.metadata.uid.clone().ok_or("Run UID missing")?,
        },
        env,
    ))
}

fn execution_build_key(
    recipe: &RecipeSpec,
    task: &TaskSpec,
    variables: &Variables,
    inputs: &[(String, String, Labels)],
    settings: &JobSettings,
) -> Result<String, String> {
    let mut execution_recipe = recipe.clone();
    for output in &mut execution_recipe.outputs.artifacts {
        output.labels.clear();
    }
    let bytes = serde_json::to_vec(&json!([
        execution_recipe,
        task,
        variables,
        inputs,
        settings.helper_image,
        settings.output_service_account,
        settings.task_timeout_seconds,
    ]))
    .map_err(|e| e.to_string())?;
    Ok(format!("sha256:{}", fase_api::sha256_hex(&bytes)))
}

fn output_build_key(execution_key: &str, port: &str) -> String {
    format!(
        "sha256:{}",
        fase_api::sha256_hex(
            &serde_json::to_vec(&(execution_key, port)).expect("strings serialize to JSON")
        )
    )
}

fn evaluate_labels(
    recipe: &ResolvedRecipeStatus,
    bindings: &BTreeMap<String, LabelBinding>,
) -> Result<Labels, String> {
    let mut labels = Labels::new();
    for (key, binding) in bindings {
        let value = match binding {
            LabelBinding::Literal(v) => v.value.clone(),
            LabelBinding::Bound(source) => match (&source.from_variable, &source.from_input) {
                (Some(v), None) => recipe
                    .variables
                    .get(v)
                    .ok_or("unknown output variable")?
                    .clone(),
                (None, Some(i)) => recipe
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
        return Err("invalid Recipe output labels".into());
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
    recipe: &ResolvedRecipeStatus,
    si: usize,
    result: &OutputResult,
) -> Result<(), String> {
    let task = &recipe.tasks[si];
    let call = recipe
        .definition
        .tasks
        .iter()
        .find(|c| c.name == task.name)
        .ok_or("missing Task call")?;
    if call.outputs.len() != result.outputs.len() {
        return Err("output count mismatch".into());
    }
    let artifacts: Api<Artifact> = Api::namespaced(context.client.clone(), namespace);
    let claims: Api<ArtifactClaim> = Api::namespaced(context.client.clone(), namespace);
    for receipt in &result.outputs {
        let port = task
            .definition
            .outputs
            .artifacts
            .iter()
            .find(|p| p.name == receipt.name)
            .ok_or("unknown output receipt")?;
        if !call.outputs.iter().any(|o| o.name == port.name)
            || receipt.kind != port.kind
            || receipt.artifact_ref.name
                != crate::storage::artifact_id(&receipt.sha256, receipt.kind)
            || receipt.size < 0
        {
            return Err("invalid output receipt".into());
        }
        let artifact = artifacts
            .get(&receipt.artifact_ref.name)
            .await
            .map_err(|e| e.to_string())?;
        if artifact.spec.kind != port.kind
            || artifact.spec.content_digest != format!("sha256:{}", receipt.sha256)
            || artifact.spec.size_bytes != receipt.size
        {
            return Err("output type mismatch".into());
        }
        let claim = claims
            .get(&receipt.claim_ref.name)
            .await
            .map_err(|e| e.to_string())?;
        if claim.spec.artifact_ref.name != receipt.artifact_ref.name {
            return Err("output Claim references a different Artifact".into());
        }
        for output in &recipe.definition.outputs.artifacts {
            if output.from.task.as_deref() == Some(&task.name)
                && output.from.artifact.as_deref() == Some(&port.name)
            {
                let labels = evaluate_labels(recipe, &output.labels)?;
                if claim.metadata.labels.clone().unwrap_or_default() != labels {
                    return Err("output labels violate Recipe contract".into());
                }
            }
        }
    }
    Ok(())
}
fn fail(status: &mut RunStatus, reason: &str, message: &str) {
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
async fn emit_warning(context: &Context, run: &Run, message: &str) -> Result<(), String> {
    let namespace = run.namespace().ok_or("Run namespace missing")?;
    let event: k8s_openapi::api::core::v1::Event = serde_json::from_value(json!({
        "apiVersion":"v1", "kind":"Event",
        "metadata":{"generateName":"fase-candidates-","namespace":namespace},
        "involvedObject":{"apiVersion":"skyw.top/v1beta1","kind":"Run","name":run.name_any(),
            "namespace":namespace,"uid":run.metadata.uid},
        "type":"Warning","reason":"MultipleCandidates","message":message,
        "source":{"component":"fase-controller"}
    }))
    .map_err(|e| e.to_string())?;
    Api::<k8s_openapi::api::core::v1::Event>::namespaced(context.client.clone(), &namespace)
        .create(&PostParams::default(), &event)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}
async fn patch_status(context: &Context, run: &Run, status: &RunStatus) -> Result<(), String> {
    let api: Api<Run> = Api::namespaced(
        context.client.clone(),
        &run.namespace().ok_or("namespace missing")?,
    );
    api.patch_status(
        &run.name_any(),
        &PatchParams::default(),
        &Patch::Merge(
            json!({"metadata":{"resourceVersion":run.resource_version()},"status":status}),
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

async fn create_job_if_missing(api: &Api<Job>, desired: &Job) -> Result<(), String> {
    match api.create(&PostParams::default(), desired).await {
        Ok(_) => Ok(()),
        Err(kube::Error::Api(error)) if error.code == 409 => {
            let name = desired.name_any();
            let existing = api.get(&name).await.map_err(|error| error.to_string())?;
            let key = |job: &Job| {
                job.metadata
                    .annotations
                    .as_ref()
                    .and_then(|annotations| annotations.get("skyw.top/job-key"))
                    .cloned()
            };
            let owner = |job: &Job| {
                job.metadata
                    .owner_references
                    .as_ref()
                    .and_then(|owners| {
                        owners
                            .iter()
                            .find(|owner| owner.kind == "Run" && owner.controller == Some(true))
                    })
                    .map(|owner| owner.uid.clone())
            };
            if key(&existing).is_some()
                && key(&existing) == key(desired)
                && owner(&existing).is_some()
                && owner(&existing) == owner(desired)
            {
                Ok(())
            } else {
                Err(format!("existing Job {name} conflicts"))
            }
        }
        Err(error) => Err(error.to_string()),
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[test]
    fn output_labels_do_not_change_build_key_but_task_script_does() {
        let docs: Vec<Value> =
            serde_yml::Deserializer::from_str(include_str!("../../examples/hello.yaml"))
                .map(|doc| Value::deserialize(doc).unwrap())
                .collect();
        let task: Task =
            serde_json::from_value(docs.iter().find(|v| v["kind"] == "Task").unwrap().clone())
                .unwrap();
        let recipe: Recipe =
            serde_json::from_value(docs.iter().find(|v| v["kind"] == "Recipe").unwrap().clone())
                .unwrap();
        let settings = JobSettings {
            helper_image: "helper:dev".into(),
            output_service_account: "fase-output".into(),
            s3_bucket: "fase".into(),
            s3_endpoint: "https://silo".into(),
            s3_region: "us-east-1".into(),
            s3_prefix: String::new(),
            allow_insecure_s3: false,
            s3_anonymous: false,
            max_artifact_bytes: 1024,
            task_timeout_seconds: 3600,
            read_secret: "read".into(),
            write_secret: "write".into(),
        };
        let variables = Variables::from([("version".into(), "1.0".into())]);
        let first =
            execution_build_key(&recipe.spec, &task.spec, &variables, &[], &settings).unwrap();
        let mut relabeled = recipe.spec.clone();
        relabeled.outputs.artifacts[0].labels.insert(
            "release".into(),
            LabelBinding::Literal(fase_api::LabelLiteral {
                value: "stable".into(),
            }),
        );
        assert_eq!(
            first,
            execution_build_key(&relabeled, &task.spec, &variables, &[], &settings).unwrap()
        );
        let mut changed_task = task.spec.clone();
        changed_task.script.push_str("\necho changed\n");
        assert_ne!(
            first,
            execution_build_key(&recipe.spec, &changed_task, &variables, &[], &settings).unwrap()
        );

        let first_input = vec![(
            "source".into(),
            "sha256:same".into(),
            Labels::from([("version".into(), "1".into())]),
        )];
        let second_input = vec![(
            "source".into(),
            "sha256:same".into(),
            Labels::from([("version".into(), "2".into())]),
        )];
        assert_ne!(
            execution_build_key(
                &recipe.spec,
                &task.spec,
                &variables,
                &first_input,
                &settings
            )
            .unwrap(),
            execution_build_key(
                &recipe.spec,
                &task.spec,
                &variables,
                &second_input,
                &settings
            )
            .unwrap(),
        );
        assert_ne!(
            output_build_key(&first, "source"),
            output_build_key(&first, "package")
        );
    }
}
