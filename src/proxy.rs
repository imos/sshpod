use crate::bundle;
use crate::cli::ProxyArgs;
use crate::hostspec::{self, Target};
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

    let namespace = host.namespace.clone();
    let context = host.context.as_deref();

    let pod_name = match &host.target {
        Target::Pod(pod) => pod.clone(),
        Target::Deployment(dep) => kubectl::choose_pod_for_deployment(context, &namespace, dep)
            .await
            .with_context(|| format!("failed to select pod from deployment `{}`", dep))?,
        Target::Job(job) => kubectl::choose_pod_for_job(context, &namespace, job)
            .await
            .with_context(|| format!("failed to select pod from job `{}`", job))?,
    };
    eprintln!(
        "resolved pod: {} (namespace={}, context={})",
        pod_name,
        namespace,
        context.unwrap_or("<default>")
    );

    let pod_info = kubectl::get_pod_info(context, &namespace, &pod_name)
        .await
        .with_context(|| format!("failed to inspect pod {}.{}", pod_name, namespace))?;

    let container = match host.container.as_ref() {
        Some(c) => {
            if pod_info.containers.iter().any(|name| name == c) {
                c.clone()
            } else {
                bail!("container `{}` not found in pod {}", c, pod_name);
            }
        }
        None => {
            if pod_info.containers.len() == 1 {
                pod_info.containers[0].clone()
            } else {
                bail!("This Pod has multiple containers. Use container--<container>.pod--<pod>.namespace--<namespace>[.context--<context>].sshpod to specify the target container.");
            }
        }
    };

    let base = format!("/tmp/sshpod/{}/{}", pod_info.uid, container);

    let local_key = keys::ensure_local_key()
        .await
        .context("failed to ensure ~/.cache/sshpod/id_ed25519 exists")?;

    remote::try_acquire_lock(context, &namespace, &pod_name, &container, &base).await;
    remote::assert_login_user_allowed(context, &namespace, &pod_name, &container, &login_user)
        .await?;

    let arch = bundle::detect_remote_arch(context, &namespace, &pod_name, &container)
        .await
        .context("failed to detect remote arch")?;
    eprintln!("remote architecture: {}", arch);
    bundle::ensure_bundle(context, &namespace, &pod_name, &container, &base, &arch).await?;
    eprintln!("sshd bundle ready for pod {}", pod_name);

    let remote_port = remote::ensure_sshd_running(
        context,
        &namespace,
        &pod_name,
        &container,
        &base,
        &login_user,
        &local_key.public_key,
    )
    .await?;

    let (mut forward, local_port) =
        PortForward::start(context, &namespace, &pod_name, remote_port).await?;

    let stream = TcpStream::connect(("127.0.0.1", local_port))
        .await
        .context("failed to connect to forwarded sshd port")?;

    let pump_result = proxy_io::pump(stream).await;
    let stop_result = forward.stop().await;

    pump_result?;
    stop_result?;
    Ok(())
}
