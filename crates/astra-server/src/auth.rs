//! Authentication: HS256 session JWTs, the dev token endpoint, GitHub/GitLab OAuth device flow (proxied,
//! because those endpoints do not allow browser CORS), the GitHub redirect flow (the relay keeps the client
//! secret) and `GET /v1/me`.
//!
//! The provider access token is used once to read the profile and then discarded; nothing but the person's
//! id, name and provider login is stored.

use std::collections::HashMap;
use std::sync::Mutex;

use astra_protocol::{HlcClock, Op, OpBody, Person};
use axum::Json;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::AppState;

/// Session lifetime.
pub const SESSION_DAYS: i64 = 30;

/// JWT claims: `{sub: personId, ws: workspaceId, exp, iat}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Claims {
    pub sub: String,
    pub ws: String,
    pub exp: i64,
    pub iat: i64,
}

/// HS256 signer/verifier.
#[derive(Clone)]
pub struct Jwt {
    enc: EncodingKey,
    dec: DecodingKey,
}

impl Jwt {
    /// Keys from a shared secret.
    pub fn new(secret: &[u8]) -> Result<Self, jsonwebtoken::errors::Error> {
        Ok(Jwt {
            enc: EncodingKey::from_secret(secret)?,
            dec: DecodingKey::from_secret(secret)?,
        })
    }

    /// Issue a token for `person` in `workspace`.
    pub fn issue(
        &self,
        person: &str,
        workspace: &str,
    ) -> Result<(String, Claims), jsonwebtoken::errors::Error> {
        let now = chrono::Utc::now().timestamp();
        let claims = Claims {
            sub: person.into(),
            ws: workspace.into(),
            iat: now,
            exp: now + SESSION_DAYS * 86_400,
        };
        let token = jsonwebtoken::encode(&Header::new(Algorithm::HS256), &claims, &self.enc)?;
        Ok((token, claims))
    }

    /// Verify signature and expiry.
    pub fn verify(&self, token: &str) -> Result<Claims, jsonwebtoken::errors::Error> {
        let mut v = Validation::new(Algorithm::HS256);
        v.leeway = 30;
        v.set_required_spec_claims(&["exp", "sub"]);
        Ok(jsonwebtoken::decode::<Claims>(token, &self.dec, &v)?.claims)
    }
}

/// Authorizes sync `hello`s: a valid session token whose `ws` claim matches the workspace.
pub struct JwtAuthorizer {
    pub jwt: Jwt,
}

#[async_trait::async_trait]
impl astra_sync::Authorizer for JwtAuthorizer {
    async fn authorize(
        &self,
        workspace_id: &str,
        _node_id: &str,
        token: Option<&str>,
    ) -> astra_sync::Result<astra_sync::Principal> {
        let token =
            token.ok_or_else(|| astra_sync::SyncError::Unauthorized("missing token".into()))?;
        let claims = self
            .jwt
            .verify(token)
            .map_err(|e| astra_sync::SyncError::Unauthorized(e.to_string()))?;
        if claims.ws != workspace_id {
            return Err(astra_sync::SyncError::Unauthorized(
                "workspace mismatch".into(),
            ));
        }
        Ok(astra_sync::Principal {
            person: Some(claims.sub.clone()),
            subject: claims.sub,
        })
    }
}

/// Session returned to clients.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Session {
    pub token: String,
    pub workspace_id: String,
    pub person: Value,
    pub expires_at: String,
}

/// Initials: first letters of the first two words of `name`, else the first two characters.
pub fn initials(name: &str, login: &str) -> String {
    let words: Vec<&str> = name.split_whitespace().collect();
    let s: String = match words.as_slice() {
        [a, b, ..] => [a.chars().next(), b.chars().next()]
            .into_iter()
            .flatten()
            .collect(),
        [a] => a.chars().take(2).collect(),
        [] => login.chars().take(2).collect(),
    };
    s.to_uppercase()
}

/// Lowercase slug of `[a-z0-9-]`.
pub fn slug(s: &str) -> String {
    let mut out = String::new();
    for c in s.trim().to_lowercase().chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    let out = out.trim_matches('-').to_string();
    if out.is_empty() { "user".into() } else { out }
}

fn person_json(p: &Person) -> Value {
    serde_json::to_value(astra_protocol::Entity::Person(p.clone())).unwrap_or(Value::Null)
}

fn error(status: StatusCode, code: &str) -> Response {
    (status, Json(json!({"error": code}))).into_response()
}

/// People known to the server.
pub enum Directory {
    Memory(Mutex<HashMap<String, (Person, String)>>),
    Postgres(sqlx::PgPool),
}

impl Directory {
    /// Record a person; returns true when they were not known before.
    pub async fn upsert(
        &self,
        person: &Person,
        provider: &str,
        login: &str,
        workspace: &str,
    ) -> Result<bool, String> {
        match self {
            Directory::Memory(m) => {
                let mut m = m.lock().map_err(|_| "poisoned".to_string())?;
                let new = !m.contains_key(&person.id);
                m.insert(person.id.clone(), (person.clone(), workspace.to_string()));
                Ok(new)
            }
            Directory::Postgres(pool) => {
                let existed: Option<(String,)> =
                    sqlx::query_as("SELECT id FROM people WHERE id = $1")
                        .bind(&person.id)
                        .fetch_optional(pool)
                        .await
                        .map_err(|e| e.to_string())?;
                crate::pg::upsert_person(
                    pool,
                    &person.id,
                    &person.name,
                    provider,
                    login,
                    workspace,
                )
                .await
                .map_err(|e| e.to_string())?;
                Ok(existed.is_none())
            }
        }
    }

