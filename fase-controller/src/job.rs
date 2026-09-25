use crate::transfer::JobManifest;
use fase_api::sha256_hex;
use fase_api::{Request, StepStatus};
use k8s_openapi::api::{batch::v1::Job, core::v1::ConfigMap};
use kube::ResourceExt;
use serde_json::json;
use std::collections::BTreeMap;

#[derive(Clone)]
pub struct JobSettings {
    pub helper_image: String,
    pub output_service_account: String,
    pub s3_bucket: String,
    pub s3_endpoint: String,
    pub s3_region: String,
    pub s3_prefix: String,
    pub allow_insecure_s3: bool,
    pub s3_anonymous: bool,
    pub max_artifact_bytes: u64,
    pub step_timeout_seconds: u64,
    pub read_secret: String,
    pub write_secret: String,
}

impl JobSettings {
    pub fn from_env() -> Result<Self, String> {
        fn required(name: &str) -> Result<String, String> {
            std::env::var(name).map_err(|_| format!("{name} is required"))
        }
        let s3_endpoint = required("FASE_S3_ENDPOINT")?;
        let allow_insecure_s3 = std::env::var("FASE_ALLOW_INSECURE_S3")
            .map(|value| value == "true")
            .unwrap_or(false);
        if !s3_endpoint.starts_with("https://") && !s3_endpoint.starts_with("http://") {
            return Err("FASE_S3_ENDPOINT must use http or https".into());
        }
        if s3_endpoint.starts_with("http://") && !allow_insecure_s3 {
            return Err(
                "FASE_S3_ENDPOINT must use https unless FASE_ALLOW_INSECURE_S3=true".into(),
            );
        }
        let s3_anonymous = std::env::var("FASE_S3_ANONYMOUS")
            .map(|value| value == "true")
            .unwrap_or(false);
        let max_artifact_bytes = std::env::var("FASE_MAX_ARTIFACT_BYTES")
            .unwrap_or_else(|_| "536870912".into())
            .parse::<u64>()
            .map_err(|_| "FASE_MAX_ARTIFACT_BYTES must be a positive integer".to_owned())?;
        if max_artifact_bytes == 0 || max_artifact_bytes > 8 * 1024 * 1024 * 1024 {
            return Err("FASE_MAX_ARTIFACT_BYTES must be between 1 and 8589934592".into());
        }
        let step_timeout_seconds = std::env::var("FASE_STEP_TIMEOUT_SECONDS")
            .unwrap_or_else(|_| "3600".into())
            .parse::<u64>()
            .map_err(|_| "FASE_STEP_TIMEOUT_SECONDS must be a positive integer".to_owned())?;
        if step_timeout_seconds == 0 || step_timeout_seconds > 7 * 24 * 60 * 60 {
            return Err("FASE_STEP_TIMEOUT_SECONDS must be between 1 and 604800".into());
        }
        Ok(Self {
            helper_image: required("FASE_HELPER_IMAGE")?,
            output_service_account: required("FASE_OUTPUT_SERVICE_ACCOUNT")?,
            s3_bucket: required("FASE_S3_BUCKET")?,
            s3_endpoint,
            s3_region: std::env::var("FASE_S3_REGION").unwrap_or_else(|_| "us-east-1".into()),
            s3_prefix: std::env::var("FASE_S3_PREFIX").unwrap_or_default(),
            allow_insecure_s3,
            s3_anonymous,
            max_artifact_bytes,
            step_timeout_seconds,
            read_secret: required("FASE_S3_READ_SECRET")?,
            write_secret: required("FASE_S3_WRITE_SECRET")?,
        })
    }
}

pub fn job_name(
    request: &Request,
    plan_name: &str,
    step: &StepStatus,
    attempt: i32,
    settings: &JobSettings,
    manifest: &JobManifest,
) -> Result<String, String> {
    let uid = request
        .metadata
        .uid
        .as_deref()
        .ok_or("Request UID is required")?;
    let identity = json!([
        uid,
        plan_name,
        &step.step_ref,
        &step.definition,
        &step.variables,
        &step.name,
        attempt,
        &settings.helper_image,
        &settings.output_service_account,
        &settings.s3_bucket,
        &settings.s3_endpoint,
        &settings.s3_region,
        &settings.s3_prefix,
        settings.allow_insecure_s3,
        settings.s3_anonymous,
        settings.max_artifact_bytes,
        settings.step_timeout_seconds,
        &settings.read_secret,
        &settings.write_secret,
        manifest,
    ]);
    let bytes = serde_json::to_vec(&identity).map_err(|error| error.to_string())?;
    let suffix = sha256_hex(&bytes);
    let raw = format!("{}-{}", request.name_any(), step.name);
    let prefix = dns_label(&raw);
    let prefix = prefix
        .chars()
        .take(40)
        .collect::<String>()
        .trim_end_matches('-')
        .to_string();
    Ok(format!("{prefix}-a{attempt}-{}", &suffix[..12]))
}

