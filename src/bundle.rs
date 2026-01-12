use crate::{embedded, kubectl};
use anyhow::{bail, Context, Result};
use flate2::write::GzEncoder;
use flate2::Compression;
use std::borrow::Cow;
use std::collections::HashSet;
use std::env;
use std::io::Read;
use std::path::PathBuf;
use xz2::read::XzDecoder;

pub const BUNDLE_VERSION: &str = concat!(env!("CARGO_PKG_VERSION"), "+sshd1");

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

    let meta = format!(
        "printf '%s\\n' \"{BUNDLE_VERSION}\" > \"{base}/bundle/VERSION\"; \
         printf '%s\\n' \"{arch}\" > \"{base}/bundle/ARCH\"; \
         chmod 600 \"{base}/bundle/VERSION\" \"{base}/bundle/ARCH\";"
    );

    let install_xz = format!(
        "set -eu; umask 077; mkdir -p \"{base}/bundle\"; chmod 700 \"{base}\" \"{base}/bundle\"; \
         xz -dc > \"{base}/bundle/sshd\"; chmod 700 \"{base}/bundle/sshd\"; {meta}"
    );
    match kubectl::exec_with_input(
        context,
        namespace,
        pod,
        container,
        &["sh", "-c", &install_xz],
        &bundle_data,
    )
    .await
    {
        Ok(_) => return Ok(()),
        Err(first_err) => {
            let sshd_data =
                decompress_xz(&bundle_data).context("failed to decompress sshd xz locally")?;
            let mut gz = GzEncoder::new(Vec::new(), Compression::default());
            use std::io::Write;
            gz.write_all(&sshd_data).context("failed to gzip sshd")?;
            let gz_data = gz.finish().context("failed to finalize gzip")?;

            let install_gz = format!(
                "set -eu; umask 077; mkdir -p \"{base}/bundle\"; chmod 700 \"{base}\" \"{base}/bundle\"; \
                 gzip -dc > \"{base}/bundle/sshd\"; chmod 700 \"{base}/bundle/sshd\"; {meta}"
            );
            match kubectl::exec_with_input(
                context,
                namespace,
                pod,
                container,
                &["sh", "-c", &install_gz],
                &gz_data,
            )
            .await
            {
                Ok(_) => return Ok(()),
                Err(second_err) => {
                    let install_plain = format!(
                        "set -eu; umask 077; mkdir -p \"{base}/bundle\"; chmod 700 \"{base}\" \"{base}/bundle\"; \
                         cat > \"{base}/bundle/sshd\"; chmod 700 \"{base}/bundle/sshd\"; {meta}"
                    );
                    kubectl::exec_with_input(
                        context,
                        namespace,
                        pod,
                        container,
                        &["sh", "-c", &install_plain],
                        &sshd_data,
                    )
                    .await
                    .with_context(|| {
                        format!(
                            "failed to install bundle into {} (xz error: {}; gzip error: {})",
                            base, first_err, second_err
                        )
                    })?;
                }
            }
        }
    }

    Ok(())
}

fn locate_bundle(arch: &str) -> Result<PathBuf> {
    let filename = match arch {
        "linux/amd64" => "sshd_amd64.xz".to_string(),
        "linux/arm64" => "sshd_arm64.xz".to_string(),
        _ => format!("sshd_{}.xz", arch.replace('/', "_")),
    };
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

fn decompress_xz(data: &[u8]) -> Result<Vec<u8>> {
    let mut decoder = XzDecoder::new(data);
    let mut buf = Vec::new();
    decoder
        .read_to_end(&mut buf)
        .context("failed to decompress xz")?;
    Ok(buf)
}

#[cfg(test)]
mod tests {
    use super::decompress_xz;
    use std::io::Write;
    use xz2::write::XzEncoder;

    #[test]
    fn decompress_smoke() {
        let mut encoder = XzEncoder::new(Vec::new(), 6);
        encoder.write_all(b"hello world").unwrap();
        let data = encoder.finish().unwrap();
        let out = decompress_xz(&data).expect("decompress");
        assert_eq!(out, b"hello world");
    }
}