    /// Look a person up.
    pub async fn get(&self, id: &str) -> Option<Person> {
        match self {
            Directory::Memory(m) => m.lock().ok()?.get(id).map(|(p, _)| p.clone()),
            Directory::Postgres(pool) => {
                let row: Option<(String, String, Option<String>)> =
                    sqlx::query_as("SELECT id, name, provider_id FROM people WHERE id = $1")
                        .bind(id)
                        .fetch_optional(pool)
                        .await
                        .ok()?;
                row.map(|(id, name, login)| Person {
                    initials: initials(&name, login.as_deref().unwrap_or(&id)),
                    id,
                    name,
                })
            }
        }
    }
}

/// Sign `person` in to `workspace`: record them, emit their `put` op on first sign-in, issue a session.
pub async fn sign_in(
    state: &AppState,
    person: Person,
    provider: &str,
    login: &str,
    workspace: &str,
) -> Result<Session, String> {
    let new = state
        .directory
        .upsert(&person, provider, login, workspace)
        .await?;
    if new {
        let hlc = state
            .clock
            .lock()
            .map_err(|_| "poisoned".to_string())?
            .tick();
        let op = Op::new(hlc, "server", OpBody::put(person.clone()));
        state
            .hub
            .append_server_ops(workspace, &[op])
            .await
            .map_err(|e| e.to_string())?;
    }
    let (token, claims) = state
        .jwt
        .issue(&person.id, workspace)
        .map_err(|e| e.to_string())?;
    let expires_at = chrono::DateTime::from_timestamp(claims.exp, 0)
        .map(|d| d.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
        .unwrap_or_default();
    Ok(Session {
        token,
        workspace_id: workspace.into(),
        person: person_json(&person),
        expires_at,
    })
}

/// A server-side HLC for ops the server emits.
pub fn server_clock() -> Mutex<HlcClock> {
    Mutex::new(HlcClock::new("server"))
}

#[derive(Deserialize)]
pub struct DevTokenRequest {
    pub name: String,
}

/// `POST /v1/auth/dev-token` (only with `ASTRA_DEV_AUTH=1`).
pub async fn dev_token(
    State(state): State<AppState>,
    Json(req): Json<DevTokenRequest>,
) -> Response {
    if !state.config.dev_auth {
        return error(StatusCode::NOT_FOUND, "notFound");
    }
    let name = req.name.trim();
    let name = if name.is_empty() { "Dev" } else { name };
    let person = Person {
        id: format!("p-dev-{}", slug(name)),
        name: name.into(),
        initials: initials(name, name),
    };
    match sign_in(&state, person, "dev", &slug(name), "ws-dev").await {
        Ok(session) => Json(json!({"session": session})).into_response(),
        Err(e) => {
            tracing::error!(error = %e, "dev sign-in failed");
            error(StatusCode::INTERNAL_SERVER_ERROR, "internal")
        }
    }
}

/// `GET /v1/me`.
pub async fn me(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let token = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "));
    let Some(claims) = token.and_then(|t| state.jwt.verify(t).ok()) else {
        return error(StatusCode::UNAUTHORIZED, "unauthorized");
    };
    let person = state
        .directory
        .get(&claims.sub)
        .await
        .unwrap_or_else(|| Person {
            id: claims.sub.clone(),
            name: claims.sub.clone(),
            initials: initials("", &claims.sub),
        });
    Json(json!({"person": person_json(&person), "workspaceId": claims.ws})).into_response()
}

// ------------------------------------------------------------------------------------------ device flow

/// OAuth provider endpoints.
#[derive(Debug, Clone)]
pub struct Providers {
    pub github_client_id: Option<String>,
    /// The GitHub OAuth app's client secret, for the redirect flow (`GITHUB_CLIENT_SECRET`). Server-side only.
    pub github_client_secret: Option<String>,
    pub gitlab_client_id: Option<String>,
    /// `https://github.com` (override: `ASTRA_GITHUB_OAUTH_BASE`).
    pub github_oauth_base: String,
    /// `https://api.github.com` (override: `ASTRA_GITHUB_API_BASE`).
    pub github_api_base: String,
    /// `https://gitlab.com` (override: `ASTRA_GITLAB_BASE`).
    pub gitlab_base: String,
}

impl Default for Providers {
    fn default() -> Self {
        Providers {
            github_client_id: None,
            github_client_secret: None,
            gitlab_client_id: None,
            github_oauth_base: "https://github.com".into(),
            github_api_base: "https://api.github.com".into(),
            gitlab_base: "https://gitlab.com".into(),
        }
    }
}

/// A supported provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provider {
    Github,
    Gitlab,
}

