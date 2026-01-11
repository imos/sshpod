use anyhow::{bail, Context, Result};
use serde::Deserialize;
use std::collections::HashMap;
use std::process::{Output, Stdio};
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

#[derive(Debug, Clone)]
pub struct PodInfo {
    pub uid: String,
    pub containers: Vec<String>,
}

#[derive(Deserialize)]
struct Pod {
    metadata: PodMetadata,
    spec: PodSpec,
}

#[derive(Deserialize)]
struct PodMetadata {
    uid: String,
}

#[derive(Deserialize)]
struct PodSpec {
    containers: Vec<ContainerSpec>,
}

#[derive(Deserialize)]
struct ContainerSpec {
    name: String,
}

#[derive(Deserialize)]
struct LabelSelector {
    #[serde(default, rename = "matchLabels")]
    match_labels: HashMap<String, String>,
    #[serde(default, rename = "matchExpressions")]
    match_expressions: Vec<MatchExpression>,
}

#[derive(Deserialize)]
struct MatchExpression {
    key: String,
    operator: String,
    #[serde(default)]
    values: Vec<String>,
}

#[derive(Deserialize)]
struct Deployment {
    spec: DeploymentSpec,
}

#[derive(Deserialize)]
struct DeploymentSpec {
    selector: LabelSelector,
}

#[derive(Deserialize)]
struct Job {
    spec: JobSpec,
}

#[derive(Deserialize)]
struct JobSpec {
    #[serde(default)]
    selector: Option<LabelSelector>,
    template: PodTemplate,
}

#[derive(Deserialize)]
struct PodTemplate {
    metadata: Option<PodTemplateMetadata>,
}

#[derive(Deserialize)]
struct PodTemplateMetadata {
    #[serde(default)]
    labels: HashMap<String, String>,
}

#[derive(Deserialize)]
struct PodList {
    items: Vec<PodListItem>,
}

#[derive(Deserialize)]
struct PodListItem {
    metadata: PodMetadataName,
    #[serde(default)]
    status: Option<PodStatus>,
}

#[derive(Deserialize)]
struct PodMetadataName {
    name: String,
}

#[derive(Deserialize)]
struct PodStatus {
    #[serde(default)]
    phase: Option<String>,
    #[serde(default, rename = "conditions")]
    conditions: Option<Vec<PodCondition>>,
}

#[derive(Deserialize)]
struct PodCondition {
    #[serde(rename = "type")]
    type_name: String,
    status: String,
}

#[derive(Deserialize)]
struct DeploymentList {
    items: Vec<DeploymentItem>,
}

#[derive(Deserialize)]
struct DeploymentItem {
    metadata: PodMetadataName,
    #[serde(default)]
    status: Option<DeploymentStatus>,
}

#[derive(Deserialize)]
struct DeploymentStatus {
    #[serde(default, rename = "availableReplicas")]
    available_replicas: Option<u32>,
    #[serde(default, rename = "readyReplicas")]
    ready_replicas: Option<u32>,
}

#[derive(Deserialize)]
struct JobList {
    items: Vec<JobItem>,
}

#[derive(Deserialize)]
struct JobItem {
    metadata: PodMetadataName,
    #[serde(default)]
    status: Option<JobStatus>,
}

#[derive(Deserialize)]
struct JobStatus {
    #[serde(default)]
    succeeded: Option<u32>,
    #[serde(default)]
    active: Option<u32>,
    #[serde(default)]
    ready: Option<u32>,
}

fn kubectl_base(context: Option<&str>) -> Command {
    let mut cmd = Command::new("kubectl");
    if let Some(ctx) = context {
        cmd.arg("--context").arg(ctx);
    }
    cmd
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_ready_true() {
        let pod = PodListItem {
            metadata: PodMetadataName { name: "p".into() },
            status: Some(PodStatus {
                phase: Some("Running".into()),
                conditions: Some(vec![PodCondition {
                    type_name: "Ready".into(),
                    status: "True".into(),
                }]),
            }),
        };
        assert!(is_ready(&pod));
    }

    #[test]
    fn test_is_ready_false_when_not_running() {
        let pod = PodListItem {
            metadata: PodMetadataName { name: "p".into() },
            status: Some(PodStatus {
                phase: Some("Pending".into()),
                conditions: None,
            }),
        };
        assert!(!is_ready(&pod));
    }
}