pub fn resources(
    request: &Request,
    step: &StepStatus,
    manifest: &JobManifest,
    name: &str,
    settings: &JobSettings,
    injected_env: Vec<serde_json::Value>,
) -> Result<(ConfigMap, Job), String> {
    let namespace = request.namespace().ok_or("Request namespace is required")?;
    let uid = request
        .metadata
        .uid
        .as_deref()
        .ok_or("Request UID is required")?;
    let owner = json!([{"apiVersion":"fase.io/v1alpha1","kind":"Request","name":request.name_any(),"uid":uid,"controller":true,"blockOwnerDeletion":false}]);
    let config_name = format!("{name}-cfg");
    let config: ConfigMap = serde_json::from_value(json!({
        "apiVersion":"v1","kind":"ConfigMap",
        "metadata":{"name":config_name,"namespace":namespace,"ownerReferences":owner},
        "immutable":true,
        "data":{
            "manifest.json":serde_json::to_string(manifest).map_err(|error| error.to_string())?,
            "script":step.definition.script,
            "entrypoint.sh":"#!/bin/sh\nexec \"$@\" /run/fase/config/script\n"
        }
    }))
    .map_err(|error| error.to_string())?;

    let storage_env = json!([
        {"name":"FASE_S3_BUCKET","value":settings.s3_bucket},
        {"name":"FASE_S3_ENDPOINT","value":settings.s3_endpoint},
        {"name":"FASE_S3_REGION","value":settings.s3_region},
        {"name":"FASE_S3_PREFIX","value":settings.s3_prefix},
        {"name":"FASE_ALLOW_INSECURE_S3","value":settings.allow_insecure_s3.to_string()},
        {"name":"FASE_S3_ANONYMOUS","value":settings.s3_anonymous.to_string()},
        {"name":"FASE_MAX_ARTIFACT_BYTES","value":settings.max_artifact_bytes.to_string()},
        {"name":"FASE_STEP_TIMEOUT_SECONDS","value":settings.step_timeout_seconds.to_string()},
        {"name":"FASE_INPUT_ROOT","value":"/in"},
        {"name":"FASE_OUTPUT_ROOT","value":"/out"},
        {"name":"FASE_RUN_ROOT","value":"/run/fase"}
    ]);
    let helper_env = |secret: &str| {
        let mut env = storage_env.as_array().cloned().unwrap_or_default();
        env.extend([
            json!({"name":"FASE_S3_ACCESS_KEY_ID","valueFrom":{"secretKeyRef":{"name":secret,"key":"accessKeyId"}}}),
            json!({"name":"FASE_S3_SECRET_ACCESS_KEY","valueFrom":{"secretKeyRef":{"name":secret,"key":"secretAccessKey"}}}),
            json!({"name":"FASE_S3_SESSION_TOKEN","valueFrom":{"secretKeyRef":{"name":secret,"key":"sessionToken","optional":true}}}),
        ]);
        env
    };
    let mut output_env = helper_env(&settings.write_secret);
    output_env.extend([
        json!({"name":"KUBERNETES_SERVICE_HOST","value":"kubernetes.default.svc"}),
        json!({"name":"KUBERNETES_SERVICE_PORT","value":"443"}),
        json!({"name":"FASE_POD_NAME","valueFrom":{"fieldRef":{"fieldPath":"metadata.name"}}}),
        json!({"name":"FASE_JOB_NAME","value":name}),
    ]);
    let mut main_env = vec![
        json!({"name":"FASE_INPUT_ROOT","value":"/in"}),
        json!({"name":"FASE_OUTPUT_ROOT","value":"/out"}),
        json!({"name":"FASE_RUN_ROOT","value":"/run/fase"}),
    ];
    for (name, value) in &step.variables {
        main_env.push(json!({"name":name,"value":value}));
    }
    for entry in &step.definition.env {
        main_env.push(serde_json::to_value(entry).map_err(|e| e.to_string())?);
    }
    main_env.extend(injected_env);
    let restricted = json!({"allowPrivilegeEscalation":false,"readOnlyRootFilesystem":true,"runAsNonRoot":true,"runAsUser":1000,"seccompProfile":{"type":"RuntimeDefault"},"capabilities":{"drop":["ALL"]}});
    let resources = json!({
        "requests":{"cpu":"100m","memory":"128Mi","ephemeral-storage":"1Gi"},
        "limits":{"cpu":"1","memory":"1Gi","ephemeral-storage":"2Gi"}
    });
    let shell = step
        .definition
        .command
        .first()
        .ok_or("Step command is required")?;
    let main_command = vec![shell.as_str(), "/run/fase/config/entrypoint.sh"];
    let main_args = step.definition.command.clone();
    let request_label = label_value(&request.name_any());
    let labels = BTreeMap::from([
        ("app.kubernetes.io/name".to_string(), "fase".to_string()),
        ("fase.io/request".to_string(), request_label),
        ("fase.io/step".to_string(), label_value(&step.name)),
    ]);
    let mut job: Job = serde_json::from_value(json!({
        "apiVersion":"batch/v1","kind":"Job",
        "metadata":{"name":name,"namespace":namespace,"labels":labels,"ownerReferences":owner},
        "spec":{"backoffLimit":0,"activeDeadlineSeconds":settings.step_timeout_seconds + 600,"ttlSecondsAfterFinished":86400,
            "template":{"metadata":{"labels":labels},"spec":{
                "restartPolicy":"Never","automountServiceAccountToken":false,
                "serviceAccountName":settings.output_service_account,
                "enableServiceLinks":false,
                "securityContext":{"fsGroup":1000},
                "volumes":[
                    {"name":"inputs","emptyDir":{"sizeLimit":"2Gi"}},
                    {"name":"outputs","emptyDir":{"sizeLimit":"2Gi"}},
                    {"name":"input-temp","emptyDir":{"sizeLimit":"2Gi"}},
                    {"name":"step-temp","emptyDir":{"sizeLimit":"2Gi"}},
                    {"name":"output-temp","emptyDir":{"sizeLimit":"2Gi"}},
                    {"name":"config","configMap":{"name":config_name}},
                    {"name":"output-token","projected":{"sources":[{"serviceAccountToken":{"path":"token","expirationSeconds":7200}},{"configMap":{"name":"kube-root-ca.crt","items":[{"key":"ca.crt","path":"ca.crt"}]}},{"downwardAPI":{"items":[{"path":"namespace","fieldRef":{"fieldPath":"metadata.namespace"}}]}}]}}
                ],
                "initContainers":[{
                    "name":"fase-input","image":settings.helper_image,
                    "command":["/usr/local/bin/fase-controller","input"],
                    "env":helper_env(&settings.read_secret),
                    "securityContext":restricted,"resources":resources.clone(),
                    "volumeMounts":[
                        {"name":"inputs","mountPath":"/in"},
                        {"name":"config","mountPath":"/run/fase/config","readOnly":true},
                        {"name":"input-temp","mountPath":"/tmp"}
                    ]
                }],
                "containers":[
                    {
                        "name":"step","image":step.definition.image,
                        "command":main_command,"args":main_args,
                        "env":main_env,"securityContext":restricted,"resources":resources.clone(),
                        "volumeMounts":[
                            {"name":"inputs","mountPath":"/in","readOnly":true},
                            {"name":"outputs","mountPath":"/out"},
                                {"name":"config","mountPath":"/run/fase/config","readOnly":true},
                            {"name":"step-temp","mountPath":"/tmp"}
                        ]
                    },
                    {
                        "name":"fase-output","image":settings.helper_image,
                        "command":["/usr/local/bin/fase-controller","output"],
                        "env":output_env,
                        "securityContext":restricted,"resources":resources.clone(),
                        "volumeMounts":[
                            {"name":"outputs","mountPath":"/out","readOnly":true},
                                {"name":"config","mountPath":"/run/fase/config","readOnly":true},
                            {"name":"output-temp","mountPath":"/tmp"},
                            {"name":"output-token","mountPath":"/var/run/secrets/kubernetes.io/serviceaccount","readOnly":true}
                        ]
                    }
                ]
            }}
        }
    })).map_err(|error| error.to_string())?;
    let pod = job.spec.as_mut().unwrap().template.spec.as_mut().unwrap();
    pod.image_pull_secrets = Some(
        step.definition
            .image_pull_secrets
            .iter()
            .map(|r| k8s_openapi::api::core::v1::LocalObjectReference {
                name: r.name.clone(),
            })
            .collect(),
    );
    pod.node_selector = Some(step.definition.node_selector.clone());
    pod.tolerations = Some(
        step.definition
            .tolerations
            .iter()
            .map(|v| serde_json::from_value(v.0.clone()).map_err(|e| e.to_string()))
            .collect::<Result<Vec<_>, _>>()?,
    );
    let main = &mut pod.containers[0];
    if let Some(v) = &step.definition.security_context {
        main.security_context =
            Some(serde_json::from_value(v.0.clone()).map_err(|e| e.to_string())?);
    }
    if let Some(v) = &step.definition.resources {
        main.resources = Some(serde_json::from_value(v.0.clone()).map_err(|e| e.to_string())?);
    }
    Ok((config, job))
}

