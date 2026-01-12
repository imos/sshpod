use crate::bundle;
use crate::cli::ProxyArgs;
use crate::hostspec::{self, Target};
use crate::keys;
use crate::kubectl;
use crate::port_forward::PortForward;
use crate::proxy_io;
use crate::remote;
use anyhow::{bail, Context, Result};
use log::info;
use std::io::Write;
use tokio::net::TcpStream;

fn init_logger(level_arg: &str) {
    let mut builder = env_logger::Builder::new();
    builder.format(|buf, record| writeln!(buf, "{}", record.args()));
    builder.parse_filters(level_arg);
    let _ = builder.try_init();
}

pub async fn run(args: ProxyArgs) -> Result<()> {
    init_logger(&args.log_level);
    let host = hostspec::parse(&args.host).context("failed to parse hostspec")?;
    let login_user = args
        .user
        .filter(|u| !u.is_empty())
        .unwrap_or_else(whoami::username);

    kubectl::ensure_context_exists(&host.context).await?;
    let namespace = if let Some(ns) = host.namespace.clone() {
        ns
    } else {
        kubectl::get_context_namespace(&host.context)
            .await?
            .unwrap_or_default()
    };
    let ns_str = namespace.as_str();
    let context_str = host.context.as_str();
    let context = Some(context_str);

    let pod_name = match &host.target {
        Target::Pod(pod) => pod.clone(),
        Target::Deployment(dep) => kubectl::choose_pod_for_deployment(context, ns_str, dep)
            .await
            .with_context(|| format!("failed to select pod from deployment `{}`", dep))?,
        Target::Job(job) => kubectl::choose_pod_for_job(context, ns_str, job)
            .await
            .with_context(|| format!("failed to select pod from job `{}`", job))?,
    };
    info!(
        "[sshpod] resolved pod: {} (namespace={}, context={})",
        pod_name, ns_str, context_str
    );

    let pod_info = kubectl::get_pod_info(context, ns_str, &pod_name)
        .await
        .with_context(|| format!("failed to inspect pod {}.{}", pod_name, ns_str))?;

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
    info!("[sshpod] resolved container: {}", container);

    let base = format!("/tmp/sshpod/{}/{}", pod_info.uid, container);

    let local_key = keys::ensure_key("id_ed25519")
        .await
        .context("failed to ensure ~/.cache/sshpod/id_ed25519 exists")?;
    let host_keys = keys::ensure_key("ssh_host_ed25519_key")
        .await
        .context("failed to create host keys")?;

    remote::try_acquire_lock(context, ns_str, &pod_name, &container, &base).await;
    remote::assert_login_user_allowed(context, ns_str, &pod_name, &container, &login_user).await?;

    let arch = bundle::detect_remote_arch(context, ns_str, &pod_name, &container)
        .await
        .context("failed to detect remote arch")?;
    info!("[sshpod] remote architecture: {}", arch);
    bundle::ensure_bundle(context, ns_str, &pod_name, &container, &base, &arch).await?;
    info!("[sshpod] sshd bundle ready for pod {}", pod_name);
    remote::install_host_keys(context, ns_str, &pod_name, &container, &base, &host_keys).await?;

    info!("[sshpod] starting/ensuring sshd in pod {}", pod_name);
    let remote_port = remote::ensure_sshd_running(
        context,
        ns_str,
        &pod_name,
        &container,
        &base,
        &login_user,
        &local_key.public,
    )
    .await?;
    info!(
        "[sshpod] sshd is listening on 127.0.0.1:{} (pod {})",
        remote_port, pod_name
    );

    info!(
        "[sshpod] starting port-forward to {}:{}",
        pod_name, remote_port
    );
    let (mut forward, local_port) =
        PortForward::start(context, ns_str, &pod_name, remote_port).await?;
    info!(
        "[sshpod] port-forward established: localhost:{} -> {}:{}",
        local_port, pod_name, remote_port
    );

    let stream = TcpStream::connect(("127.0.0.1", local_port))
        .await
        .context("failed to connect to forwarded sshd port")?;

    let pump_result = proxy_io::pump(stream).await;
    let stop_result = forward.stop().await;

    pump_result?;
    stop_result?;
    Ok(())
}