pub async fn ensure_context_exists(context: &str) -> Result<()> {
    let contexts = list_contexts().await?;
    if contexts.iter().any(|c| c == context) {
        return Ok(());
    }
    bail!(
        "context `{}` not found. Available contexts: {}",
        context,
        contexts.join(", ")
    );
}

pub async fn list_contexts() -> Result<Vec<String>> {
    let output = Command::new("kubectl")
        .args(["config", "get-contexts", "-o", "name"])
        .output()
        .await
        .context("failed to run kubectl config get-contexts")?;
    if !output.status.success() {
        bail!(
            "kubectl config get-contexts failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let list = String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    Ok(list)
}

pub async fn get_context_namespace(context: &str) -> Result<Option<String>> {
    let output = Command::new("kubectl")
        .args([
            "config",
            "view",
            "-o",
            &format!(
                "jsonpath={{.contexts[?(@.name==\"{}\")].context.namespace}}",
                context
            ),
        ])
        .output()
        .await
        .context("failed to run kubectl config view")?;
    if !output.status.success() {
        bail!(
            "kubectl config view failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let ns = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if ns.is_empty() {
        Ok(None)
    } else {
        Ok(Some(ns))
    }
}

pub async fn get_pod_info(context: Option<&str>, namespace: &str, pod: &str) -> Result<PodInfo> {
    let output = kubectl_base(context)
        .args(["get", "pod", pod, "-n", namespace, "-o", "json"])
        .output()
        .await
        .context("failed to run kubectl get pod")?;

    if !output.status.success() {
        let available = list_resources(context, namespace, "pod")
            .await
            .unwrap_or_default();
        bail!(
            "kubectl get pod failed: {}. Ready pods: {}",
            String::from_utf8_lossy(&output.stderr).trim(),
            available.join(", ")
        );
    }

    let parsed: Pod = serde_json::from_slice(&output.stdout)
        .context("failed to parse kubectl get pod json output")?;

    Ok(PodInfo {
        uid: parsed.metadata.uid,
        containers: parsed.spec.containers.into_iter().map(|c| c.name).collect(),
    })
}

pub async fn choose_pod_for_deployment(
    context: Option<&str>,
    namespace: &str,
    deployment: &str,
) -> Result<String> {
    let output = kubectl_base(context)
        .args([
            "get",
            "deployment",
            deployment,
            "-n",
            namespace,
            "-o",
            "json",
        ])
        .output()
        .await
        .context("failed to run kubectl get deployment")?;
    if !output.status.success() {
        let available = list_resources(context, namespace, "deployment")
            .await
            .unwrap_or_default();
        bail!(
            "kubectl get deployment failed: {}. Ready deployments: {}",
            String::from_utf8_lossy(&output.stderr).trim(),
            available.join(", ")
        );
    }
    let deploy: Deployment =
        serde_json::from_slice(&output.stdout).context("failed to parse deployment json")?;
    let selector = to_selector(&deploy.spec.selector)?;
    select_pod(context, namespace, &selector, "deployment").await
}

pub async fn choose_pod_for_job(
    context: Option<&str>,
    namespace: &str,
    job: &str,
) -> Result<String> {
    let output = kubectl_base(context)
        .args(["get", "job", job, "-n", namespace, "-o", "json"])
        .output()
        .await
        .context("failed to run kubectl get job")?;
    if !output.status.success() {
        let available = list_resources(context, namespace, "job")
            .await
            .unwrap_or_default();
        bail!(
            "kubectl get job failed: {}. Ready jobs: {}",
            String::from_utf8_lossy(&output.stderr).trim(),
            available.join(", ")
        );
    }
    let job_spec: Job =
        serde_json::from_slice(&output.stdout).context("failed to parse job json")?;
    let selector = if let Some(selector) = job_spec.spec.selector {
        to_selector(&selector)?
    } else if let Some(meta) = job_spec.spec.template.metadata {
        if meta.labels.is_empty() {
            format!("job-name={}", job)
        } else {
            to_selector(&LabelSelector {
                match_labels: meta.labels,
                match_expressions: Vec::new(),
            })?
        }
    } else {
        format!("job-name={}", job)
    };
    select_pod(context, namespace, &selector, "job").await
}

async fn select_pod(
    context: Option<&str>,
    namespace: &str,
    selector: &str,
    kind: &str,
) -> Result<String> {
    let output = kubectl_base(context)
        .args(["get", "pods", "-n", namespace, "-l", selector, "-o", "json"])
        .output()
        .await
        .context("failed to run kubectl get pods for selector")?;
    if !output.status.success() {
        bail!(
            "kubectl get pods failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let pods: PodList =
        serde_json::from_slice(&output.stdout).context("failed to parse pods json")?;
    if pods.items.is_empty() {
        bail!(
            "no pods found for {} selector `{}` in namespace {}",
            kind,
            selector,
            namespace
        );
    }
    if let Some(p) = pods
        .items
        .iter()
        .find(|p| is_ready(p))
        .or_else(|| pods.items.iter().find(|p| is_running(p)))
        .or_else(|| pods.items.first())
    {
        return Ok(p.metadata.name.clone());
    }
    bail!(
        "no suitable pods found for {} selector `{}` in namespace {}",
        kind,
        selector,
        namespace
    );
}

fn to_selector(sel: &LabelSelector) -> Result<String> {
    let mut parts = Vec::new();
    for (k, v) in &sel.match_labels {
        parts.push(format!("{k}={v}"));
    }
    for expr in &sel.match_expressions {
        match expr.operator.as_str() {
            "In" => {
                if expr.values.is_empty() {
                    bail!("matchExpressions In requires values");
                }
                parts.push(format!("{} in ({})", expr.key, expr.values.join(",")));
            }
            "NotIn" => {
                if expr.values.is_empty() {
                    bail!("matchExpressions NotIn requires values");
                }
                parts.push(format!("{} notin ({})", expr.key, expr.values.join(",")));
            }
            "Exists" => {
                parts.push(expr.key.clone());
            }
            "DoesNotExist" => {
                parts.push(format!("!{}", expr.key));
            }
            op => bail!("unsupported matchExpression operator: {}", op),
        }
    }
    if parts.is_empty() {
        bail!("label selector is empty");
    }
    Ok(parts.join(","))
}

fn is_ready(pod: &PodListItem) -> bool {
    if pod
        .status
        .as_ref()
        .and_then(|s| s.phase.as_ref())
        .map(|p| p == "Running")
        != Some(true)
    {
        return false;
    }
    if let Some(conds) = pod.status.as_ref().and_then(|s| s.conditions.as_ref()) {
        return conds
            .iter()
            .any(|c| c.type_name == "Ready" && c.status == "True");
    }
    false
}

fn is_running(pod: &PodListItem) -> bool {
    pod.status
        .as_ref()
        .and_then(|s| s.phase.as_ref())
        .map(|p| p == "Running")
        .unwrap_or(false)
}

async fn list_resources(context: Option<&str>, namespace: &str, kind: &str) -> Result<Vec<String>> {
    match kind {
        "pod" => {
            let output = kubectl_base(context)
                .args(["get", "pods", "-n", namespace, "-o", "json"])
                .output()
                .await
                .context("failed to list pods")?;
            if !output.status.success() {
                bail!(
                    "kubectl get pods failed: {}",
                    String::from_utf8_lossy(&output.stderr).trim()
                );
            }
            let pods: PodList =
                serde_json::from_slice(&output.stdout).context("failed to parse pod list")?;
            Ok(pods
                .items
                .into_iter()
                .filter(is_ready)
                .map(|p| p.metadata.name)
                .collect())
        }
        "deployment" => {
            let output = kubectl_base(context)
                .args(["get", "deployments", "-n", namespace, "-o", "json"])
                .output()
                .await
                .context("failed to list deployments")?;
            if !output.status.success() {
                bail!(
                    "kubectl get deployments failed: {}",
                    String::from_utf8_lossy(&output.stderr).trim()
                );
            }
            let list: DeploymentList = serde_json::from_slice(&output.stdout)
                .context("failed to parse deployment list")?;
            Ok(list
                .items
                .into_iter()
                .filter(|d| {
                    if let Some(status) = &d.status {
                        status
                            .available_replicas
                            .unwrap_or(0)
                            .saturating_add(status.ready_replicas.unwrap_or(0))
                            > 0
                    } else {
                        false
                    }
                })
                .map(|d| d.metadata.name)
                .collect())
        }
        "job" => {
            let output = kubectl_base(context)
                .args(["get", "jobs", "-n", namespace, "-o", "json"])
                .output()
                .await
                .context("failed to list jobs")?;
            if !output.status.success() {
                bail!(
                    "kubectl get jobs failed: {}",
                    String::from_utf8_lossy(&output.stderr).trim()
                );
            }
            let list: JobList =
                serde_json::from_slice(&output.stdout).context("failed to parse job list")?;
            Ok(list
                .items
                .into_iter()
                .filter(|j| {
                    if let Some(status) = &j.status {
                        status.succeeded.unwrap_or(0) > 0
                            || status.ready.unwrap_or(0) > 0
                            || status.active.unwrap_or(0) > 0
                    } else {
                        false
                    }
                })
                .map(|j| j.metadata.name)
                .collect())
        }
        _ => Ok(Vec::new()),
    }
}

pub async fn exec_capture(
    context: Option<&str>,
    namespace: &str,
    pod: &str,
    container: &str,
    command: &[&str],
) -> Result<String> {
    let output = exec(context, namespace, pod, container, command, None).await?;
    if !output.status.success() {
        bail!(
            "kubectl exec failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

pub async fn exec_capture_optional(
    context: Option<&str>,
    namespace: &str,
    pod: &str,
    container: &str,
    command: &[&str],
) -> Result<Option<String>> {
    let output = exec(context, namespace, pod, container, command, None).await?;
    if !output.status.success() {
        return Ok(None);
    }
    Ok(Some(
        String::from_utf8_lossy(&output.stdout).trim().to_string(),
    ))
}

pub async fn exec_with_input(
    context: Option<&str>,
    namespace: &str,
    pod: &str,
    container: &str,
    command: &[&str],
    input: &[u8],
) -> Result<String> {
    let mut cmd = kubectl_base(context);
    cmd.args(["exec", "-i", "-n", namespace, pod, "-c", container, "--"]);
    cmd.args(command);
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());
    cmd.stdin(Stdio::piped());

    let mut child = cmd.spawn().context("failed to spawn kubectl exec")?;

    let mut input_err = None;
    if let Some(mut stdin) = child.stdin.take() {
        if let Err(err) = stdin.write_all(input).await {
            input_err = Some(err);
        }
    }

    let output = child
        .wait_with_output()
        .await
        .context("failed to wait for kubectl exec")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        if let Some(err) = input_err {
            bail!("kubectl exec failed (stdin error: {}): {}", err, stderr);
        } else {
            bail!("kubectl exec failed: {}", stderr);
        }
    }

    if let Some(err) = input_err {
        bail!("kubectl exec stdin error: {}", err);
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

async fn exec(
    context: Option<&str>,
    namespace: &str,
    pod: &str,
    container: &str,
    command: &[&str],
    input: Option<&[u8]>,
) -> Result<Output> {
    let mut cmd = kubectl_base(context);
    cmd.args(["exec", "-n", namespace, pod, "-c", container, "--"]);
    cmd.args(command);
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());
    if input.is_some() {
        cmd.stdin(Stdio::piped());
    }

    let mut child = cmd.spawn().context("failed to spawn kubectl exec")?;
    if let Some(data) = input {
        if let Some(mut stdin) = child.stdin.take() {
            stdin
                .write_all(data)
                .await
                .context("failed to write to kubectl exec stdin")?;
        }
    }

    let output = child
        .wait_with_output()
        .await
        .context("failed to wait for kubectl exec")?;
    Ok(output)
}
