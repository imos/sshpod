use crate::proxy;
use anyhow::Result;
use clap::{Args, Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "sshpod",
    version,
    about = "ProxyCommand helper for ssh/scp/sftp to Kubernetes Pods"
)]
pub struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// ProxyCommand entry point
    Proxy(ProxyArgs),
}

#[derive(Args, Debug, Clone)]
pub struct ProxyArgs {
    /// Target host (e.g. api-xxxx.ns.sshpod)
    #[arg(long)]
    pub host: String,
    /// SSH login user (defaults to local user)
    #[arg(long)]
    pub user: Option<String>,
    /// OpenSSH-supplied port (unused but accepted for compatibility)
    #[arg(long)]
    pub port: Option<u16>,
}

pub async fn run() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Proxy(args) => proxy::run(args).await?,
    }
    Ok(())
}