impl Provider {
    fn parse(s: &str) -> Option<Provider> {
        match s {
            "github" => Some(Provider::Github),
            "gitlab" => Some(Provider::Gitlab),
            _ => None,
        }
    }
}

const DEVICE_GRANT: &str = "urn:ietf:params:oauth:grant-type:device_code";

impl Providers {
    fn client_id(&self, p: Provider) -> Option<&str> {
        match p {
            Provider::Github => self.github_client_id.as_deref(),
            Provider::Gitlab => self.gitlab_client_id.as_deref(),
        }
        .filter(|s| !s.is_empty())
    }

    /// The device-code request.
    pub fn device_code_request(
        &self,
        http: &reqwest::Client,
        p: Provider,
        client_id: &str,
    ) -> reqwest::Result<reqwest::Request> {
        let (url, scope) = match p {
            Provider::Github => (
                format!("{}/login/device/code", self.github_oauth_base),
                "read:user user:email",
            ),
            Provider::Gitlab => (
                format!("{}/oauth/authorize_device", self.gitlab_base),
                "read_user",
            ),
        };
        http.post(url)
            .header("accept", "application/json")
            .form(&[("client_id", client_id), ("scope", scope)])
            .build()
    }

    /// The token poll request.
    pub fn token_request(
        &self,
        http: &reqwest::Client,
        p: Provider,
        client_id: &str,
        device_code: &str,
    ) -> reqwest::Result<reqwest::Request> {
        let url = match p {
            Provider::Github => format!("{}/login/oauth/access_token", self.github_oauth_base),
            Provider::Gitlab => format!("{}/oauth/token", self.gitlab_base),
        };
        http.post(url)
            .header("accept", "application/json")
            .form(&[
                ("client_id", client_id),
                ("device_code", device_code),
                ("grant_type", DEVICE_GRANT),
            ])
            .build()
    }

