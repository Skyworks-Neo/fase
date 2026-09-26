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
    group = "skyw.top",
    version = "v1beta1",
    kind = "Artifact",
    plural = "artifacts",
    namespaced
)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[schemars(extend("x-kubernetes-validations" = [{"rule": "self == oldSelf", "message": "Artifact spec is immutable"}]))]
pub struct ArtifactSpec {
    pub content_digest: String,
    pub size_bytes: i64,
    pub kind: ArtifactKind,
    pub storage_ref: StorageReference,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StorageReference {
    pub key: String,
}

#[derive(CustomResource, Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[kube(
    group = "skyw.top",
    version = "v1beta1",
    kind = "ArtifactClaim",
    plural = "artifactclaims",
    namespaced
)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[schemars(extend("x-kubernetes-validations" = [{"rule": "self == oldSelf", "message": "ArtifactClaim spec is immutable"}]))]
pub struct ArtifactClaimSpec {
    pub artifact_ref: ArtifactReference,
    pub build_key: String,
    pub producer: ProducerReference,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProducerReference {
    pub recipe_ref: ObjectReference,
    pub run_ref: ObjectReference,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ArtifactKind {
    File,
    Tree,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(transparent)]
pub struct TolerationValue(pub serde_json::Value);
impl JsonSchema for TolerationValue {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "TolerationValue".into()
    }
    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({"type":"object","additionalProperties":false,"properties":{
            "key":{"type":"string"},"operator":{"type":"string","enum":["Exists","Equal"]},
            "value":{"type":"string"},"effect":{"type":"string","enum":["NoSchedule","PreferNoSchedule","NoExecute",""]},
            "tolerationSeconds":{"type":"integer","format":"int64","minimum":0}
        }})
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(transparent)]
pub struct SecurityContextValue(pub serde_json::Value);
impl JsonSchema for SecurityContextValue {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "SecurityContextValue".into()
    }
    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({"type":"object","additionalProperties":false,"properties":{
            "allowPrivilegeEscalation":{"type":"boolean"},"privileged":{"type":"boolean"},
            "readOnlyRootFilesystem":{"type":"boolean"},"runAsNonRoot":{"type":"boolean"},
            "runAsUser":{"type":"integer","format":"int64"},"runAsGroup":{"type":"integer","format":"int64"},
            "capabilities":{"type":"object","additionalProperties":false,"properties":{
                "add":{"type":"array","items":{"type":"string"}},"drop":{"type":"array","items":{"type":"string"}}}},
            "seccompProfile":{"type":"object","additionalProperties":false,"required":["type"],"properties":{
                "type":{"type":"string","enum":["Localhost","RuntimeDefault","Unconfined"]},"localhostProfile":{"type":"string"}}},
            "seLinuxOptions":{"type":"object","additionalProperties":false,"properties":{
                "user":{"type":"string"},"role":{"type":"string"},"type":{"type":"string"},"level":{"type":"string"}}},
            "procMount":{"type":"string","enum":["Default","Unmasked"]},
            "appArmorProfile":{"type":"object","additionalProperties":false,"required":["type"],"properties":{
                "type":{"type":"string","enum":["Localhost","RuntimeDefault","Unconfined"]},"localhostProfile":{"type":"string"}}}
        }})
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(transparent)]
pub struct ResourcesValue(pub serde_json::Value);
impl JsonSchema for ResourcesValue {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "ResourcesValue".into()
    }
    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({"type":"object","additionalProperties":false,"properties":{
            "requests":{"type":"object","additionalProperties":{"type":"string"}},
            "limits":{"type":"object","additionalProperties":{"type":"string"}},
            "claims":{"type":"array","items":{"type":"object","additionalProperties":false,"required":["name"],"properties":{
                "name":{"type":"string"},"request":{"type":"string"}}}}
        }})
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(transparent)]
pub struct EnvValueFrom(pub serde_json::Value);
impl JsonSchema for EnvValueFrom {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "EnvValueFrom".into()
    }
    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        let key_ref = serde_json::json!({"type":"object","additionalProperties":false,"required":["name","key"],"properties":{
            "name":{"type":"string"},"key":{"type":"string"},"optional":{"type":"boolean"}}});
        let field_ref = serde_json::json!({"type":"object","additionalProperties":false,"required":["fieldPath"],"properties":{
            "apiVersion":{"type":"string"},"fieldPath":{"type":"string"}}});
        let resource_ref = serde_json::json!({"type":"object","additionalProperties":false,"required":["resource"],"properties":{
            "containerName":{"type":"string"},"resource":{"type":"string"},"divisor":{"type":"string"}}});
        json_schema!({"type":"object","additionalProperties":false,"properties":{
            "configMapKeyRef":key_ref,"secretKeyRef":key_ref,
            "fieldRef":field_ref,"resourceFieldRef":resource_ref
        },"oneOf":[
            {"required":["configMapKeyRef"]}, {"required":["secretKeyRef"]},
            {"required":["fieldRef"]}, {"required":["resourceFieldRef"]}
        ]})
    }
}

