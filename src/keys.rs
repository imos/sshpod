use anyhow::{Context, Result};
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use tokio::fs;
use tokio::process::Command;

pub struct LocalKey {
    pub public_key: String,
}

pub async fn ensure_local_key() -> Result<LocalKey> {
    let home = std::env::var("HOME").context("HOME is not set")?;
    let cache_dir = PathBuf::from(home).join(".cache/sshpod");
    fs::create_dir_all(&cache_dir)
        .await
        .with_context(|| format!("failed to create {}", cache_dir.display()))?;
    fs::set_permissions(&cache_dir, std::fs::Permissions::from_mode(0o700))
        .await
        .ok();

    let private_key = cache_dir.join("id_ed25519");
    let public_key = private_key.with_extension("pub");

    if !private_key.exists() || !public_key.exists() {
        let status = Command::new("ssh-keygen")
            .args([
                "-t",
                "ed25519",
                "-f",
                private_key.to_str().unwrap_or_default(),
                "-N",
                "",
            ])
            .status()
            .await
            .context("failed to spawn ssh-keygen")?;
        if !status.success() {
            anyhow::bail!("ssh-keygen failed with status {}", status);
        }
    }

    let public_key_contents = fs::read_to_string(&public_key)
        .await
        .with_context(|| format!("failed to read {}", public_key.display()))?;
    Ok(LocalKey {
        public_key: public_key_contents.trim().to_string(),
    })
}
