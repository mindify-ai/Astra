//! `astra-server`: the Astra SaaS backend. Configuration comes from the environment (see the crate docs).

use std::net::SocketAddr;
use std::time::Duration;

use anyhow::Context;
use astra_server::ServerConfig;
use astra_server::auth::Providers;
use clap::{ArgAction, Parser};
use tracing_subscriber::EnvFilter;

#[derive(Parser, Debug)]
#[command(
    name = "astra-server",
    version,
    about = "Astra sync relay and identity server"
)]
struct Cli {
    /// Full bind address (overrides PORT).
    #[arg(long, env = "ASTRA_BIND")]
    bind: Option<SocketAddr>,
    /// Port to listen on, on 0.0.0.0.
    #[arg(long, env = "PORT", default_value_t = 8080)]
    port: u16,
    /// Postgres URL; unset uses the in-memory backend.
    #[arg(long, env = "DATABASE_URL", hide_env_values = true)]
    database_url: Option<String>,
    #[arg(long, env = "ASTRA_DB_MAX_CONNECTIONS", default_value_t = 10)]
    db_max_connections: u32,
    #[arg(long, env = "ASTRA_JWT_SECRET", hide_env_values = true)]
    jwt_secret: Option<String>,
    /// Enable `POST /v1/auth/dev-token` and permissive CORS.
    /// Accepts `1`/`0`, `true`/`false`, `yes`/`no`, `on`/`off` from the environment.
    #[arg(long, env = "ASTRA_DEV_AUTH", action = ArgAction::SetTrue, value_parser = clap::builder::BoolishValueParser::new())]
    dev_auth: bool,
    #[arg(long, env = "GITHUB_CLIENT_ID")]
    github_client_id: Option<String>,
    #[arg(long, env = "GITLAB_CLIENT_ID")]
    gitlab_client_id: Option<String>,
    #[arg(
        long,
        env = "ASTRA_GITHUB_OAUTH_BASE",
        default_value = "https://github.com"
    )]
    github_oauth_base: String,
    #[arg(
        long,
        env = "ASTRA_GITHUB_API_BASE",
        default_value = "https://api.github.com"
    )]
    github_api_base: String,
    #[arg(long, env = "ASTRA_GITLAB_BASE", default_value = "https://gitlab.com")]
    gitlab_base: String,
    /// Comma-separated CORS origins.
    #[arg(long, env = "ASTRA_ALLOWED_ORIGINS")]
    allowed_origins: Option<String>,
    #[arg(long, env = "ASTRA_RATE_LIMIT_PER_MIN", default_value_t = 600)]
    rate_limit_per_min: u32,
    /// `json` for JSON logs (Cloud Logging).
    #[arg(long, env = "ASTRA_LOG_FORMAT")]
    log_format: Option<String>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,tower_http=info"));
    if cli.log_format.as_deref() == Some("json") {
        tracing_subscriber::fmt()
            .json()
            .with_env_filter(filter)
            .with_current_span(false)
            .init();
    } else {
        tracing_subscriber::fmt().with_env_filter(filter).init();
    }

    let config = ServerConfig {
        bind: cli
            .bind
            .unwrap_or_else(|| SocketAddr::from(([0, 0, 0, 0], cli.port))),
        database_url: cli.database_url,
        db_max_connections: cli.db_max_connections,
        jwt_secret: cli.jwt_secret,
        dev_auth: cli.dev_auth,
        providers: Providers {
            github_client_id: cli.github_client_id,
            gitlab_client_id: cli.gitlab_client_id,
            github_oauth_base: cli.github_oauth_base.trim_end_matches('/').into(),
            github_api_base: cli.github_api_base.trim_end_matches('/').into(),
            gitlab_base: cli.gitlab_base.trim_end_matches('/').into(),
        },
        allowed_origins: cli.allowed_origins.map(|s| {
            s.split(',')
                .map(|o| o.trim().to_string())
                .filter(|o| !o.is_empty())
                .collect()
        }),
        rate_limit_per_min: cli.rate_limit_per_min,
        ..ServerConfig::default()
    };
    let bind = config.bind;
    let (tx, rx) = tokio::sync::watch::channel(false);
    let state = astra_server::build_state(config, rx.clone())
        .await
        .context("startup failed")?;
    let listener = tokio::net::TcpListener::bind(bind)
        .await
        .with_context(|| format!("bind {bind}"))?;
    tracing::info!(%bind, "astra-server listening");

    tokio::spawn(async move {
        shutdown_signal().await;
        tracing::info!("shutting down");
        let _ = tx.send(true);
        // Cloud Run allows ~10 s after SIGTERM.
        tokio::time::sleep(Duration::from_secs(9)).await;
        tracing::warn!("forcing exit after grace period");
        std::process::exit(0);
    });
    astra_server::serve(listener, state, rx).await?;
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
