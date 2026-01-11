use crate::bundle;
use crate::cli::ProxyArgs;
use crate::hostspec;
use crate::keys;
use crate::kubectl;
use crate::port_forward::PortForward;
use crate::proxy_io;
use crate::remote;
use anyhow::{bail, Context, Result};
use tokio::net::TcpStream;

pub async fn run(args: ProxyArgs) -> Result<()> {
    let host = hostspec::parse(&args.host).context("failed to parse hostspec")?;
    let login_user = args
        .user
        .filter(|u| !u.is_empty())
        .unwrap_or_else(whoami::username);

    let namespace = kubectl::resolve_namespace(&host.namespace_hint)
        .await
        .with_context(|| {
            format!(
                "failed to resolve namespace or context `{}`",
                host.namespace_hint
            )
        })?;

    let pod_info = kubectl::get_pod_info(&namespace, &host.pod)
        .await
        .with_context(|| format!("failed to inspect pod {}.{}", host.pod, namespace))?;

    let container = match host.container.as_ref() {
        Some(c) => {
            if pod_info.containers.iter().any(|name| name == c) {
                c.clone()
            } else {
                bail!("container `{}` not found in pod {}", c, host.pod);
            }
        }
        None => {
            if pod_info.containers.len() == 1 {
                pod_info.containers[0].clone()
            } else {
                bail!("This Pod has multiple containers. Use <container>.<pod>.<namespace>.sshpod to specify the target container.");
            }
        }
    };

    let base = format!("/tmp/sshpod/{}/{}", pod_info.uid, container);

    let local_key = keys::ensure_local_key()
        .await
        .context("failed to ensure ~/.cache/sshpod/id_ed25519 exists")?;

    remote::try_acquire_lock(&namespace, &host.pod, &container, &base).await;
    remote::assert_login_user_allowed(&namespace, &host.pod, &container, &login_user).await?;

    let arch = bundle::detect_remote_arch(&namespace, &host.pod, &container)
        .await
        .context("failed to detect remote arch")?;
    bundle::ensure_bundle(&namespace, &host.pod, &container, &base, &arch).await?;

    let remote_port = remote::ensure_sshd_running(
        &namespace,
        &host.pod,
        &container,
        &base,
        &login_user,
        &local_key.public_key,
    )
    .await?;

    let (mut forward, local_port) = PortForward::start(&namespace, &host.pod, remote_port).await?;

    let stream = TcpStream::connect(("127.0.0.1", local_port))
        .await
        .context("failed to connect to forwarded sshd port")?;

    let pump_result = proxy_io::pump(stream).await;
    let stop_result = forward.stop().await;

    pump_result?;
    stop_result?;
    Ok(())
}
