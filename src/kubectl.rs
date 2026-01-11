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
struct Deployment {
    spec: DeploymentSpec,
}

#[derive(Deserialize)]
struct DeploymentSpec {
    selector: LabelSelector,
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

fn kubectl_base(context: Option<&str>) -> Command {
    let mut cmd = Command::new("kubectl");
    if let Some(ctx) = context {
        cmd.arg("--context").arg(ctx);
    }
    cmd
}

pub async fn get_pod_info(context: Option<&str>, namespace: &str, pod: &str) -> Result<PodInfo> {
    let output = kubectl_base(context)
        .args(["get", "pod", pod, "-n", namespace, "-o", "json"])
        .output()
        .await
        .context("failed to run kubectl get pod")?;

    if !output.status.success() {
        bail!(
            "kubectl get pod failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
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
        bail!(
            "kubectl get deployment failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let deploy: Deployment =
        serde_json::from_slice(&output.stdout).context("failed to parse deployment json")?;
    if deploy.spec.selector.match_labels.is_empty() {
        bail!("deployment selector has no matchLabels; cannot select a pod");
    }
    let selector = to_selector(&deploy.spec.selector.match_labels)?;
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
        bail!(
            "kubectl get job failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let job_spec: Job =
        serde_json::from_slice(&output.stdout).context("failed to parse job json")?;
    let mut labels = if let Some(selector) = job_spec.spec.selector {
        selector.match_labels
    } else {
        HashMap::new()
    };
    if labels.is_empty() {
        if let Some(meta) = job_spec.spec.template.metadata {
            labels = meta.labels;
        }
    }
    if labels.is_empty() {
        labels.insert("job-name".to_string(), job.to_string());
    }
    let selector = to_selector(&labels)?;
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

fn to_selector(labels: &HashMap<String, String>) -> Result<String> {
    if labels.is_empty() {
        bail!("label selector is empty");
    }
    let mut parts = Vec::new();
    for (k, v) in labels {
        parts.push(format!("{k}={v}"));
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