    /// The profile request.
    pub fn user_request(
        &self,
        http: &reqwest::Client,
        p: Provider,
        access_token: &str,
    ) -> reqwest::Result<reqwest::Request> {
        let url = match p {
            Provider::Github => format!("{}/user", self.github_api_base),
            Provider::Gitlab => format!("{}/api/v4/user", self.gitlab_base),
        };
        http.get(url)
            .bearer_auth(access_token)
            .header("accept", "application/json")
            .header("user-agent", "astra-server")
            .build()
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceStartRequest {
    pub provider: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DevicePollRequest {
    pub provider: String,
    pub device_code: String,
}

/// A provider answer that means the server's OAuth app is set up wrong, not that sign-in failed: an unknown client
/// id (GitHub answers 404 `Not Found`), or an app without device flow enabled. These are `503`s with a code the
/// sign-in screen explains, and they are logged for whoever runs the relay.
fn misconfigured(status: u16, v: &Value) -> Option<Response> {
    let code = v["error"].as_str().unwrap_or_default();
    let reply = match code {
        "device_flow_disabled" => "deviceFlowDisabled",
        "incorrect_client_credentials" | "unauthorized_client" | "invalid_client" => "badClientId",
        _ if status == 404 => "badClientId",
        _ => return None,
    };
    tracing::error!(
        status,
        error = code,
        "the OAuth app is misconfigured: check GITHUB_CLIENT_ID and that device flow is enabled"
    );
    Some(error(StatusCode::SERVICE_UNAVAILABLE, reply))
}

async fn send_json(http: &reqwest::Client, req: reqwest::Request) -> Result<(u16, Value), String> {
    let resp = http
        .execute(req)
        .await
        .map_err(|e| e.without_url().to_string())?;
    let status = resp.status().as_u16();
    let v = resp.json::<Value>().await.unwrap_or(Value::Null);
    Ok((status, v))
}

/// `POST /v1/auth/device/start`.
pub async fn device_start(
    State(state): State<AppState>,
    Json(req): Json<DeviceStartRequest>,
) -> Response {
    let Some(p) = Provider::parse(&req.provider) else {
        return error(StatusCode::BAD_REQUEST, "badProvider");
    };
    let Some(client_id) = state.config.providers.client_id(p) else {
        return error(StatusCode::SERVICE_UNAVAILABLE, "notConfigured");
    };
    let r = match state
        .config
        .providers
        .device_code_request(&state.http, p, client_id)
    {
        Ok(r) => r,
        Err(_) => return error(StatusCode::INTERNAL_SERVER_ERROR, "internal"),
    };
    match send_json(&state.http, r).await {
        Ok((200, v)) if v["device_code"].is_string() => Json(json!({
            "deviceCode": v["device_code"],
            "userCode": v["user_code"],
            "verificationUri": v["verification_uri"],
            "verificationUriComplete": v.get("verification_uri_complete").cloned().unwrap_or(Value::Null),
            "interval": v["interval"].as_u64().unwrap_or(5),
            "expiresIn": v["expires_in"].as_u64().unwrap_or(900),
        }))
        .into_response(),
        Ok((status, v)) => misconfigured(status, &v).unwrap_or_else(|| {
            tracing::warn!(status, error = %v["error"], "device start failed");
            error(StatusCode::BAD_GATEWAY, "providerError")
        }),
        Err(e) => {
            tracing::warn!(error = %e, "device start failed");
            error(StatusCode::BAD_GATEWAY, "providerError")
        }
    }
}

/// `POST /v1/auth/device/poll`.
pub async fn device_poll(
    State(state): State<AppState>,
    Json(req): Json<DevicePollRequest>,
) -> Response {
    let Some(p) = Provider::parse(&req.provider) else {
        return error(StatusCode::BAD_REQUEST, "badProvider");
    };
    let providers = &state.config.providers;
    let Some(client_id) = providers.client_id(p) else {
        return error(StatusCode::SERVICE_UNAVAILABLE, "notConfigured");
    };
    let r = match providers.token_request(&state.http, p, client_id, &req.device_code) {
        Ok(r) => r,
        Err(_) => return error(StatusCode::INTERNAL_SERVER_ERROR, "internal"),
    };
    let v = match send_json(&state.http, r).await {
        Ok((status, v)) => {
            if let Some(reply) = misconfigured(status, &v) {
                return reply;
            }
            v
        }
        Err(e) => {
            tracing::warn!(error = %e, "device poll failed");
            return error(StatusCode::BAD_GATEWAY, "providerError");
        }
    };
    if let Some(code) = v["error"].as_str() {
        return Json(match code {
            "authorization_pending" => json!({"status": "pending"}),
            "slow_down" => {
                json!({"status": "slowDown", "interval": v["interval"].as_u64().unwrap_or(10)})
            }
            // An unknown device code: start again, as for an expired one.
            "expired_token" | "incorrect_device_code" => json!({"status": "expired"}),
            "access_denied" => json!({"status": "denied"}),
            other => {
                tracing::warn!(error = other, "device poll error");
                return error(StatusCode::BAD_GATEWAY, "providerError");
            }
        })
        .into_response();
    }
    let Some(access_token) = v["access_token"].as_str().map(str::to_string) else {
        return error(StatusCode::BAD_GATEWAY, "providerError");
    };
    match session_for_token(&state, p, access_token).await {
        Ok(session) => Json(json!({"status": "complete", "session": session})).into_response(),
        Err((status, code)) => error(status, code),
    }
}

/// Reads the profile with a provider access token, then drops the token and signs that person in.
async fn session_for_token(
    state: &AppState,
    p: Provider,
    access_token: String,
) -> Result<Session, (StatusCode, &'static str)> {
    let providers = &state.config.providers;
    // The access token is used once, here, and then dropped.
    let user = match providers.user_request(&state.http, p, &access_token) {
        Ok(r) => send_json(&state.http, r).await,
        Err(_) => return Err((StatusCode::INTERNAL_SERVER_ERROR, "internal")),
    };
    drop(access_token);
    let user = match user {
        Ok((200, u)) => u,
        _ => return Err((StatusCode::BAD_GATEWAY, "providerError")),
    };
    let (prefix, login_field) = match p {
        Provider::Github => ("gh", "login"),
        Provider::Gitlab => ("gl", "username"),
    };
    let Some(login) = user[login_field].as_str().map(|l| l.to_lowercase()) else {
        return Err((StatusCode::BAD_GATEWAY, "providerError"));
    };
    let name = user["name"]
        .as_str()
        .filter(|n| !n.trim().is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| login.clone());
    let person = Person {
        id: format!("p-{prefix}-{login}"),
        initials: initials(&name, &login),
        name,
    };
    let workspace = format!("ws-{prefix}-{login}");
    sign_in(state, person, prefix, &login, &workspace)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "sign-in failed");
            (StatusCode::INTERNAL_SERVER_ERROR, "internal")
        })
}

// ---------------------------------------------------------------------------------------- redirect flow
//
// "Continue with GitHub" in one click: the app sends the browser to GitHub's authorize page, GitHub sends it back
// to the relay's callback, and the relay sends it on to the app with a one-time code. The app trades that code,
// with the PKCE verifier only it holds, for a session. The relay keeps the OAuth app's client secret; neither
// the code from GitHub nor GitHub's token ever reaches the app.

/// How long the authorize round trip may take, and how long a one-time code lives.
const STATE_TTL_SECS: i64 = 600;
const CODE_TTL: std::time::Duration = std::time::Duration::from_secs(120);
const STATE_AUDIENCE: &str = "astra-oauth-state";
pub const CALLBACK_PATH: &str = "/v1/auth/redirect/callback";

/// The signed `state` round-tripped through GitHub: where to return, and the app's PKCE challenge.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StateClaims {
    pub aud: String,
    pub provider: String,
    pub redirect: String,
    pub challenge: String,
    /// The callback URL sent to GitHub; the token exchange must repeat it.
    pub callback: String,
    pub nonce: String,
    pub exp: i64,
}

impl Jwt {
    /// Signs a redirect-flow `state`.
    pub fn sign_state(&self, claims: &StateClaims) -> Result<String, jsonwebtoken::errors::Error> {
        jsonwebtoken::encode(&Header::new(Algorithm::HS256), claims, &self.enc)
    }

