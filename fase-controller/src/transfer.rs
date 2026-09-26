use crate::storage::{Store, artifact_id, checked_output_path, collect, materialize};
use fase_api::sha256_hex;
use fase_api::{
    Artifact, ArtifactClaim, ArtifactClaimSpec, ArtifactKind, ArtifactReference, ArtifactSpec,
    Labels, ProducerReference, StorageReference,
};
use kube::{Api, Client, api::PostParams};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::PathBuf, time::Duration};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InputItem {
    pub path: String,
    pub kind: ArtifactKind,
    pub artifact_name: String,
    pub artifact_spec: ArtifactSpec,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OutputItem {
    pub name: String,
    pub path: String,
    pub kind: ArtifactKind,
    pub labels: Labels,
    pub build_key: String,
    pub producer: ProducerReference,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct JobManifest {
    pub inputs: Vec<InputItem>,
    pub outputs: Vec<OutputItem>,
    pub execution_build_key: String,
    pub namespace: String,
    pub run_name: String,
    pub run_uid: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OutputReceipt {
    pub name: String,
    pub artifact_ref: ArtifactReference,
    pub claim_ref: ArtifactReference,
    pub sha256: String,
    pub size: i64,
    pub kind: ArtifactKind,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct OutputResult {
    pub outputs: Vec<OutputReceipt>,
}

pub async fn run_input() -> Result<(), String> {
    let input_root = root("FASE_INPUT_ROOT", "/in");
    let manifest = read_manifest()?;
    let store = Store::from_env()?;
    let max_bytes = max_artifact_bytes()?;
    for input in &manifest.inputs {
        let bytes = store
            .get(
                &input.artifact_name,
                &input.artifact_spec,
                input.kind,
                max_bytes,
            )
            .await?;
        if bytes.len() as u64 > max_bytes {
            return Err(format!(
                "input {} exceeds FASE_MAX_ARTIFACT_BYTES",
                input.artifact_name
            ));
        }
        materialize(&input_root, &input.path, input.kind, &bytes)?;
    }
    Ok(())
}

pub async fn run_output() -> Result<(), String> {
    let manifest = read_manifest()?;
    let client = Client::try_default()
        .await
        .map_err(|error| error.to_string())?;
    let pod_name =
        std::env::var("FASE_POD_NAME").map_err(|_| "FASE_POD_NAME is required".to_owned())?;
    let exit_code = wait_for_task(&client, &manifest.namespace, &pod_name).await?;
    if exit_code != 0 {
        return Err(format!("task container exited with {exit_code}"));
    }

    let output_root = root("FASE_OUTPUT_ROOT", "/out");
    let store = Store::from_env()?;
    let artifacts: Api<Artifact> = Api::namespaced(client.clone(), &manifest.namespace);
    let claims: Api<ArtifactClaim> = Api::namespaced(client.clone(), &manifest.namespace);
    let max_bytes = max_artifact_bytes()?;
    let mut prepared = Vec::new();
    for output in &manifest.outputs {
        let path = checked_output_path(&output_root, &output.path)?;
        let bytes = collect(&path, output.kind)?;
        if bytes.len() as u64 > max_bytes {
            return Err(format!(
                "output {} exceeds FASE_MAX_ARTIFACT_BYTES",
                output.name
            ));
        }
        let sha256 = sha256_hex(&bytes);
        let name = artifact_id(&sha256, output.kind);
        let spec = ArtifactSpec {
            content_digest: format!("sha256:{sha256}"),
            size_bytes: bytes.len() as i64,
            kind: output.kind,
            storage_ref: StorageReference {
                key: format!("objects/{name}"),
            },
        };
        prepared.push((output.clone(), name, spec, bytes, sha256));
    }
    let max_bytes = max_artifact_bytes()?;
    for (_, name, _, bytes, _) in &prepared {
        store.put(name, bytes.clone(), max_bytes).await?;
    }

    let mut receipts = Vec::new();
    for (output, name, spec, bytes, sha256) in prepared {
        let mut artifact = Artifact::new(&name, spec);
        artifact.metadata.namespace = Some(manifest.namespace.clone());
        match artifacts.create(&PostParams::default(), &artifact).await {
            Ok(_) => {}
            Err(kube::Error::Api(error)) if error.code == 409 => {
                // The identity is content-derived. Labels and the logical
                // name are presentation metadata, so a second publisher with
                // different labels may safely reuse the existing object.
                let existing = artifacts
                    .get(&name)
                    .await
                    .map_err(|error| error.to_string())?;
                if existing.spec.content_digest != artifact.spec.content_digest
                    || existing.spec.size_bytes != artifact.spec.size_bytes
                    || existing.spec.kind != artifact.spec.kind
                    || existing.spec.storage_ref.key != artifact.spec.storage_ref.key
                {
                    return Err(format!("Artifact {name} has conflicting content metadata"));
                }
            }
            Err(error) => return Err(error.to_string()),
        }
        let identity =
            serde_json::to_vec(&(name.as_str(), &output.labels)).map_err(|e| e.to_string())?;
        let claim_name = format!("claim-{}", fase_api::sha256_hex(&identity));
        let mut claim = ArtifactClaim::new(
            &claim_name,
            ArtifactClaimSpec {
                artifact_ref: ArtifactReference { name: name.clone() },
                build_key: output.build_key,
                producer: output.producer,
            },
        );
        claim.metadata.namespace = Some(manifest.namespace.clone());
        claim.metadata.labels = Some(output.labels);
        match claims.create(&PostParams::default(), &claim).await {
            Ok(_) => {}
            Err(kube::Error::Api(error)) if error.code == 409 => {
                let existing = claims.get(&claim_name).await.map_err(|e| e.to_string())?;
                if existing.spec.artifact_ref.name != name
                    || existing.metadata.labels.unwrap_or_default()
                        != claim.metadata.labels.unwrap_or_default()
                {
                    return Err(format!("ArtifactClaim {claim_name} conflicts"));
                }
            }
            Err(error) => return Err(error.to_string()),
        }
        receipts.push(OutputReceipt {
            name: output.name,
            artifact_ref: ArtifactReference { name },
            claim_ref: ArtifactReference { name: claim_name },
            size: bytes.len() as i64,
            sha256,
            kind: output.kind,
        });
    }

    let result = OutputResult { outputs: receipts };
    let body = serde_json::to_string(&result).map_err(|error| error.to_string())?;
    let job_name =
        std::env::var("FASE_JOB_NAME").map_err(|_| "FASE_JOB_NAME is required".to_owned())?;
    let result_name = format!("{job_name}-result");
    let mut result_config = k8s_openapi::api::core::v1::ConfigMap {
        data: Some(BTreeMap::from([("result.json".to_owned(), body.clone())])),
        ..Default::default()
    };
    result_config.metadata.name = Some(result_name.clone());
    result_config.metadata.namespace = Some(manifest.namespace.clone());
    result_config.immutable = Some(true);
    result_config.metadata.owner_references = Some(vec![
        k8s_openapi::apimachinery::pkg::apis::meta::v1::OwnerReference {
            api_version: "skyw.top/v1beta1".into(),
            kind: "Run".into(),
            name: manifest.run_name.clone(),
            uid: manifest.run_uid.clone(),
            controller: Some(false),
            block_owner_deletion: Some(false),
        },
    ]);
    let configs: Api<k8s_openapi::api::core::v1::ConfigMap> =
        Api::namespaced(client, &manifest.namespace);
    match configs.create(&PostParams::default(), &result_config).await {
        Ok(_) => {}
        Err(kube::Error::Api(error)) if error.code == 409 => {
            let existing = configs
                .get(&result_name)
                .await
                .map_err(|error| error.to_string())?;
            if existing
                .data
                .as_ref()
                .and_then(|data| data.get("result.json"))
                != Some(&body)
            {
                return Err(format!("result ConfigMap {result_name} conflicts"));
            }
        }
        Err(error) => return Err(error.to_string()),
    }
    println!("{body}");
    Ok(())
}

async fn wait_for_task(client: &Client, namespace: &str, pod_name: &str) -> Result<i32, String> {
    let pods: Api<k8s_openapi::api::core::v1::Pod> = Api::namespaced(client.clone(), namespace);
    let timeout = task_timeout_seconds()?;
    for _ in 0..timeout.saturating_mul(2) {
        let pod = pods
            .get(pod_name)
            .await
            .map_err(|error| error.to_string())?;
        if let Some(status) = pod
            .status
            .as_ref()
            .and_then(|status| status.container_statuses.as_ref())
            .and_then(|statuses| statuses.iter().find(|status| status.name == "task"))
            .and_then(|status| status.state.as_ref())
            .and_then(|state| state.terminated.as_ref())
        {
            return Ok(status.exit_code);
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    Err("timed out waiting for the task container".to_owned())
}

fn read_manifest() -> Result<JobManifest, String> {
    let bytes = std::fs::read(root("FASE_RUN_ROOT", "/run/fase").join("config/manifest.json"))
        .map_err(|error| error.to_string())?;
    serde_json::from_slice(&bytes).map_err(|error| error.to_string())
}

fn task_timeout_seconds() -> Result<u64, String> {
    let value = std::env::var("FASE_STEP_TIMEOUT_SECONDS")
        .unwrap_or_else(|_| "3600".into())
        .parse::<u64>()
        .map_err(|_| "FASE_STEP_TIMEOUT_SECONDS must be a positive integer".to_owned())?;
    if value == 0 || value > 7 * 24 * 60 * 60 {
        return Err("FASE_STEP_TIMEOUT_SECONDS must be between 1 and 604800".into());
    }
    Ok(value)
}

fn max_artifact_bytes() -> Result<u64, String> {
    let value = std::env::var("FASE_MAX_ARTIFACT_BYTES")
        .unwrap_or_else(|_| "536870912".into())
        .parse::<u64>()
        .map_err(|_| "FASE_MAX_ARTIFACT_BYTES must be a positive integer".to_owned())?;
    if value == 0 || value > 8 * 1024 * 1024 * 1024 {
        return Err("FASE_MAX_ARTIFACT_BYTES must be between 1 and 8589934592".into());
    }
    Ok(value)
}

fn root(name: &str, default: &str) -> PathBuf {
    std::env::var_os(name)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(default))
}
