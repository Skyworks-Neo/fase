//! Fase's Kubernetes API and shared validation.
#![forbid(unsafe_code)]

use kube::{CustomResource, CustomResourceExt};
use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

pub type Labels = BTreeMap<String, String>;
pub type Variables = BTreeMap<String, String>;

#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LabelSelector {
    #[serde(default)]
    pub match_labels: Labels,
    #[serde(default)]
    pub match_expressions: Vec<LabelExpression>,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct LabelExpression {
    pub key: String,
    pub operator: LabelOperator,
    #[serde(default)]
    pub values: Vec<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
pub enum LabelOperator {
    In,
    NotIn,
    Exists,
    DoesNotExist,
}

impl LabelSelector {
    pub fn validate(&self) -> Result<(), String> {
        if !valid_labels(&self.match_labels) {
            return Err("invalid matchLabels".into());
        }
        for expression in &self.match_expressions {
            if !valid_label_key(&expression.key)
                || !expression.values.iter().all(|v| valid_label_value(v))
            {
                return Err(format!("invalid matchExpression for {}", expression.key));
            }
            if matches!(
                expression.operator,
                LabelOperator::In | LabelOperator::NotIn
            ) != !expression.values.is_empty()
            {
                return Err(format!("invalid values for {}", expression.key));
            }
        }
        Ok(())
    }
    pub fn matches(&self, labels: &Labels) -> bool {
        self.match_labels
            .iter()
            .all(|(k, v)| labels.get(k) == Some(v))
            && self.match_expressions.iter().all(|e| match e.operator {
                LabelOperator::In => labels.get(&e.key).is_some_and(|v| e.values.contains(v)),
                LabelOperator::NotIn => labels.get(&e.key).is_none_or(|v| !e.values.contains(v)),
                LabelOperator::Exists => labels.contains_key(&e.key),
                LabelOperator::DoesNotExist => !labels.contains_key(&e.key),
            })
    }
    pub fn is_empty(&self) -> bool {
        self.match_labels.is_empty() && self.match_expressions.is_empty()
    }
}

#[derive(CustomResource, Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[kube(
    group = "fase.io",
    version = "v1alpha1",
    kind = "Artifact",
    plural = "artifacts",
    namespaced
)]
#[serde(deny_unknown_fields)]
pub struct ArtifactSpec {
    pub name: String,
    #[serde(rename = "type")]
    pub artifact_type: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ArtifactFormat {
    File,
    Directory,
    Archive,
}

/// Kubernetes owns these nested container fields; retain their native shape in the CRD.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(transparent)]
pub struct KubernetesObject(pub serde_json::Value);

impl JsonSchema for KubernetesObject {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "KubernetesObject".into()
    }
    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({"type":"object","x-kubernetes-preserve-unknown-fields":true})
    }
}