fn dns_label(value: &str) -> String {
    let value = value
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>();
    let value = value.trim_matches('-');
    if value.is_empty() {
        "request".to_string()
    } else {
        value.to_string()
    }
}

fn label_value(value: &str) -> String {
    dns_label(value)
        .chars()
        .take(63)
        .collect::<String>()
        .trim_end_matches('-')
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use fase_api::{Phase, PlanReference, Step, StepStatus};
    use serde::Deserialize;

    #[test]
    fn step_pod_receives_declared_settings_and_scoped_labels() {
        let yaml = include_str!("../../examples/hello.yaml");
        let step_value = serde_yml::Deserializer::from_str(yaml)
            .next()
            .map(|doc| serde_json::Value::deserialize(doc).unwrap())
            .unwrap();
        let mut template: Step = serde_json::from_value(step_value).unwrap();
        template.spec.image_pull_secrets.push(fase_api::LocalRef {
            name: "registry".into(),
        });
        template
            .spec
            .node_selector
            .insert("kubernetes.io/os".into(), "linux".into());
        let step = StepStatus {
            name: "make".into(),
            phase: Phase::Pending,
            attempt: 0,
            step_ref: PlanReference {
                name: "make-source".into(),
                uid: "step-uid".into(),
                digest: "sha256:example".into(),
            },
            definition: template.spec,
            variables: BTreeMap::from([("FASE_VAR_VERSION".into(), "1.0".into())]),
            job_ref: None,
            outputs: BTreeMap::new(),
            message: None,
        };
        let mut request = Request::new(
            "hello",
            fase_api::RequestSpec {
                artifact_selector: fase_api::LabelSelector::default(),
                variables: BTreeMap::new(),
            },
        );
        request.metadata.namespace = Some("builds".into());
        request.metadata.uid = Some("request-uid".into());
        let settings = JobSettings {
            helper_image: "helper:dev".into(),
            output_service_account: "fase-output".into(),
            s3_bucket: "fase".into(),
            s3_endpoint: "https://silo".into(),
            s3_region: "us-east-1".into(),
            s3_prefix: String::new(),
            allow_insecure_s3: false,
            s3_anonymous: false,
            max_artifact_bytes: 512 * 1024 * 1024,
            step_timeout_seconds: 3600,
            read_secret: "read".into(),
            write_secret: "write".into(),
        };
        let manifest = JobManifest {
            inputs: vec![],
            outputs: vec![],
            namespace: "builds".into(),
            request_name: "hello".into(),
            request_uid: "request-uid".into(),
        };
        let (_, job) = resources(
            &request,
            &step,
            &manifest,
            "hello-make",
            &settings,
            vec![json!({"name":"INPUTS_SOURCE_LABELS_VERSION","value":"1.0"})],
        )
        .unwrap();
        let pod = job.spec.unwrap().template.spec.unwrap();
        assert_eq!(pod.image_pull_secrets.unwrap()[0].name, "registry");
        assert_eq!(pod.node_selector.unwrap()["kubernetes.io/os"], "linux");
        let env = pod.containers[0].env.as_ref().unwrap();
        assert!(
            env.iter()
                .any(|e| e.name == "INPUTS_SOURCE_LABELS_VERSION"
                    && e.value.as_deref() == Some("1.0"))
        );
        assert!(env.iter().any(|e| e.name == "FASE_VAR_VERSION"));
        assert!(!env.iter().any(|e| e.name.starts_with("FASE_S3_")));
    }
}