#[derive(CustomResource, Clone, Debug, Default, Deserialize, Serialize, JsonSchema)]
#[kube(
    group = "skyw.top",
    version = "v1beta1",
    kind = "Task",
    plural = "tasks",
    namespaced
)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TaskSpec {
    pub image: String,
    pub command: Vec<String>,
    #[serde(default)]
    pub image_pull_secrets: Vec<LocalRef>,
    #[serde(default)]
    pub node_selector: Labels,
    #[serde(default)]
    pub tolerations: Vec<TolerationValue>,
    #[serde(default)]
    pub security_context: Option<SecurityContextValue>,
    #[serde(default)]
    pub resources: Option<ResourcesValue>,
    #[serde(default)]
    pub workspace: TaskWorkspace,
    #[serde(default)]
    pub env: Vec<EnvVar>,
    #[serde(default)]
    pub retry: RetryPolicy,
    #[serde(default)]
    pub inputs: TaskInputs,
    pub outputs: TaskOutputs,
    pub script: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TaskWorkspace {
    pub task_size_limit: String,
    pub input_size_limit: String,
    pub output_size_limit: String,
    pub transfer_size_limit: String,
}

impl Default for TaskWorkspace {
    fn default() -> Self {
        Self {
            task_size_limit: "2Gi".into(),
            input_size_limit: "2Gi".into(),
            output_size_limit: "2Gi".into(),
            transfer_size_limit: "2Gi".into(),
        }
    }
}

impl TaskWorkspace {
    fn validate(&self) -> Result<(), String> {
        for (name, value) in [
            ("taskSizeLimit", &self.task_size_limit),
            ("inputSizeLimit", &self.input_size_limit),
            ("outputSizeLimit", &self.output_size_limit),
            ("transferSizeLimit", &self.transfer_size_limit),
        ] {
            let amount = value
                .strip_suffix("Gi")
                .and_then(|number| number.parse::<u32>().ok())
                .filter(|amount| (1..=512).contains(amount));
            if amount.is_none() {
                return Err(format!("workspace.{name} must be between 1Gi and 512Gi"));
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RetryPolicy {
    pub max_attempts: i32,
    #[serde(default)]
    pub retry_on: Vec<FailureClass>,
}
impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 1,
            retry_on: Vec::new(),
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
pub enum FailureClass {
    Infrastructure,
    Output,
    Task,
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
    pub value_from: Option<EnvValueFrom>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TaskInputs {
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
    pub kind: ArtifactKind,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TaskOutputs {
    pub artifacts: Vec<ArtifactPort>,
}

#[derive(CustomResource, Clone, Debug, Default, Deserialize, Serialize, JsonSchema)]
#[kube(
    group = "skyw.top",
    version = "v1beta1",
    kind = "Recipe",
    plural = "recipes",
    namespaced
)]
#[serde(deny_unknown_fields)]
pub struct RecipeSpec {
    #[serde(default)]
    pub inputs: RecipeInputs,
    pub tasks: Vec<RecipeTask>,
    pub outputs: RecipeOutputs,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RecipeInputs {
    #[serde(default)]
    pub variables: Vec<RecipeVariable>,
    #[serde(default)]
    pub artifacts: Vec<RecipeArtifactInput>,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RecipeVariable {
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
pub struct RecipeArtifactInput {
    pub name: String,
    pub artifact_selector: LabelSelector,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RecipeTask {
    pub name: String,
    pub task_selector: LabelSelector,
    #[serde(default)]
    pub variables: BTreeMap<String, VariableBinding>,
    #[serde(default)]
    pub inputs: TaskBindings,
    pub outputs: Vec<NamedOutput>,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VariableBinding {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_variable: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_input: Option<InputLabel>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TaskBindings {
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
    pub recipe_input: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NamedOutput {
    pub name: String,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RecipeOutputs {
    pub artifacts: Vec<RecipeOutput>,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RecipeOutput {
    pub name: String,
    pub from: ArtifactOrigin,
    pub labels: BTreeMap<String, LabelBinding>,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(untagged)]
pub enum LabelBinding {
    Literal(LabelLiteral),
    Bound(LabelSource),
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LabelLiteral {
    pub value: String,
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
    group = "skyw.top",
    version = "v1beta1",
    kind = "Request",
    plural = "requests",
    namespaced,
    status = "RequestStatus"
)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RequestSpec {
    pub artifact_selector: LabelSelector,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recipe_selector: Option<LabelSelector>,
    #[serde(default)]
    pub variables: Variables,
    #[serde(default)]
    #[schemars(range(min = 0))]
    pub rerun: i64,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ObjectReference {
    pub api_version: String,
    pub kind: String,
    pub name: String,
    pub uid: String,
    pub generation: i64,
}

#[derive(CustomResource, Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[kube(
    group = "skyw.top",
    version = "v1beta1",
    kind = "Run",
    plural = "runs",
    namespaced,
    status = "RunStatus"
)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[schemars(extend("x-kubernetes-validations" = [{"rule": "self == oldSelf", "message": "Run spec is immutable"}]))]
pub struct RunSpec {
    pub request_ref: ObjectReference,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RequestStatus {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase: Option<Phase>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_ref: Option<ArtifactReference>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claim_ref: Option<ArtifactReference>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact_ref: Option<ArtifactReference>,
    #[serde(default)]
    pub conditions: Vec<Condition>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RunStatus {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase: Option<Phase>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_digest: Option<String>,
    #[serde(default)]
    pub resolved_recipes: Vec<ResolvedRecipeStatus>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact_ref: Option<ArtifactReference>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_digest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claim_ref: Option<ArtifactReference>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_output: Option<String>,
    #[serde(default)]
    pub diagnostics: Vec<ResolutionDiagnostic>,
    #[serde(default)]
    pub conditions: Vec<Condition>,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResolutionDiagnostic {
    pub candidate: String,
    pub reason: String,
    pub message: String,
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
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResolvedRecipeStatus {
    pub name: String,
    pub recipe_ref: ObjectReference,
    #[serde(skip)]
    #[schemars(skip)]
    pub definition: RecipeSpec,
    pub variables: Variables,
    pub phase: Phase,
    pub inputs: ResolvedInputs,
    pub tasks: Vec<TaskStatus>,
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
    pub kind: ArtifactKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_digest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact_ref: Option<ArtifactReference>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claim_ref: Option<ArtifactReference>,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResolvedInputOrigin {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_recipe: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TaskStatus {
    pub name: String,
    pub phase: Phase,
    pub attempt: i32,
    pub task_ref: ObjectReference,
    #[serde(skip)]
    #[schemars(skip)]
    pub definition: TaskSpec,
    pub variables: Variables,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub build_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub job_ref: Option<ArtifactReference>,
    #[serde(default)]
    pub outputs: BTreeMap<String, ArtifactReference>,
    #[serde(default)]
    pub output_digests: BTreeMap<String, String>,
    #[serde(default)]
    pub claims: BTreeMap<String, ArtifactReference>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_digest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claim_ref: Option<ArtifactReference>,
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
    group = "skyw.top",
    version = "v1beta1",
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
        ArtifactClaim::crd(),
        Task::crd(),
        Recipe::crd(),
        Request::crd(),
        Run::crd(),
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
    valid_part(name)
        && prefix.is_none_or(|p| {
            p.len() <= 253
                && p.split('.').all(|part| {
                    !part.is_empty()
                        && part.len() <= 63
                        && part
                            .bytes()
                            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
                        && part
                            .as_bytes()
                            .first()
                            .is_some_and(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
                        && part
                            .as_bytes()
                            .last()
                            .is_some_and(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
                })
        })
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
impl TaskSpec {
    pub fn validate(&self) -> Result<(), String> {
        self.workspace.validate()?;
        if !(1..=10).contains(&self.retry.max_attempts) {
            return Err("retry.maxAttempts must be between 1 and 10".into());
        }
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
                let source = value
                    .0
                    .as_object()
                    .ok_or_else(|| format!("invalid valueFrom for {}", e.name))?;
                if source
                    .keys()
                    .filter(|key| {
                        matches!(
                            key.as_str(),
                            "configMapKeyRef" | "secretKeyRef" | "fieldRef" | "resourceFieldRef"
                        )
                    })
                    .count()
                    != 1
                {
                    return Err(format!(
                        "valueFrom for {} must select exactly one source",
                        e.name
                    ));
                }
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
        assert!(valid_label_key("skyw.top/name"));
        assert!(!valid_label_key("bad_prefix/name"));
        assert!(!valid_label_key("Upper.example/name"));
    }

    #[test]
    fn run_status_serializes_references_without_definition_snapshots() {
        let reference = ObjectReference {
            api_version: "skyw.top/v1beta1".into(),
            kind: "Recipe".into(),
            name: "recipe".into(),
            uid: "uid".into(),
            generation: 2,
        };
        let task_reference = ObjectReference {
            kind: "Task".into(),
            ..reference.clone()
        };
        let task = TaskStatus {
            name: "task".into(),
            phase: Phase::Pending,
            attempt: 0,
            task_ref: task_reference,
            definition: TaskSpec::default(),
            variables: Variables::new(),
            build_key: None,
            job_ref: None,
            outputs: BTreeMap::new(),
            output_digests: BTreeMap::new(),
            claims: BTreeMap::new(),
            message: None,
        };
        let recipe = ResolvedRecipeStatus {
            name: "recipe".into(),
            recipe_ref: reference,
            definition: RecipeSpec::default(),
            variables: Variables::new(),
            phase: Phase::Pending,
            inputs: ResolvedInputs::default(),
            tasks: vec![task],
            outputs: ResolvedOutputs::default(),
        };
        let status = RunStatus {
            resolved_recipes: vec![recipe],
            ..RunStatus::default()
        };
        let json = serde_json::to_value(status).unwrap();
        assert!(json["resolvedRecipes"][0].get("definition").is_none());
        assert!(
            json["resolvedRecipes"][0]["tasks"][0]
                .get("definition")
                .is_none()
        );
        assert_eq!(json["resolvedRecipes"][0]["recipeRef"]["generation"], 2);
    }

    #[test]
    fn generated_crds_expose_only_file_and_tree_ports() {
        let crds = crds_yaml().unwrap();
        assert!(!crds.contains("v1alpha1"));
        let task = serde_yml::Deserializer::from_str(&crds)
            .filter_map(|doc| serde_json::Value::deserialize(doc).ok())
            .find(|doc| doc["spec"]["names"]["kind"] == "Task")
            .unwrap();
        let port = &task["spec"]["versions"][0]["schema"]["openAPIV3Schema"]["properties"]["spec"]
            ["properties"]["outputs"]["properties"]["artifacts"]["items"]["properties"];
        assert!(port.get("kind").is_some());
        assert!(port.get("format").is_none());
        assert!(port.get("type").is_none());
    }
}