#[derive(CustomResource, Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[kube(
    group = "fase.io",
    version = "v1alpha1",
    kind = "Step",
    plural = "steps",
    namespaced
)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StepSpec {
    pub image: String,
    pub command: Vec<String>,
    #[serde(default)]
    pub image_pull_secrets: Vec<LocalRef>,
    #[serde(default)]
    pub node_selector: Labels,
    #[serde(default)]
    pub tolerations: Vec<KubernetesObject>,
    #[serde(default)]
    pub security_context: Option<KubernetesObject>,
    #[serde(default)]
    pub resources: Option<KubernetesObject>,
    #[serde(default)]
    pub env: Vec<EnvVar>,
    #[serde(default)]
    pub inputs: StepInputs,
    pub outputs: StepOutputs,
    pub script: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LocalRef {
    pub name: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EnvVar {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value_from: Option<KubernetesObject>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StepInputs {
    #[serde(default)]
    pub variables: Vec<VariableInput>,
    #[serde(default)]
    pub artifacts: Vec<ArtifactPort>,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct VariableInput {
    pub name: String,
    pub env: String,
    #[serde(default)]
    pub required: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ArtifactPort {
    pub name: String,
    pub path: String,
    pub format: ArtifactFormat,
    #[serde(rename = "type", default, skip_serializing_if = "Option::is_none")]
    pub artifact_type: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StepOutputs {
    pub artifacts: Vec<ArtifactPort>,
}

#[derive(CustomResource, Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[kube(
    group = "fase.io",
    version = "v1alpha1",
    kind = "Plan",
    plural = "plans",
    namespaced
)]
#[serde(deny_unknown_fields)]
pub struct PlanSpec {
    #[serde(default)]
    pub inputs: PlanInputs,
    pub steps: Vec<PlanStep>,
    pub outputs: PlanOutputs,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PlanInputs {
    #[serde(default)]
    pub variables: Vec<PlanVariable>,
    #[serde(default)]
    pub artifacts: Vec<PlanArtifactInput>,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PlanVariable {
    pub name: String,
    #[serde(rename = "type")]
    pub value_type: VariableType,
    #[serde(default)]
    pub required: bool,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum VariableType {
    String,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PlanArtifactInput {
    pub name: String,
    pub artifact_selector: LabelSelector,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PlanStep {
    pub name: String,
    pub step_selector: LabelSelector,
    #[serde(default)]
    pub variables: BTreeMap<String, VariableBinding>,
    #[serde(default)]
    pub inputs: StepBindings,
    pub outputs: Vec<NamedOutput>,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VariableBinding {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_variable: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StepBindings {
    #[serde(default)]
    pub artifacts: Vec<ArtifactBinding>,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ArtifactBinding {
    pub name: String,
    pub from: ArtifactOrigin,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ArtifactOrigin {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_input: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NamedOutput {
    pub name: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PlanOutputs {
    pub artifacts: Vec<PlanOutput>,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PlanOutput {
    pub name: String,
    pub from: ArtifactOrigin,
    pub labels: BTreeMap<String, LabelBinding>,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(untagged)]
pub enum LabelBinding {
    Literal(String),
    Bound(LabelSource),
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LabelSource {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_variable: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_input: Option<InputLabel>,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InputLabel {
    pub input: String,
    pub label: String,
}

#[derive(CustomResource, Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[kube(
    group = "fase.io",
    version = "v1alpha1",
    kind = "Request",
    plural = "requests",
    namespaced,
    status = "RequestStatus"
)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RequestSpec {
    pub artifact_selector: LabelSelector,
    #[serde(default)]
    pub variables: Variables,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RequestStatus {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase: Option<Phase>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_digest: Option<String>,
    #[serde(default)]
    pub resolved_plans: Vec<ResolvedPlanStatus>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact_ref: Option<ArtifactReference>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_output: Option<String>,
    #[serde(default)]
    pub conditions: Vec<Condition>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
pub enum Phase {
    Pending,
    Running,
    Verifying,
    Succeeded,
    Failed,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PlanReference {
    pub name: String,
    pub uid: String,
    pub digest: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResolvedPlanStatus {
    pub name: String,
    pub plan_ref: PlanReference,
    pub definition: PlanSpec,
    pub variables: Variables,
    pub phase: Phase,
    pub inputs: ResolvedInputs,
    pub steps: Vec<StepStatus>,
    pub outputs: ResolvedOutputs,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ResolvedInputs {
    #[serde(default)]
    pub artifacts: Vec<ResolvedInput>,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ResolvedInput {
    pub name: String,
    pub from: ResolvedInputOrigin,
    pub labels: Labels,
    #[serde(rename = "type")]
    pub artifact_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact_ref: Option<ArtifactReference>,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResolvedInputOrigin {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_plan: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StepStatus {
    pub name: String,
    pub phase: Phase,
    pub attempt: i32,
    pub step_ref: PlanReference,
    pub definition: StepSpec,
    pub variables: Variables,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub job_ref: Option<ArtifactReference>,
    #[serde(default)]
    pub outputs: BTreeMap<String, ArtifactReference>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ResolvedOutputs {
    #[serde(default)]
    pub artifacts: BTreeMap<String, ResolvedOutput>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResolvedOutput {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact_ref: Option<ArtifactReference>,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ArtifactReference {
    pub name: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Condition {
    pub r#type: String,
    pub status: String,
    pub reason: String,
    pub message: String,
    pub last_transition_time: String,
}

#[derive(CustomResource, Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[kube(
    group = "fase.io",
    version = "v1alpha1",
    kind = "RequestGenerator",
    plural = "requestgenerators",
    namespaced,
    status = "RequestGeneratorStatus"
)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RequestGeneratorSpec {
    pub schedule: Schedule,
    pub template: GeneratorTemplate,
    #[serde(default)]
    pub matrix: BTreeMap<String, Vec<String>>,
    pub source: GeneratorSource,
}
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Schedule {
    pub cron: String,
}
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GeneratorTemplate {
    pub request: RequestSpec,
}
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GeneratorSource {
    #[serde(rename = "type")]
    pub source_type: String,
    pub selector: BTreeMap<String, serde_json::Value>,
}
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RequestGeneratorStatus {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_schedule_time: Option<String>,
    #[serde(default)]
    pub generated_requests: Vec<GeneratedRequest>,
    #[serde(default)]
    pub conditions: Vec<Condition>,
}
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GeneratedRequest {
    pub name: String,
    pub variables: Variables,
}

pub fn crds_yaml() -> Result<String, serde_yml::Error> {
    let mut result = String::new();
    for crd in [
        Artifact::crd(),
        Step::crd(),
        Plan::crd(),
        Request::crd(),
        RequestGenerator::crd(),
    ] {
        if !result.is_empty() {
            result.push_str("---\n");
        }
        result.push_str(&serde_yml::to_string(&crd)?);
    }
    Ok(result)
}
pub fn spec_digest<T: Serialize>(value: &T) -> Result<String, serde_json::Error> {
    Ok(format!(
        "sha256:{}",
        sha256_hex(&serde_json::to_vec(value)?)
    ))
}
pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
pub fn valid_labels(labels: &Labels) -> bool {
    labels
        .iter()
        .all(|(k, v)| valid_label_key(k) && valid_label_value(v))
}
pub fn valid_label_key(key: &str) -> bool {
    let (prefix, name) = key
        .split_once('/')
        .map_or((None, key), |(p, n)| (Some(p), n));
    let valid_part = |s: &str| {
        !s.is_empty()
            && s.len() <= 63
            && s.bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
            && s.as_bytes().first().is_some_and(u8::is_ascii_alphanumeric)
            && s.as_bytes().last().is_some_and(u8::is_ascii_alphanumeric)
    };
    valid_part(name) && prefix.is_none_or(|p| p.len() <= 253 && p.split('.').all(valid_part))
}
pub fn valid_label_value(v: &str) -> bool {
    v.is_empty()
        || (v.len() <= 63
            && v.bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
            && v.as_bytes().first().is_some_and(u8::is_ascii_alphanumeric)
            && v.as_bytes().last().is_some_and(u8::is_ascii_alphanumeric))
}
pub fn valid_name(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 63
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
}
pub fn valid_path(path: &str) -> bool {
    let p = std::path::Path::new(path);
    !path.is_empty()
        && p.components()
            .all(|c| matches!(c, std::path::Component::Normal(_)))
        && p.components()
            .map(|c| c.as_os_str().to_string_lossy())
            .collect::<Vec<_>>()
            .join("/")
            == path
}
impl StepSpec {
    pub fn validate(&self) -> Result<(), String> {
        if self.image.trim().is_empty()
            || self.command.len() != 1
            || !matches!(
                self.command[0].as_str(),
                "/bin/sh" | "/bin/bash" | "sh" | "bash"
            )
            || self.script.is_empty()
            || self.script.len() > 512 * 1024
        {
            return Err("invalid image, command or script".into());
        }
        if self
            .image_pull_secrets
            .iter()
            .any(|secret| secret.name.is_empty())
            || !valid_labels(&self.node_selector)
        {
            return Err("invalid Pod placement settings".into());
        }
        for value in &self.tolerations {
            serde_json::from_value::<k8s_openapi::api::core::v1::Toleration>(value.0.clone())
                .map_err(|e| format!("invalid toleration: {e}"))?;
        }
        if let Some(value) = &self.security_context {
            serde_json::from_value::<k8s_openapi::api::core::v1::SecurityContext>(value.0.clone())
                .map_err(|e| format!("invalid securityContext: {e}"))?;
        }
        if let Some(value) = &self.resources {
            serde_json::from_value::<k8s_openapi::api::core::v1::ResourceRequirements>(
                value.0.clone(),
            )
            .map_err(|e| format!("invalid resources: {e}"))?;
        }
        let mut names = BTreeSet::new();
        let mut env = BTreeSet::new();
        for e in &self.env {
            if !valid_env(&e.name)
                || e.name.starts_with("INPUTS_")
                || e.value.is_some() == e.value_from.is_some()
                || !env.insert(e.name.clone())
            {
                return Err(format!("invalid env {}", e.name));
            }
            if let Some(value) = &e.value_from {
                serde_json::from_value::<k8s_openapi::api::core::v1::EnvVarSource>(value.0.clone())
                    .map_err(|error| format!("invalid valueFrom for {}: {error}", e.name))?;
            }
        }
        for v in &self.inputs.variables {
            if !valid_name(&v.name)
                || !names.insert(v.name.clone())
                || !valid_env(&v.env)
                || v.env.starts_with("INPUTS_")
                || !env.insert(v.env.clone())
            {
                return Err(format!("invalid variable {}", v.name));
            }
        }
        for ports in [&self.inputs.artifacts, &self.outputs.artifacts] {
            let mut names = BTreeSet::new();
            let mut paths = BTreeSet::new();
            for p in ports {
                if !valid_name(&p.name)
                    || !valid_path(&p.path)
                    || p.artifact_type
                        .as_ref()
                        .is_some_and(|kind| !artifact_type_matches_format(kind, p.format))
                    || !names.insert(&p.name)
                    || !paths.insert(&p.path)
                {
                    return Err(format!("invalid artifact port {}", p.name));
                }
            }
            for a in &paths {
                for b in &paths {
                    if a != b && b.starts_with(&format!("{a}/")) {
                        return Err("overlapping artifact paths".into());
                    }
                }
            }
        }
        Ok(())
    }
}
pub fn artifact_type_matches_format(kind: &str, format: ArtifactFormat) -> bool {
    matches!(
        (kind, format),
        ("file", ArtifactFormat::File)
            | ("directory", ArtifactFormat::Directory)
            | ("tar.zst", ArtifactFormat::Archive)
    )
}
pub fn artifact_format(kind: &str) -> Option<ArtifactFormat> {
    match kind {
        "file" => Some(ArtifactFormat::File),
        "directory" => Some(ArtifactFormat::Directory),
        "tar.zst" => Some(ArtifactFormat::Archive),
        _ => None,
    }
}
pub fn valid_env(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 253
        && s.bytes()
            .next()
            .is_some_and(|b| b.is_ascii_alphabetic() || b == b'_')
        && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn label_selector_uses_kubernetes_expression_semantics() {
        let selector = LabelSelector {
            match_labels: Labels::from([("name".into(), "package".into())]),
            match_expressions: vec![LabelExpression {
                key: "arch".into(),
                operator: LabelOperator::NotIn,
                values: vec!["arm64".into()],
            }],
        };
        selector.validate().unwrap();
        assert!(selector.matches(&Labels::from([("name".into(), "package".into())])));
        assert!(selector.matches(&Labels::from([
            ("name".into(), "package".into()),
            ("arch".into(), "amd64".into())
        ])));
        assert!(!selector.matches(&Labels::from([
            ("name".into(), "package".into()),
            ("arch".into(), "arm64".into())
        ])));
    }
}
