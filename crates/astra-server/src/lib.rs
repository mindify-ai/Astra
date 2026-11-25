//! The Astra SaaS backend: identity, sync relay (per-workspace op log, replay, fan-out, command routing),
//! presence relay and node registry. It never stores or runs agents and never sees agent keys.
//!
//! Runtime contract (env): `PORT` (default 8080) or `ASTRA_BIND`; `DATABASE_URL` (unset → in-memory);
//! `ASTRA_DB_MAX_CONNECTIONS` (10); `ASTRA_JWT_SECRET` (required unless `ASTRA_DEV_AUTH=1`);
//! `GITHUB_CLIENT_ID` / `GITLAB_CLIENT_ID`; `ASTRA_GITHUB_OAUTH_BASE` / `ASTRA_GITHUB_API_BASE` /
//! `ASTRA_GITLAB_BASE`; `ASTRA_ALLOWED_ORIGINS` (the desktop app's origins are always allowed); `ASTRA_RATE_LIMIT_PER_MIN`; `RUST_LOG`; `ASTRA_LOG_FORMAT=json`.

pub mod auth;
pub mod pg;
pub mod ratelimit;
pub mod ws;

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use astra_protocol::HlcClock;
use astra_sync::{Hub, MemoryOpLog, OpLog};
use axum::Router;
use axum::extract::State;
use axum::http::{HeaderValue, Method, StatusCode, header};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use tokio::sync::watch;
use tower_http::cors::{AllowOrigin, CorsLayer};
use tower_http::trace::TraceLayer;

use crate::auth::{Directory, Jwt, JwtAuthorizer, Providers};

/// Development JWT secret used only with `ASTRA_DEV_AUTH=1` and no `ASTRA_JWT_SECRET`.
pub const DEV_JWT_SECRET: &str = "astra-dev-secret-do-not-use-in-production";

/// Server configuration.
#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub bind: SocketAddr,
    pub database_url: Option<String>,
    pub db_max_connections: u32,
    pub jwt_secret: Option<String>,
    pub dev_auth: bool,
    pub providers: Providers,
    /// `None` = no CORS origins (or `*` in dev auth mode); `Some(list)` = exactly these.
    pub allowed_origins: Option<Vec<String>>,
    pub rate_limit_per_min: u32,
    /// WebSocket ping interval.
    pub ping_interval: Duration,
}

impl Default for ServerConfig {
    fn default() -> Self {
        ServerConfig {
            bind: SocketAddr::from(([0, 0, 0, 0], 8080)),
            database_url: None,
            db_max_connections: 10,
            jwt_secret: None,
            dev_auth: false,
            providers: Providers::default(),
            allowed_origins: None,
            rate_limit_per_min: 600,
            ping_interval: Duration::from_secs(25),
        }
    }
}