    /// Verifies a `state`: signature, expiry and audience, so a session token is never accepted as one.
    pub fn verify_state(&self, token: &str) -> Result<StateClaims, jsonwebtoken::errors::Error> {
        let mut v = Validation::new(Algorithm::HS256);
        v.leeway = 5;
        v.set_audience(&[STATE_AUDIENCE]);
        v.set_required_spec_claims(&["exp", "aud"]);
        Ok(jsonwebtoken::decode::<StateClaims>(token, &self.dec, &v)?.claims)
    }
}

/// Random URL-safe text with `bytes` bytes of entropy from the OS.
pub fn random_token(bytes: usize) -> String {
    use base64::Engine;
    let mut buf = vec![0u8; bytes];
    getrandom::fill(&mut buf).expect("the OS random number generator");
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(buf)
}

/// The PKCE `S256` challenge for `verifier`.
pub fn pkce_challenge(verifier: &str) -> String {
    use base64::Engine;
    use sha2::{Digest, Sha256};
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

fn is_pkce_value(s: &str, min: usize) -> bool {
    (min..=128).contains(&s.len())
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~'))
}

fn same(a: &str, b: &str) -> bool {
    a.len() == b.len()
        && a.bytes()
            .zip(b.bytes())
            .fold(0u8, |acc, (x, y)| acc | (x ^ y))
            == 0
}

struct Pending {
    session: Session,
    challenge: String,
    expires: std::time::Instant,
}

/// One-time codes waiting for the app to trade them in. In memory: the relay runs as one instance, and a code
/// lives two minutes, so a restart only means signing in again.
#[derive(Default)]
pub struct PendingCodes(Mutex<HashMap<String, Pending>>);

/// Why a one-time code could not be traded in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodeError {
    Unknown,
    Expired,
    WrongVerifier,
}

impl PendingCodes {
    /// Holds `session` for the app that sent `challenge`; returns the one-time code.
    pub fn insert(&self, session: Session, challenge: String) -> String {
        let code = random_token(32);
        let now = std::time::Instant::now();
        if let Ok(mut m) = self.0.lock() {
            m.retain(|_, p| p.expires > now);
            m.insert(
                code.clone(),
                Pending {
                    session,
                    challenge,
                    expires: now + CODE_TTL,
                },
            );
        }
        code
    }

    /// Trades `code` in. A code works once: a wrong verifier burns it too.
    pub fn take(&self, code: &str, verifier: &str) -> Result<Session, CodeError> {
        let pending = self
            .0
            .lock()
            .map_err(|_| CodeError::Unknown)?
            .remove(code)
            .ok_or(CodeError::Unknown)?;
        if pending.expires <= std::time::Instant::now() {
            return Err(CodeError::Expired);
        }
        if !same(&pkce_challenge(verifier), &pending.challenge) {
            return Err(CodeError::WrongVerifier);
        }
        Ok(pending.session)
    }
}

/// Where the relay may send a browser back to: the web app's origins (`ASTRA_ALLOWED_ORIGINS`; any origin in dev
/// auth mode when that is unset), or a loopback address, where the desktop app listens for its sign-in to finish
/// (RFC 8252). Anything else is refused: an open redirect would hand someone else's code to an attacker's page.
pub fn redirect_allowed(config: &crate::ServerConfig, redirect: &str) -> bool {
    let Ok(u) = url::Url::parse(redirect) else {
        return false;
    };
    if !matches!(u.scheme(), "http" | "https") || !u.username().is_empty() || u.password().is_some()
    {
        return false;
    }
    let loopback =
        u.scheme() == "http" && matches!(u.host_str(), Some("127.0.0.1" | "localhost" | "[::1]"));
    if loopback {
        return true;
    }
    let origin = u.origin().ascii_serialization();
    match &config.allowed_origins {
        Some(list) => list
            .iter()
            .any(|o| o.trim().trim_end_matches('/') == origin),
        None => config.dev_auth,
    }
}

/// The relay's own public URL: `ASTRA_PUBLIC_URL`, else what the request (and the load balancer) says.
fn public_url(state: &AppState, headers: &HeaderMap) -> Option<String> {
    if let Some(u) = state.config.public_url.as_deref().filter(|u| !u.is_empty()) {
        return Some(u.trim_end_matches('/').to_string());
    }
    let first = |name: &str| {
        headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.split(',').next())
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
    };
    let host = first("x-forwarded-host").or_else(|| first("host"))?;
    if !host
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b':' | b'[' | b']'))
    {
        return None;
    }
    let proto = match first("x-forwarded-proto").as_deref() {
        Some("https") => "https",
        _ => "http",
    };
    Some(format!("{proto}://{host}"))
}

impl Providers {
    fn client_secret(&self, p: Provider) -> Option<&str> {
        match p {
            Provider::Github => self.github_client_secret.as_deref(),
            Provider::Gitlab => None,
        }
        .filter(|s| !s.is_empty())
    }

    /// GitHub's authorize page for this sign-in.
    pub fn authorize_url(&self, client_id: &str, callback: &str, state: &str) -> String {
        let mut u = url::Url::parse(&format!("{}/login/oauth/authorize", self.github_oauth_base))
            .unwrap_or_else(|_| {
                url::Url::parse("https://github.com/login/oauth/authorize").expect("url")
            });
        u.query_pairs_mut()
            .append_pair("client_id", client_id)
            .append_pair("redirect_uri", callback)
            .append_pair("scope", "read:user user:email")
            .append_pair("state", state)
            .append_pair("allow_signup", "true");
        u.into()
    }

