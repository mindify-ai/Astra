//! Authentication: HS256 session JWTs, the dev token endpoint, GitHub/GitLab OAuth device flow (proxied,
//! because those endpoints do not allow browser CORS) and `GET /v1/me`.
//!
//! The provider access token is used once to read the profile and then discarded; nothing but the person's
//! id, name and provider login is stored.

use std::collections::HashMap;
use std::sync::Mutex;

use astra_protocol::{HlcClock, Op, OpBody, Person};
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
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
        Ok(Jwt { enc: EncodingKey::from_secret(secret)?, dec: DecodingKey::from_secret(secret)? })
    }

    /// Issue a token for `person` in `workspace`.
    pub fn issue(&self, person: &str, workspace: &str) -> Result<(String, Claims), jsonwebtoken::errors::Error> {
        let now = chrono::Utc::now().timestamp();
        let claims = Claims { sub: person.into(), ws: workspace.into(), iat: now, exp: now + SESSION_DAYS * 86_400 };
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
    async fn authorize(&self, workspace_id: &str, _node_id: &str, token: Option<&str>) -> astra_sync::Result<astra_sync::Principal> {
        let token = token.ok_or_else(|| astra_sync::SyncError::Unauthorized("missing token".into()))?;
        let claims = self.jwt.verify(token).map_err(|e| astra_sync::SyncError::Unauthorized(e.to_string()))?;
        if claims.ws != workspace_id {
            return Err(astra_sync::SyncError::Unauthorized("workspace mismatch".into()));
        }
        Ok(astra_sync::Principal { subject: claims.sub })
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
        [a, b, ..] => [a.chars().next(), b.chars().next()].into_iter().flatten().collect(),
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
    pub async fn upsert(&self, person: &Person, provider: &str, login: &str, workspace: &str) -> Result<bool, String> {
        match self {
            Directory::Memory(m) => {
                let mut m = m.lock().map_err(|_| "poisoned".to_string())?;
                let new = !m.contains_key(&person.id);
                m.insert(person.id.clone(), (person.clone(), workspace.to_string()));
                Ok(new)
            }
            Directory::Postgres(pool) => {
                let existed: Option<(String,)> = sqlx::query_as("SELECT id FROM people WHERE id = $1")
                    .bind(&person.id)
                    .fetch_optional(pool)
                    .await
                    .map_err(|e| e.to_string())?;
                crate::pg::upsert_person(pool, &person.id, &person.name, provider, login, workspace)
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
pub async fn sign_in(state: &AppState, person: Person, provider: &str, login: &str, workspace: &str) -> Result<Session, String> {
    let new = state.directory.upsert(&person, provider, login, workspace).await?;
    if new {
        let hlc = state.clock.lock().map_err(|_| "poisoned".to_string())?.tick();
        let op = Op::new(hlc, "server", OpBody::put(person.clone()));
        state.hub.append_server_ops(workspace, &[op]).await.map_err(|e| e.to_string())?;
    }
    let (token, claims) = state.jwt.issue(&person.id, workspace).map_err(|e| e.to_string())?;
    let expires_at = chrono::DateTime::from_timestamp(claims.exp, 0)
        .map(|d| d.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
        .unwrap_or_default();
    Ok(Session { token, workspace_id: workspace.into(), person: person_json(&person), expires_at })
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
pub async fn dev_token(State(state): State<AppState>, Json(req): Json<DevTokenRequest>) -> Response {
    if !state.config.dev_auth {
        return error(StatusCode::NOT_FOUND, "notFound");
    }
    let name = req.name.trim();
    let name = if name.is_empty() { "Dev" } else { name };
    let person = Person { id: format!("p-dev-{}", slug(name)), name: name.into(), initials: initials(name, name) };
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
    let person = state.directory.get(&claims.sub).await.unwrap_or_else(|| Person {
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
    pub fn device_code_request(&self, http: &reqwest::Client, p: Provider, client_id: &str) -> reqwest::Result<reqwest::Request> {
        let (url, scope) = match p {
            Provider::Github => (format!("{}/login/device/code", self.github_oauth_base), "read:user user:email"),
            Provider::Gitlab => (format!("{}/oauth/authorize_device", self.gitlab_base), "read_user"),
        };
        http.post(url).header("accept", "application/json").form(&[("client_id", client_id), ("scope", scope)]).build()
    }

    /// The token poll request.
    pub fn token_request(&self, http: &reqwest::Client, p: Provider, client_id: &str, device_code: &str) -> reqwest::Result<reqwest::Request> {
        let url = match p {
            Provider::Github => format!("{}/login/oauth/access_token", self.github_oauth_base),
            Provider::Gitlab => format!("{}/oauth/token", self.gitlab_base),
        };
        http.post(url)
            .header("accept", "application/json")
            .form(&[("client_id", client_id), ("device_code", device_code), ("grant_type", DEVICE_GRANT)])
            .build()
    }

    /// The profile request.
    pub fn user_request(&self, http: &reqwest::Client, p: Provider, access_token: &str) -> reqwest::Result<reqwest::Request> {
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

async fn send_json(http: &reqwest::Client, req: reqwest::Request) -> Result<(u16, Value), String> {
    let resp = http.execute(req).await.map_err(|e| e.without_url().to_string())?;
    let status = resp.status().as_u16();
    let v = resp.json::<Value>().await.unwrap_or(Value::Null);
    Ok((status, v))
}

/// `POST /v1/auth/device/start`.
pub async fn device_start(State(state): State<AppState>, Json(req): Json<DeviceStartRequest>) -> Response {
    let Some(p) = Provider::parse(&req.provider) else { return error(StatusCode::BAD_REQUEST, "badProvider") };
    let Some(client_id) = state.config.providers.client_id(p) else {
        return error(StatusCode::SERVICE_UNAVAILABLE, "notConfigured");
    };
    let r = match state.config.providers.device_code_request(&state.http, p, client_id) {
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
        Ok((status, v)) => {
            tracing::warn!(status, error = %v["error"], "device start failed");
            error(StatusCode::BAD_GATEWAY, "providerError")
        }
        Err(e) => {
            tracing::warn!(error = %e, "device start failed");
            error(StatusCode::BAD_GATEWAY, "providerError")
        }
    }
}

/// `POST /v1/auth/device/poll`.
pub async fn device_poll(State(state): State<AppState>, Json(req): Json<DevicePollRequest>) -> Response {
    let Some(p) = Provider::parse(&req.provider) else { return error(StatusCode::BAD_REQUEST, "badProvider") };
    let providers = &state.config.providers;
    let Some(client_id) = providers.client_id(p) else {
        return error(StatusCode::SERVICE_UNAVAILABLE, "notConfigured");
    };
    let r = match providers.token_request(&state.http, p, client_id, &req.device_code) {
        Ok(r) => r,
        Err(_) => return error(StatusCode::INTERNAL_SERVER_ERROR, "internal"),
    };
    let v = match send_json(&state.http, r).await {
        Ok((_, v)) => v,
        Err(e) => {
            tracing::warn!(error = %e, "device poll failed");
            return error(StatusCode::BAD_GATEWAY, "providerError");
        }
    };
    if let Some(code) = v["error"].as_str() {
        return Json(match code {
            "authorization_pending" => json!({"status": "pending"}),
            "slow_down" => json!({"status": "slowDown", "interval": v["interval"].as_u64().unwrap_or(10)}),
            "expired_token" => json!({"status": "expired"}),
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
    // The access token is used once, here, and then dropped.
    let user = match providers.user_request(&state.http, p, &access_token) {
        Ok(r) => send_json(&state.http, r).await,
        Err(_) => return error(StatusCode::INTERNAL_SERVER_ERROR, "internal"),
    };
    drop(access_token);
    let user = match user {
        Ok((200, u)) => u,
        _ => return error(StatusCode::BAD_GATEWAY, "providerError"),
    };
    let (prefix, login_field) = match p {
        Provider::Github => ("gh", "login"),
        Provider::Gitlab => ("gl", "username"),
    };
    let Some(login) = user[login_field].as_str().map(|l| l.to_lowercase()) else {
        return error(StatusCode::BAD_GATEWAY, "providerError");
    };
    let name = user["name"].as_str().filter(|n| !n.trim().is_empty()).map(str::to_string).unwrap_or_else(|| login.clone());
    let person = Person { id: format!("p-{prefix}-{login}"), initials: initials(&name, &login), name };
    let workspace = format!("ws-{prefix}-{login}");
    match sign_in(&state, person, prefix, &login, &workspace).await {
        Ok(session) => Json(json!({"status": "complete", "session": session})).into_response(),
        Err(e) => {
            tracing::error!(error = %e, "sign-in failed");
            error(StatusCode::INTERNAL_SERVER_ERROR, "internal")
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
        let past = Claims { sub: "p".into(), ws: "w".into(), iat: 0, exp: 1000 };
        let old = jsonwebtoken::encode(&Header::new(Algorithm::HS256), &past, &EncodingKey::from_secret(b"secret").expect("key")).expect("enc");
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
        let r = p.device_code_request(&http, Provider::Github, "cid").expect("req");
        assert_eq!(r.url().as_str(), "https://github.com/login/device/code");
        assert_eq!(r.headers()["accept"], "application/json");
        let body = String::from_utf8(r.body().and_then(|b| b.as_bytes()).expect("body").to_vec()).expect("utf8");
        assert_eq!(body, "client_id=cid&scope=read%3Auser+user%3Aemail");
        let r = p.token_request(&http, Provider::Github, "cid", "dc").expect("req");
        assert_eq!(r.url().as_str(), "https://github.com/login/oauth/access_token");
        let body = String::from_utf8(r.body().and_then(|b| b.as_bytes()).expect("body").to_vec()).expect("utf8");
        assert_eq!(body, "client_id=cid&device_code=dc&grant_type=urn%3Aietf%3Aparams%3Aoauth%3Agrant-type%3Adevice_code");
        let r = p.user_request(&http, Provider::Github, "tok").expect("req");
        assert_eq!(r.url().as_str(), "https://api.github.com/user");
        assert_eq!(r.headers()["authorization"], "Bearer tok");
        let r = p.device_code_request(&http, Provider::Gitlab, "gid").expect("req");
        assert_eq!(r.url().as_str(), "https://gitlab.com/oauth/authorize_device");
        let r = p.user_request(&http, Provider::Gitlab, "tok").expect("req");
        assert_eq!(r.url().as_str(), "https://gitlab.com/api/v4/user");
    }
}
