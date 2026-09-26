use chrono::{DateTime, Timelike, Utc};
use fase_api::{
    Condition, GeneratedRequest, Request, RequestGenerator, RequestGeneratorStatus, RequestSpec,
    sha256_hex,
};
use kube::{
    Api, Client, ResourceExt,
    api::{ListParams, Patch, PatchParams, PostParams},
};
use serde_json::json;
use std::{collections::BTreeMap, time::Duration};

pub async fn run(client: Client, namespace: String) {
    loop {
        let generators: Api<RequestGenerator> = Api::namespaced(client.clone(), &namespace);
        match generators.list(&ListParams::default()).await {
            Ok(list) => {
                for generator in list {
                    if let Err(error) = reconcile(&client, &generator).await {
                        tracing::error!(generator = %generator.name_any(), %error, "generator reconciliation failed");
                        if let Err(patch_error) = record_error(&client, &generator, &error).await {
                            tracing::error!(generator = %generator.name_any(), %patch_error, "generator status update failed");
                        }
                    }
                }
            }
            Err(error) => tracing::error!(%error, "failed to list RequestGenerators"),
        }
        tokio::time::sleep(Duration::from_secs(30)).await;
    }
}

async fn record_error(
    client: &Client,
    generator: &RequestGenerator,
    message: &str,
) -> Result<(), String> {
    let mut status = generator.status.clone().unwrap_or_default();
    if status.conditions.iter().any(|condition| {
        condition.r#type == "Generated"
            && condition.status == "False"
            && condition.message == message
    }) {
        return Ok(());
    }
    status.conditions = vec![Condition {
        r#type: "Generated".into(),
        status: "False".into(),
        reason: "GenerationFailed".into(),
        message: message.into(),
        last_transition_time: Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
    }];
    let namespace = generator
        .namespace()
        .ok_or("RequestGenerator has no namespace")?;
    let api: Api<RequestGenerator> = Api::namespaced(client.clone(), &namespace);
    api.patch_status(
        &generator.name_any(),
        &PatchParams::default(),
        &Patch::Merge(json!({
            "metadata": {"resourceVersion": generator.resource_version()},
            "status": status
        })),
    )
    .await
    .map_err(|error| error.to_string())?;
    Ok(())
}

