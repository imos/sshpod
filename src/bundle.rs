use crate::{embedded, kubectl};
use anyhow::{bail, Context, Result};
use std::borrow::Cow;
use std::collections::HashSet;
use std::env;
use std::path::PathBuf;

pub const BUNDLE_VERSION: &str = concat!(env!("CARGO_PKG_VERSION"), "+bundle1");

pub async fn detect_remote_arch(
    context: Option<&str>,
    namespace: &str,
    pod: &str,
    container: &str,
) -> Result<String> {
    let machine = kubectl::exec_capture(context, namespace, pod, container, &["uname", "-m"])
        .await
        .context("failed to detect remote arch via uname -m")?;
    let arch = match machine.trim() {
        "x86_64" | "amd64" => "linux/amd64",
        "aarch64" | "arm64" => "linux/arm64",
        other => {
            bail!("unsupported remote architecture: {}", other);
        }
    };
    Ok(arch.to_string())
}

pub async fn ensure_bundle(
    context: Option<&str>,
    namespace: &str,
    pod: &str,
    container: &str,
    base: &str,
    arch: &str,
) -> Result<()> {
    let version_path = format!("{}/bundle/VERSION", base);
    let arch_path = format!("{}/bundle/ARCH", base);
    let remote_version =
        kubectl::exec_capture_optional(context, namespace, pod, container, &["cat", &version_path])
            .await?;
    let remote_arch =
        kubectl::exec_capture_optional(context, namespace, pod, container, &["cat", &arch_path])
            .await?;

    if remote_version.as_deref() == Some(BUNDLE_VERSION) && remote_arch.as_deref() == Some(arch) {
        return Ok(());
    }

    let bundle_data = if let Some(data) = embedded::get_bundle(arch) {
        Cow::from(data)
    } else {
        let bundle_path = locate_bundle(arch)?;
        Cow::from(
            tokio::fs::read(&bundle_path)
                .await
                .with_context(|| format!("failed to read bundle {}", bundle_path.display()))?,
        )
    };

    let install = format!("umask 077; mkdir -p \"{base}/bundle\"; tar xJf - -C \"{base}/bundle\"");
    kubectl::exec_with_input(
        context,
        namespace,
        pod,
        container,
        &["sh", "-c", &install],
        &bundle_data,
    )
    .await
    .with_context(|| format!("failed to install bundle into {}", base))?;

    Ok(())
}

fn locate_bundle(arch: &str) -> Result<PathBuf> {
    let filename = format!("openssh-bundle-{arch}.tar.xz");
    let mut candidates = Vec::new();
    let mut seen = HashSet::new();

    candidates.push(PathBuf::from(&filename));
    candidates.push(PathBuf::from("bundles").join(&filename));
    if let Ok(exe) = env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.join(&filename));
            candidates.push(dir.join("bundles").join(&filename));
            if let Some(root) = dir.parent() {
                candidates.push(root.join("bundles").join(&filename));
            }
        }
    }

    for candidate in candidates.into_iter().filter(|p| seen.insert(p.clone())) {
        if candidate.exists() {
            return Ok(candidate);
        }
    }

    bail!(
        "bundle file {} not found; place it alongside the binary or in ./bundles",
        filename
    );
}