/// Startup errors.
#[derive(Debug, thiserror::Error)]
pub enum StartupError {
    #[error("ASTRA_JWT_SECRET is required (set ASTRA_DEV_AUTH=1 for local development)")]
    MissingJwtSecret,
    #[error("database: {0}")]
    Database(#[from] sqlx::Error),
    #[error("migrations: {0}")]
    Migrate(#[from] sqlx::migrate::MigrateError),
    #[error("http client: {0}")]
    Http(String),
    #[error("jwt: {0}")]
    Jwt(#[from] jsonwebtoken::errors::Error),
}

/// Shared handler state.
#[derive(Clone)]
pub struct AppState {
    pub config: Arc<ServerConfig>,
    pub hub: Arc<Hub>,
    pub jwt: Jwt,
    pub db: Option<sqlx::PgPool>,
    pub directory: Arc<Directory>,
    pub clock: Arc<Mutex<HlcClock>>,
    pub http: reqwest::Client,
    pub shutdown: watch::Receiver<bool>,
    pub limiter: Arc<ratelimit::Limiter>,
}

/// Build the state: connect + migrate Postgres (or fall back to memory), set up auth.
pub async fn build_state(
    config: ServerConfig,
    shutdown: watch::Receiver<bool>,
) -> Result<AppState, StartupError> {
    let secret = match (&config.jwt_secret, config.dev_auth) {
        (Some(s), _) if !s.is_empty() => s.clone(),
        (_, true) => {
            tracing::warn!(
                "ASTRA_DEV_AUTH=1 without ASTRA_JWT_SECRET: using the development secret"
            );
            DEV_JWT_SECRET.to_string()
        }
        _ => return Err(StartupError::MissingJwtSecret),
    };
    let jwt = Jwt::new(secret.as_bytes())?;
    let (log, db, directory): (Arc<dyn OpLog>, Option<sqlx::PgPool>, Directory) =
        match &config.database_url {
            Some(url) if !url.is_empty() => {
                let pool = sqlx::postgres::PgPoolOptions::new()
                    .max_connections(config.db_max_connections)
                    .acquire_timeout(Duration::from_secs(10))
                    .connect(url)
                    .await?;
                pg::migrate(&pool).await?;
                tracing::info!("using Postgres");
                (
                    Arc::new(pg::PgOpLog::new(pool.clone())),
                    Some(pool.clone()),
                    Directory::Postgres(pool),
                )
            }
            _ => {
                tracing::warn!(
                    "DATABASE_URL is not set: using the in-memory backend (data is lost on restart)"
                );
                (
                    Arc::new(MemoryOpLog::new()),
                    None,
                    Directory::Memory(Mutex::new(Default::default())),
                )
            }
        };
    let hub = Hub::new(log, Arc::new(JwtAuthorizer { jwt: jwt.clone() }));
    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|e| StartupError::Http(e.to_string()))?;
    let limiter = Arc::new(ratelimit::Limiter::new(config.rate_limit_per_min));
    Ok(AppState {
        config: Arc::new(config),
        hub,
        jwt,
        db,
        directory: Arc::new(directory),
        clock: Arc::new(auth::server_clock()),
        http,
        shutdown,
        limiter,
    })
}

async fn healthz() -> &'static str {
    "ok"
}

async fn readyz(State(state): State<AppState>) -> impl IntoResponse {
    match &state.db {
        None => (StatusCode::OK, "ok"),
        Some(pool) => match sqlx::query("SELECT 1").execute(pool).await {
            Ok(_) => (StatusCode::OK, "ok"),
            Err(e) => {
                tracing::warn!(error = %e, "readiness check failed");
                (StatusCode::SERVICE_UNAVAILABLE, "database unavailable")
            }
        },
    }
}

/// Origins of the desktop app's webview (macOS/Linux, Windows, Windows with `useHttpsScheme`). The desktop app
/// signs in through the same device-flow endpoints as the web app, so these are always allowed.
pub const DESKTOP_ORIGINS: [&str; 3] = [
    "tauri://localhost",
    "http://tauri.localhost",
    "https://tauri.localhost",
];

fn cors(config: &ServerConfig) -> CorsLayer {
    let base = CorsLayer::new()
        .allow_methods([Method::GET, Method::POST, Method::OPTIONS])
        .allow_headers([header::AUTHORIZATION, header::CONTENT_TYPE]);
    let desktop = DESKTOP_ORIGINS.iter().map(|o| HeaderValue::from_static(o));
    match &config.allowed_origins {
        Some(list) => {
            let origins: Vec<HeaderValue> = list
                .iter()
                .filter_map(|o| HeaderValue::from_str(o.trim()).ok())
                .chain(desktop)
                .collect();
            base.allow_origin(AllowOrigin::list(origins))
        }
        None if config.dev_auth => base.allow_origin(AllowOrigin::any()),
        None => base.allow_origin(AllowOrigin::list(desktop)),
    }
}

/// The HTTP router.
pub fn router(state: AppState) -> Router {
    let limited = Router::new()
        .route("/v1/sync", get(ws::sync_handler))
        .route("/v1/auth/dev-token", post(auth::dev_token))
        .route("/v1/auth/device/start", post(auth::device_start))
        .route("/v1/auth/device/poll", post(auth::device_poll))
        .route("/v1/me", get(auth::me))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            ratelimit::middleware,
        ));
    Router::new()
        .route("/healthz", get(healthz))
        .route("/readyz", get(readyz))
        .merge(limited)
        .layer(cors(&state.config))
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

/// Serve on `listener` until `shutdown` flips to true; open WebSockets get a normal close frame.
pub async fn serve(
    listener: tokio::net::TcpListener,
    state: AppState,
    mut shutdown: watch::Receiver<bool>,
) -> std::io::Result<()> {
    let app = router(state).into_make_service_with_connect_info::<SocketAddr>();
    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            while !*shutdown.borrow() {
                if shutdown.changed().await.is_err() {
                    break;
                }
            }
        })
        .await
}