pub async fn reconcile(client: &Client, generator: &RequestGenerator) -> Result<(), String> {
    if generator.metadata.deletion_timestamp.is_some() {
        return Ok(());
    }
    let namespace = generator
        .namespace()
        .ok_or("RequestGenerator has no namespace")?;
    let now = Utc::now();
    let schedule = latest_due(
        &generator.spec.schedule.cron,
        now,
        generator
            .status
            .as_ref()
            .and_then(|status| status.last_schedule_time.as_deref()),
    )?;
    let Some(schedule) = schedule else {
        return Ok(());
    };
    let kernel_version = kernel_version(&generator.spec.source).await?;
    if generator
        .spec
        .template
        .request
        .variables
        .keys()
        .any(|name| !fase_api::valid_name(name))
    {
        return Err("generator template contains an invalid variable name".into());
    }
    let matrix = expand_matrix(&generator.spec.matrix)?;
    let requests: Api<Request> = Api::namespaced(client.clone(), &namespace);
    let mut generated = Vec::new();
    for values in matrix {
        let render = |template: &str| render_template(template, &kernel_version, &values);
        let variables = generator
            .spec
            .template
            .request
            .variables
            .iter()
            .map(|(name, template)| Ok((name.clone(), render(template)?)))
            .collect::<Result<BTreeMap<String, String>, String>>()?;
        let mut artifact_selector = generator.spec.template.request.artifact_selector.clone();
        for value in artifact_selector.match_labels.values_mut() {
            *value = render(value)?;
        }
        for expression in &mut artifact_selector.match_expressions {
            for value in &mut expression.values {
                *value = render(value)?;
            }
        }
        artifact_selector.validate()?;
        let mut recipe_selector = generator.spec.template.request.recipe_selector.clone();
        if let Some(selector) = &mut recipe_selector {
            for value in selector.match_labels.values_mut() {
                *value = render(value)?;
            }
            for expression in &mut selector.match_expressions {
                for value in &mut expression.values {
                    *value = render(value)?;
                }
            }
            selector.validate()?;
        }
        let readable_suffix = values
            .values()
            .map(String::as_str)
            .collect::<Vec<_>>()
            .join("-");
        let identity = serde_json::to_string(&values).map_err(|error| error.to_string())?;
        let stable_suffix = format!(
            "{readable_suffix}-{}",
            &sha256_hex(identity.as_bytes())[..10]
        );
        let raw_name = format!(
            "{}-{}-{}",
            generator.name_any(),
            schedule.format("%Y%m%d%H%M"),
            stable_suffix
        );
        let name = dns_label(&raw_name);
        let spec = RequestSpec {
            artifact_selector,
            recipe_selector,
            variables: variables.clone(),
            rerun: generator.spec.template.request.rerun,
        };
        let mut request = Request::new(&name, spec);
        request.metadata.namespace = Some(namespace.clone());
        if let Some(uid) = &generator.metadata.uid {
            request.metadata.owner_references = Some(vec![
                k8s_openapi::apimachinery::pkg::apis::meta::v1::OwnerReference {
                    api_version: "skyw.top/v1beta1".into(),
                    kind: "RequestGenerator".into(),
                    name: generator.name_any(),
                    uid: uid.clone(),
                    controller: Some(true),
                    block_owner_deletion: Some(false),
                },
            ]);
        }
        match requests.create(&PostParams::default(), &request).await {
            Ok(_) => {}
            Err(kube::Error::Api(error)) if error.code == 409 => {
                let existing = requests
                    .get(&name)
                    .await
                    .map_err(|error| error.to_string())?;
                if existing.spec.variables != variables
                    || existing.spec.artifact_selector != request.spec.artifact_selector
                    || existing.spec.recipe_selector != request.spec.recipe_selector
                    || existing.spec.rerun != request.spec.rerun
                    || generator.metadata.uid.as_ref().is_some_and(|uid| {
                        existing
                            .metadata
                            .owner_references
                            .as_ref()
                            .is_none_or(|owners| !owners.iter().any(|owner| &owner.uid == uid))
                    })
                {
                    return Err(format!(
                        "generated Request {name} already exists with different spec or owner"
                    ));
                }
            }
            Err(error) => return Err(error.to_string()),
        }
        generated.push(GeneratedRequest { name, variables });
    }
    let status = RequestGeneratorStatus {
        last_schedule_time: Some(schedule.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)),
        generated_requests: generated.clone(),
        conditions: vec![Condition {
            r#type: "Generated".into(),
            status: "True".into(),
            reason: "MatrixExpanded".into(),
            message: format!("{} Requests were created", generated.len()),
            last_transition_time: Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        }],
    };
    let api: Api<RequestGenerator> = Api::namespaced(client.clone(), &namespace);
    api.patch_status(
        &generator.name_any(),
        &PatchParams::default(),
        &Patch::Merge(json!({
            "metadata": {"resourceVersion": generator.resource_version()},
            "status": status
        })),
    )
    .await
    .map_err(|error| error.to_string())?;
    Ok(())
}

fn render_template(
    template: &str,
    kernel_version: &str,
    matrix: &BTreeMap<String, String>,
) -> Result<String, String> {
    match template {
        "{{ source.kernelVersion }}" => Ok(kernel_version.to_owned()),
        _ if template.starts_with("{{ matrix.") && template.ends_with(" }}") => {
            let dimension = template
                .strip_prefix("{{ matrix.")
                .and_then(|value| value.strip_suffix(" }}"))
                .ok_or_else(|| format!("unsupported template expression {template}"))?;
            matrix
                .get(dimension)
                .cloned()
                .ok_or_else(|| format!("unknown matrix dimension {dimension}"))
        }
        _ if template.contains("{{") || template.contains("}}") => {
            Err(format!("unsupported template expression {template}"))
        }
        _ => Ok(template.to_owned()),
    }
}

async fn kernel_version(source: &fase_api::GeneratorSource) -> Result<String, String> {
    if source.source_type != "kernel-release" || source.selector.get("latest") != Some(&json!(true))
    {
        return Err(
            "only source.type=kernel-release with selector.latest=true is supported".into(),
        );
    }
    let payload: serde_json::Value = reqwest::Client::new()
        .get("https://www.kernel.org/releases.json")
        .timeout(Duration::from_secs(20))
        .send()
        .await
        .map_err(|error| error.to_string())?
        .error_for_status()
        .map_err(|error| error.to_string())?
        .json()
        .await
        .map_err(|error| error.to_string())?;
    payload["latest_stable"]["version"]
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| "kernel.org response omitted latest_stable.version".into())
}

