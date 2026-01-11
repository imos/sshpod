use anyhow::{bail, Context, Result};
use serde::Deserialize;
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
struct ConfigView {
    contexts: Vec<NamedContext>,
}

#[derive(Deserialize)]
struct NamedContext {
    name: String,
    context: ContextEntry,
}

#[derive(Deserialize)]
struct ContextEntry {
    #[serde(default)]
    namespace: Option<String>,
}

pub async fn resolve_namespace(token: &str) -> Result<String> {
    if let Some(ns) = get_context_namespace(token).await? {
        return Ok(ns);
    }
    Ok(token.to_string())
}

pub async fn get_context_namespace(context: &str) -> Result<Option<String>> {
    let output = Command::new("kubectl")
        .args(["config", "view", "-o", "json"])
        .output()
        .await
        .context("failed to run kubectl config view")?;
    if !output.status.success() {
        bail!(
            "kubectl config view failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let view: ConfigView =
        serde_json::from_slice(&output.stdout).context("failed to parse kubectl config view")?;
    for ctx in view.contexts {
        if ctx.name == context {
            return Ok(ctx.context.namespace);
        }
    }
    Ok(None)
}

pub async fn get_pod_info(namespace: &str, pod: &str) -> Result<PodInfo> {
    let output = Command::new("kubectl")
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

pub async fn exec_capture(
    namespace: &str,
    pod: &str,
    container: &str,
    command: &[&str],
) -> Result<String> {
    let output = exec(namespace, pod, container, command, None).await?;
    if !output.status.success() {
        bail!(
            "kubectl exec failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

pub async fn exec_capture_optional(
    namespace: &str,
    pod: &str,
    container: &str,
    command: &[&str],
) -> Result<Option<String>> {
    let output = exec(namespace, pod, container, command, None).await?;
    if !output.status.success() {
        return Ok(None);
    }
    Ok(Some(
        String::from_utf8_lossy(&output.stdout).trim().to_string(),
    ))
}

pub async fn exec_with_input(
    namespace: &str,
    pod: &str,
    container: &str,
    command: &[&str],
    input: &[u8],
) -> Result<String> {
    let mut cmd = Command::new("kubectl");
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
    namespace: &str,
    pod: &str,
    container: &str,
    command: &[&str],
    input: Option<&[u8]>,
) -> Result<Output> {
    let mut cmd = Command::new("kubectl");
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
