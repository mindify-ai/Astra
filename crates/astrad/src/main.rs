//! `astrad`: headless Astra runner node and LAN host.

use std::net::SocketAddr;
use std::path::PathBuf;

use anyhow::Context;
use astra_node::{Node, NodeConfig, RelayConfig};
use astrad::pairing::{LanFiles, ServerInfo, local_ip, pair_uri, qr_text};
use clap::{Args, Parser, Subcommand};
use tracing_subscriber::EnvFilter;

#[derive(Parser, Debug)]
#[command(
    name = "astrad",
    version,
    about = "Headless Astra runner node",
    args_conflicts_with_subcommands = true
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Cmd>,
    #[command(flatten)]
    run: RunArgs,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Run the node (default).
    Run(RunArgs),
    /// Print a one-time pairing code and QR for a phone on the LAN.
    Pair(PairArgs),
}

#[derive(Args, Debug, Clone)]
struct RunArgs {
    /// Node data directory (SQLite store, worktrees, LAN keys).
    #[arg(long, env = "ASTRA_DATA_DIR", default_value = "./astra-data")]
    data_dir: PathBuf,
    /// This runner node's id (commands addressed to it are executed here).
    #[arg(long, env = "ASTRA_NODE_ID", default_value = "n-astrad")]
    node_id: String,
    /// Workspace id.
    #[arg(long, env = "ASTRA_WORKSPACE", default_value = "ws-local")]
    workspace: String,
    /// Local person id (defaults to the node id).
    #[arg(long, env = "ASTRA_ME")]
    me: Option<String>,
    /// Sync relay WebSocket URL, e.g. wss://astra.example.com/v1/sync.
    #[arg(long, env = "ASTRA_RELAY_URL")]
    relay_url: Option<String>,
    /// Session token for the relay.
    #[arg(long, env = "ASTRA_TOKEN", hide_env_values = true)]
    token: Option<String>,
    /// Serve LAN sync (Noise WebSocket at /v1/lan) on this address, e.g. 0.0.0.0:7420.
    #[arg(long, env = "ASTRA_SERVE_LAN")]
    serve_lan: Option<SocketAddr>,
    /// Seed document loaded when the store is empty.
    #[arg(long)]
    seed: Option<PathBuf>,
    /// Do not run agents (sync only).
    #[arg(long)]
    no_runtime: bool,
    /// Do not advertise over mDNS.
    #[arg(long)]
    no_mdns: bool,
}

#[derive(Args, Debug)]
struct PairArgs {
    #[arg(long, env = "ASTRA_DATA_DIR", default_value = "./astra-data")]
    data_dir: PathBuf,
    /// Host to put in the QR (default: the address recorded by the running LAN server, else this machine's LAN IP).
    #[arg(long)]
    host: Option<String>,
    /// Port to put in the QR (default: the running LAN server's port).
    #[arg(long)]
    port: Option<u16>,
    /// Code lifetime in seconds.
    #[arg(long, default_value_t = 600)]
    ttl_secs: i64,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();
    let cli = Cli::parse();
    match cli.command {
        Some(Cmd::Pair(p)) => pair(p),
        Some(Cmd::Run(r)) => run(r).await,
        None => run(cli.run).await,
    }
}

fn pair(args: PairArgs) -> anyhow::Result<()> {
    let files = LanFiles::new(&args.data_dir);
    let key = files.key().context("node key")?;
    let info = files.server_info();
    let host = args
        .host
        .or_else(|| info.as_ref().map(|i| i.host.clone()))
        .or_else(|| local_ip().map(|ip| ip.to_string()))
        .unwrap_or_else(|| "127.0.0.1".into());
    let port = args.port.or(info.as_ref().map(|i| i.port)).unwrap_or(7420);
    let code = files
        .create_code(args.ttl_secs)
        .context("write pairing code")?;
    let fp = astra_node::noise::fingerprint(&key.public);
    let uri = pair_uri(&host, port, &code, &fp);
    println!(
        "{}",
        qr_text(&uri).map_err(|e| anyhow::anyhow!("qr: {e:?}"))?
    );
    println!(
        "Pairing code: {code}  (valid for {} min)",
        args.ttl_secs / 60
    );
    println!("{uri}");
    println!("Node key fingerprint: {fp}");
    Ok(())
}

async fn run(args: RunArgs) -> anyhow::Result<()> {
    let relay = match (&args.relay_url, &args.token) {
        (Some(url), Some(token)) => Some(RelayConfig {
            url: url.clone(),
            token: token.clone(),
        }),
        (Some(_), None) => anyhow::bail!("--relay-url needs --token"),
        _ => None,
    };
    let mut config = NodeConfig::new(
        &args.data_dir,
        &args.node_id,
        &args.workspace,
        args.me.clone().unwrap_or_else(|| args.node_id.clone()),
    );
    config.runtime = !args.no_runtime;
    config.relay = relay;
    config.seed_if_empty = args.seed.clone();
    let node = Node::open(config).await.context("open node")?;
    tracing::info!(node = %args.node_id, workspace = %args.workspace, data_dir = %args.data_dir.display(), "astrad running");

    let mut _mdns = None;
    if let Some(addr) = args.serve_lan {
        let files = LanFiles::new(&args.data_dir);
        let key = files.key().context("node key")?;
        let fp = astra_node::noise::fingerprint(&key.public);
        let listener = tokio::net::TcpListener::bind(addr)
            .await
            .with_context(|| format!("bind {addr}"))?;
        let bound = listener.local_addr()?;
        let ip = if bound.ip().is_unspecified() {
            local_ip().unwrap_or(bound.ip())
        } else {
            bound.ip()
        };
        files.write_server_info(&ServerInfo {
            host: ip.to_string(),
            port: bound.port(),
        })?;
        if !args.no_mdns {
            match astrad::lan::advertise(&args.node_id, &args.workspace, ip, bound.port(), &fp) {
                Ok(d) => _mdns = Some(d),
                Err(e) => tracing::warn!(error = %e, "mDNS advertisement failed"),
            }
        }
        tracing::info!(%bound, %fp, "LAN sync listening (run `astrad pair` to pair a phone)");
        let app = astrad::lan::router(astrad::lan::LanState::new(node.clone(), files, key));
        tokio::spawn(async move {
            if let Err(e) = axum::serve(listener, app).await {
                tracing::error!(error = %e, "LAN server stopped");
            }
        });
    }

    shutdown_signal().await;
    tracing::info!("shutting down");
    node.shutdown();
    if let Some(d) = _mdns {
        let _ = d.shutdown();
    }
    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let term = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut s) => {
                s.recv().await;
            }
            Err(_) => std::future::pending::<()>().await,
        }
    };
    #[cfg(not(unix))]
    let term = std::future::pending::<()>();
    tokio::select! {
        _ = ctrl_c => {},
        _ = term => {},
    }
}