fn expand_matrix(
    matrix: &BTreeMap<String, Vec<String>>,
) -> Result<Vec<BTreeMap<String, String>>, String> {
    const MAX_COMBINATIONS: usize = 256;
    if matrix.len() > 8 {
        return Err("matrix has more than 8 dimensions".into());
    }
    let mut combinations = 1usize;
    for (name, values) in matrix {
        if !fase_api::valid_name(name) {
            return Err(format!("matrix dimension {name} has an invalid name"));
        }
        if values.is_empty() {
            return Err(format!("matrix dimension {name} is empty"));
        }
        if values
            .iter()
            .any(|value| value.is_empty() || value.len() > 128)
        {
            return Err(format!("matrix dimension {name} has an invalid value"));
        }
        let unique = values.iter().collect::<std::collections::BTreeSet<_>>();
        if unique.len() != values.len() {
            return Err(format!("matrix dimension {name} contains duplicate values"));
        }
        combinations = combinations
            .checked_mul(values.len())
            .ok_or("matrix combination count overflow")?;
        if combinations > MAX_COMBINATIONS {
            return Err(format!(
                "matrix expands to more than {MAX_COMBINATIONS} combinations"
            ));
        }
    }
    let mut rows = vec![BTreeMap::new()];
    for (name, values) in matrix {
        let mut next = Vec::new();
        for row in &rows {
            for value in values {
                let mut entry = row.clone();
                entry.insert(name.clone(), value.clone());
                next.push(entry);
            }
        }
        rows = next;
    }
    Ok(rows)
}

fn latest_due(
    expression: &str,
    now: DateTime<Utc>,
    last: Option<&str>,
) -> Result<Option<DateTime<Utc>>, String> {
    if expression.split_whitespace().count() != 5 {
        return Err("cron must contain five UTC fields".into());
    }
    let schedule: cron::Schedule = format!("0 {expression} *")
        .parse()
        .map_err(|error| format!("invalid cron: {error}"))?;
    let minute = now
        - chrono::Duration::seconds(now.second() as i64)
        - chrono::Duration::nanoseconds(now.nanosecond() as i64);
    let last = last
        .map(|value| {
            DateTime::parse_from_rfc3339(value)
                .map(|time| time.with_timezone(&Utc))
                .map_err(|error| format!("invalid lastScheduleTime: {error}"))
        })
        .transpose()?;
    let horizon = minute - chrono::Duration::days(366);
    let start = last
        .unwrap_or(minute - chrono::Duration::days(1))
        .max(horizon);
    Ok(schedule
        .after(&start)
        .take_while(|time| *time <= minute)
        .last())
}

fn dns_label(value: &str) -> String {
    let normalized = value
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>();
    let prefix = normalized
        .trim_matches('-')
        .chars()
        .take(46)
        .collect::<String>();
    let digest = fase_api::sha256_hex(value.as_bytes());
    format!("{}-{}", prefix.trim_end_matches('-'), &digest[..16])
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn matrix_is_bounded_and_names_include_a_stable_identity() {
        let matrix = BTreeMap::from([
            ("a".to_owned(), vec!["x".to_owned(), "y-z".to_owned()]),
            ("b".to_owned(), vec!["1".to_owned(), "2".to_owned()]),
        ]);
        assert_eq!(expand_matrix(&matrix).unwrap().len(), 4);
        let large = BTreeMap::from([(
            "a".to_owned(),
            (0..20).map(|value| value.to_string()).collect::<Vec<_>>(),
        )]);
        assert!(
            expand_matrix(&BTreeMap::from([
                ("a".into(), large["a"].clone()),
                ("b".into(), large["a"].clone()),
            ]))
            .is_err()
        );
        let first = dns_label("g-a-b-c");
        let second = dns_label("g-a-b-c-different");
        assert_ne!(first, second);
    }

    #[test]
    fn cron_fires_once_for_a_minute() {
        let now = DateTime::parse_from_rfc3339("2026-09-24T02:00:32Z")
            .unwrap()
            .with_timezone(&Utc);
        let due = latest_due("0 2 * * *", now, None).unwrap().unwrap();
        assert_eq!(due.to_rfc3339(), "2026-09-24T02:00:00+00:00");
        assert!(
            latest_due("0 2 * * *", now, Some("2026-09-24T02:00:00Z"))
                .unwrap()
                .is_none()
        );
    }
}