    /// Trades GitHub's authorization code for an access token (needs the client secret).
    pub fn code_request(
        &self,
        http: &reqwest::Client,
        client_id: &str,
        client_secret: &str,
        code: &str,
        callback: &str,
    ) -> reqwest::Result<reqwest::Request> {
        http.post(format!(
            "{}/login/oauth/access_token",
            self.github_oauth_base
        ))
        .header("accept", "application/json")
        .form(&[
            ("client_id", client_id),
            ("client_secret", client_secret),
            ("code", code),
            ("redirect_uri", callback),
        ])
        .build()
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RedirectStartRequest {
    pub provider: String,
    pub redirect: String,
    pub challenge: String,
}

/// `POST /v1/auth/redirect/start`: the provider's authorize URL for the app to open.
pub async fn redirect_start(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<RedirectStartRequest>,
) -> Response {
    let Some(p) = Provider::parse(&req.provider) else {
        return error(StatusCode::BAD_REQUEST, "badProvider");
    };
    let providers = &state.config.providers;
    let Some(client_id) = providers.client_id(p) else {
        return error(StatusCode::SERVICE_UNAVAILABLE, "notConfigured");
    };
    // Without a client secret (or for GitLab) the app falls back to the device flow.
    if providers.client_secret(p).is_none() {
        return error(StatusCode::SERVICE_UNAVAILABLE, "redirectNotConfigured");
    }
    if !is_pkce_value(&req.challenge, 43) {
        return error(StatusCode::BAD_REQUEST, "badChallenge");
    }
    if !redirect_allowed(&state.config, &req.redirect) {
        tracing::warn!(redirect = %req.redirect, "refused a sign-in redirect that is not an allowed origin");
        return error(StatusCode::BAD_REQUEST, "badRedirect");
    }
    let Some(base) = public_url(&state, &headers) else {
        return error(StatusCode::INTERNAL_SERVER_ERROR, "internal");
    };
    let callback = format!("{base}{CALLBACK_PATH}");
    let claims = StateClaims {
        aud: STATE_AUDIENCE.into(),
        provider: req.provider,
        redirect: req.redirect,
        challenge: req.challenge,
        callback: callback.clone(),
        nonce: random_token(16),
        exp: chrono::Utc::now().timestamp() + STATE_TTL_SECS,
    };
    let Ok(signed) = state.jwt.sign_state(&claims) else {
        return error(StatusCode::INTERNAL_SERVER_ERROR, "internal");
    };
    Json(json!({"url": providers.authorize_url(client_id, &callback, &signed)})).into_response()
}

#[derive(Deserialize)]
pub struct CallbackQuery {
    pub code: Option<String>,
    pub state: Option<String>,
    pub error: Option<String>,
}

/// Sends the browser back to the app with `param=value` added to its address.
fn back_to(redirect: &str, param: &str, value: &str) -> Response {
    let Ok(mut u) = url::Url::parse(redirect) else {
        return page(StatusCode::BAD_REQUEST, "This sign-in link is not valid.");
    };
    u.query_pairs_mut().append_pair(param, value);
    let mut r = axum::response::Redirect::to(u.as_str()).into_response();
    let h = r.headers_mut();
    h.insert(
        axum::http::header::CACHE_CONTROL,
        axum::http::HeaderValue::from_static("no-store"),
    );
    h.insert(
        axum::http::header::REFERRER_POLICY,
        axum::http::HeaderValue::from_static("no-referrer"),
    );
    r
}

/// A plain page for a callback that can't go back to the app (its `state` is missing, forged or expired).
fn page(status: StatusCode, message: &str) -> Response {
    let html = format!(
        "<!doctype html><meta charset=utf-8><meta name=viewport content=\"width=device-width\"><title>Astra</title>\
         <body style=\"font:16px system-ui;background:#000;color:#fff;padding:48px\"><p>{message}</p>\
         <p>Go back to Astra and choose Continue with GitHub again.</p></body>"
    );
    (
        status,
        [
            (axum::http::header::CONTENT_TYPE, "text/html; charset=utf-8"),
            (axum::http::header::CACHE_CONTROL, "no-store"),
        ],
        html,
    )
        .into_response()
}

/// `GET /v1/auth/redirect/callback`: GitHub sends the browser here; the relay signs the person in and sends the
/// browser back to the app with `astraCode` (or `authError`).
pub async fn redirect_callback(
    State(state): State<AppState>,
    axum::extract::Query(q): axum::extract::Query<CallbackQuery>,
) -> Response {
    let Some(claims) = q
        .state
        .as_deref()
        .and_then(|t| state.jwt.verify_state(t).ok())
    else {
        return page(
            StatusCode::BAD_REQUEST,
            "This sign-in link expired or is not valid.",
        );
    };
    // Re-check: the allowed origins may have changed since the round trip started.
    if !redirect_allowed(&state.config, &claims.redirect) {
        return page(StatusCode::BAD_REQUEST, "This sign-in link is not valid.");
    }
    let fail = |code: &str| back_to(&claims.redirect, "authError", code);
    if let Some(e) = q.error.as_deref() {
        return fail(if e == "access_denied" {
            "denied"
        } else {
            "server"
        });
    }
    let Some(code) = q.code.as_deref().filter(|c| !c.is_empty()) else {
        return fail("server");
    };
    let p = Provider::parse(&claims.provider).unwrap_or(Provider::Github);
    let providers = &state.config.providers;
    let (Some(client_id), Some(secret)) = (providers.client_id(p), providers.client_secret(p))
    else {
        return fail("notConfigured");
    };
    let r = match providers.code_request(&state.http, client_id, secret, code, &claims.callback) {
        Ok(r) => r,
        Err(_) => return fail("server"),
    };
    let v = match send_json(&state.http, r).await {
        Ok((_, v)) => v,
        Err(e) => {
            tracing::warn!(error = %e, "code exchange failed");
            return fail("network");
        }
    };
    let Some(access_token) = v["access_token"].as_str().map(str::to_string) else {
        let e = v["error"].as_str().unwrap_or_default();
        return match e {
            // A code that was already used or took too long.
            "bad_verification_code" => fail("expired"),
            "incorrect_client_credentials" | "invalid_client" | "unauthorized_client" => {
                tracing::error!(
                    error = e,
                    "the OAuth app is misconfigured: check GITHUB_CLIENT_ID and GITHUB_CLIENT_SECRET"
                );
                fail("badClientId")
            }
            "redirect_uri_mismatch" => {
                tracing::error!(callback = %claims.callback, "GitHub refused the callback: set the OAuth app's callback URL to it (or ASTRA_PUBLIC_URL)");
                fail("server")
            }
            other => {
                tracing::warn!(error = other, "code exchange failed");
                fail("server")
            }
        };
    };
    match session_for_token(&state, p, access_token).await {
        Ok(session) => back_to(
            &claims.redirect,
            "astraCode",
            &state.oauth.insert(session, claims.challenge),
        ),
        Err(_) => fail("server"),
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RedirectExchangeRequest {
    pub code: String,
    pub verifier: String,
}

/// `POST /v1/auth/redirect/exchange`: the one-time code and the PKCE verifier for a session.
pub async fn redirect_exchange(
    State(state): State<AppState>,
    Json(req): Json<RedirectExchangeRequest>,
) -> Response {
    if !is_pkce_value(&req.verifier, 43) {
        return error(StatusCode::BAD_REQUEST, "invalidCode");
    }
    match state.oauth.take(&req.code, &req.verifier) {
        Ok(session) => Json(json!({"session": session})).into_response(),
        Err(e) => {
            tracing::warn!(reason = ?e, "refused a sign-in code");
            error(StatusCode::BAD_REQUEST, "invalidCode")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jwt_roundtrip_and_rejects() {
        let jwt = Jwt::new(b"secret").expect("jwt");
        let (token, claims) = jwt.issue("p-1", "ws-1").expect("issue");
        assert_eq!(jwt.verify(&token).expect("verify"), claims);
        assert!(Jwt::new(b"other").expect("jwt").verify(&token).is_err());
        let past = Claims {
            sub: "p".into(),
            ws: "w".into(),
            iat: 0,
            exp: 1000,
        };
        let old = jsonwebtoken::encode(
            &Header::new(Algorithm::HS256),
            &past,
            &EncodingKey::from_secret(b"secret").expect("key"),
        )
        .expect("enc");
        assert!(jwt.verify(&old).is_err());
    }

    #[test]
    fn names() {
        assert_eq!(initials("Mark Chen", "mc"), "MC");
        assert_eq!(initials("Lin", "lin"), "LI");
        assert_eq!(initials("", "octocat"), "OC");
        assert_eq!(slug("Mark Chen!"), "mark-chen");
        assert_eq!(slug("  "), "user");
    }

    #[test]
    fn device_flow_requests() {
        let http = reqwest::Client::new();
        let p = Providers::default();
        let r = p
            .device_code_request(&http, Provider::Github, "cid")
            .expect("req");
        assert_eq!(r.url().as_str(), "https://github.com/login/device/code");
        assert_eq!(r.headers()["accept"], "application/json");
        let body = String::from_utf8(r.body().and_then(|b| b.as_bytes()).expect("body").to_vec())
            .expect("utf8");
        assert_eq!(body, "client_id=cid&scope=read%3Auser+user%3Aemail");
        let r = p
            .token_request(&http, Provider::Github, "cid", "dc")
            .expect("req");
        assert_eq!(
            r.url().as_str(),
            "https://github.com/login/oauth/access_token"
        );
        let body = String::from_utf8(r.body().and_then(|b| b.as_bytes()).expect("body").to_vec())
            .expect("utf8");
        assert_eq!(
            body,
            "client_id=cid&device_code=dc&grant_type=urn%3Aietf%3Aparams%3Aoauth%3Agrant-type%3Adevice_code"
        );
        let r = p.user_request(&http, Provider::Github, "tok").expect("req");
        assert_eq!(r.url().as_str(), "https://api.github.com/user");
        assert_eq!(r.headers()["authorization"], "Bearer tok");
        let r = p
            .device_code_request(&http, Provider::Gitlab, "gid")
            .expect("req");
        assert_eq!(
            r.url().as_str(),
            "https://gitlab.com/oauth/authorize_device"
        );
        let r = p.user_request(&http, Provider::Gitlab, "tok").expect("req");
        assert_eq!(r.url().as_str(), "https://gitlab.com/api/v4/user");
    }

    #[test]
    fn redirect_targets_are_checked() {
        let mut c = crate::ServerConfig {
            allowed_origins: Some(vec!["https://app.example.com".into()]),
            ..crate::ServerConfig::default()
        };
        assert!(redirect_allowed(&c, "https://app.example.com/"));
        assert!(redirect_allowed(
            &c,
            "https://app.example.com/deep/path?x=1"
        ));
        assert!(redirect_allowed(&c, "http://127.0.0.1:53111/callback"));
        assert!(redirect_allowed(&c, "http://localhost:4000/callback"));
        assert!(redirect_allowed(&c, "http://[::1]:4000/callback"));
        assert!(!redirect_allowed(&c, "https://evil.example.com/"));
        assert!(!redirect_allowed(&c, "https://app.example.com.evil.com/"));
        assert!(!redirect_allowed(&c, "https://user@app.example.com/"));
        assert!(!redirect_allowed(&c, "https://127.0.0.1/"));
        assert!(!redirect_allowed(&c, "javascript:alert(1)"));
        assert!(!redirect_allowed(&c, "tauri://localhost/"));
        c.allowed_origins = None;
        assert!(!redirect_allowed(&c, "https://app.example.com/"));
        assert!(redirect_allowed(&c, "http://127.0.0.1:1/callback"));
        c.dev_auth = true;
        assert!(redirect_allowed(&c, "http://192.168.1.4:5173/"));
    }

    #[test]
    fn state_tokens_and_session_tokens_never_pass_for_each_other() {
        let jwt = Jwt::new(b"secret").expect("jwt");
        let claims = StateClaims {
            aud: STATE_AUDIENCE.into(),
            provider: "github".into(),
            redirect: "http://127.0.0.1:1/callback".into(),
            challenge: pkce_challenge("v".repeat(43).as_str()),
            callback: "https://sync.example.com/v1/auth/redirect/callback".into(),
            nonce: random_token(16),
            exp: chrono::Utc::now().timestamp() + 60,
        };
        let state = jwt.sign_state(&claims).expect("sign");
        assert_eq!(jwt.verify_state(&state).expect("verify"), claims);
        assert!(jwt.verify(&state).is_err(), "a state is not a session");
        let (session, _) = jwt.issue("p-1", "ws-1").expect("issue");
        assert!(
            jwt.verify_state(&session).is_err(),
            "a session is not a state"
        );
        assert!(
            Jwt::new(b"other")
                .expect("jwt")
                .verify_state(&state)
                .is_err()
        );
        let expired = StateClaims {
            exp: 1000,
            ..claims
        };
        assert!(
            jwt.verify_state(&jwt.sign_state(&expired).expect("sign"))
                .is_err()
        );
    }

    #[test]
    fn one_time_codes_need_the_verifier_and_work_once() {
        // RFC 7636 appendix B.
        assert_eq!(
            pkce_challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
        let session = Session {
            token: "t".into(),
            workspace_id: "ws".into(),
            person: Value::Null,
            expires_at: String::new(),
        };
        let codes = PendingCodes::default();
        let verifier = random_token(32);
        let code = codes.insert(session.clone(), pkce_challenge(&verifier));
        assert_eq!(code.len(), 43);
        assert_eq!(codes.take(&code, &verifier), Ok(session.clone()));
        assert_eq!(codes.take(&code, &verifier), Err(CodeError::Unknown));
        let code = codes.insert(session, pkce_challenge(&verifier));
        assert_eq!(
            codes.take(&code, &random_token(32)),
            Err(CodeError::WrongVerifier)
        );
        assert_eq!(
            codes.take(&code, &verifier),
            Err(CodeError::Unknown),
            "a wrong verifier burns the code"
        );
    }

    #[test]
    fn authorize_url_and_code_request() {
        let p = Providers::default();
        let u = url::Url::parse(&p.authorize_url("cid", "https://s.example.com/cb", "st"))
            .expect("url");
        assert_eq!(u.path(), "/login/oauth/authorize");
        let q: HashMap<_, _> = u.query_pairs().into_owned().collect();
        assert_eq!(q["client_id"], "cid");
        assert_eq!(q["redirect_uri"], "https://s.example.com/cb");
        assert_eq!(q["scope"], "read:user user:email");
        assert_eq!(q["state"], "st");
        let r = p
            .code_request(
                &reqwest::Client::new(),
                "cid",
                "sec",
                "abc",
                "https://s.example.com/cb",
            )
            .expect("req");
        let body = String::from_utf8(r.body().and_then(|b| b.as_bytes()).expect("body").to_vec())
            .expect("utf8");
        assert_eq!(
            body,
            "client_id=cid&client_secret=sec&code=abc&redirect_uri=https%3A%2F%2Fs.example.com%2Fcb"
        );
    }
}
