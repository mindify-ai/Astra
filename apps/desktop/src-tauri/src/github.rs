//! Sign in with GitHub straight from the app (no sync server): GitHub's OAuth **device flow**, with the
//! app's public client id built in. The device flow needs no client secret, and none is shipped: anything in
//! the bundle can be read by whoever downloads it.
//!
//! GitHub's access token reads the profile (name and login), lists your repositories and clones them. On the
//! desktop it is kept in the OS keychain only: never in settings, never synced, never handed to the webview.

use serde::Serialize;
use serde_json::{Value, json};

/// The OAuth app's client id (public). Override at build time with `ASTRA_GITHUB_CLIENT_ID`.
pub const CLIENT_ID: &str = match option_env!("ASTRA_GITHUB_CLIENT_ID") {
    Some(id) => id,
    None => "Ov23liM5JwebYv5cxD3v",
};

const OAUTH_BASE: &str = "https://github.com";
const API_BASE: &str = "https://api.github.com";
const DEVICE_GRANT: &str = "urn:ietf:params:oauth:grant-type:device_code";
/// The profile (your name in Astra) and your repositories, private ones included, to list and clone them.
const SCOPE: &str = "repo read:user";

/// `@astra/client` `DeviceCode`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceCode {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    pub verification_uri_complete: Option<String>,
    pub interval: u64,
    pub expires_in: u64,
}

/// A poll that has not finished (`@astra/client` `PollResult` minus `complete`), or the access token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Poll {
    Pending,
    SlowDown(u64),
    Expired,
    Denied,
    Token(String),
}

/// The GitHub account that signed in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Profile {
    pub login: String,
    pub name: String,
}

/// Error codes the sign-in screen explains (`@astra/client` `AuthErrorCode`).
pub fn misconfigured(status: u16, v: &Value) -> Option<&'static str> {
    match v["error"].as_str().unwrap_or_default() {
        "device_flow_disabled" => Some("deviceFlowDisabled"),
        "incorrect_client_credentials" | "unauthorized_client" | "invalid_client" => {
            Some("badClientId")
        }
        _ if status == 404 => Some("badClientId"),
        _ => None,
    }
}

/// Reads GitHub's answer to the device-code request.
pub fn parse_start(status: u16, v: &Value) -> Result<DeviceCode, &'static str> {
    if let Some(code) = misconfigured(status, v) {
        return Err(code);
    }
    let text = |k: &str| v[k].as_str().map(str::to_string);
    match (text("device_code"), text("user_code"), text("verification_uri")) {
        (Some(device_code), Some(user_code), Some(verification_uri)) if status == 200 => {
            Ok(DeviceCode {
                device_code,
                user_code,
                verification_uri,
                verification_uri_complete: text("verification_uri_complete"),
                interval: v["interval"].as_u64().unwrap_or(5),
                expires_in: v["expires_in"].as_u64().unwrap_or(900),
            })
        }
        _ => Err("server"),
    }
}

/// Reads GitHub's answer to a token poll.
pub fn parse_poll(status: u16, v: &Value) -> Result<Poll, &'static str> {
    if let Some(code) = misconfigured(status, v) {
        return Err(code);
    }
    match v["error"].as_str() {
        Some("authorization_pending") => Ok(Poll::Pending),
        Some("slow_down") => Ok(Poll::SlowDown(v["interval"].as_u64().unwrap_or(10))),
        // An unknown device code: start again, as for an expired one.
        Some("expired_token" | "incorrect_device_code") => Ok(Poll::Expired),
        Some("access_denied") => Ok(Poll::Denied),
        Some(_) => Err("server"),
        None => v["access_token"]
            .as_str()
            .map(|t| Poll::Token(t.to_string()))
            .ok_or("server"),
    }
}

/// Reads the `/user` profile.
pub fn parse_profile(status: u16, v: &Value) -> Result<Profile, &'static str> {
    if status != 200 {
        return Err("server");
    }
    let login = v["login"].as_str().ok_or("server")?.to_string();
    let name = v["name"]
        .as_str()
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| login.clone());
    Ok(Profile { login, name })
}

/// The `PollResult` JSON for an unfinished poll.
pub fn poll_json(p: &Poll) -> Value {
    match p {
        Poll::Pending => json!({"status": "pending"}),
        Poll::SlowDown(interval) => json!({"status": "slowDown", "interval": interval}),
        Poll::Expired => json!({"status": "expired"}),
        Poll::Denied => json!({"status": "denied"}),
        Poll::Token(_) => json!({"status": "pending"}),
    }
}

fn client() -> Result<reqwest::Client, &'static str> {
    reqwest::Client::builder()
        .user_agent("Astra desktop")
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .map_err(|_| "network")
}

async fn send(req: reqwest::RequestBuilder) -> Result<(u16, Value), &'static str> {
    let resp = req
        .header("accept", "application/json")
        .send()
        .await
        .map_err(|_| "network")?;
    let status = resp.status().as_u16();
    Ok((status, resp.json::<Value>().await.unwrap_or(Value::Null)))
}

/// Asks GitHub for a device code.
pub async fn start() -> Result<DeviceCode, &'static str> {
    let http = client()?;
    let (status, v) = send(
        http.post(format!("{OAUTH_BASE}/login/device/code"))
            .form(&[("client_id", CLIENT_ID), ("scope", SCOPE)]),
    )
    .await?;
    parse_start(status, &v)
}

/// Polls once; on approval, reads the profile and returns it with the token.
pub async fn poll(device_code: &str) -> Result<Result<(Profile, String), Poll>, &'static str> {
    let http = client()?;
    let (status, v) = send(http.post(format!("{OAUTH_BASE}/login/oauth/access_token")).form(&[
        ("client_id", CLIENT_ID),
        ("device_code", device_code),
        ("grant_type", DEVICE_GRANT),
    ]))
    .await?;
    let token = match parse_poll(status, &v)? {
        Poll::Token(t) => t,
        other => return Ok(Err(other)),
    };
    let (status, v) = send(http.get(format!("{API_BASE}/user")).bearer_auth(&token)).await?;
    parse_profile(status, &v).map(|p| Ok((p, token)))
}

/// One of your repositories (`@astra/client` `GithubRepo`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Repo {
    pub full_name: String,
    pub name: String,
    pub owner: String,
    pub private: bool,
    pub default_branch: String,
    pub clone_url: String,
    pub description: Option<String>,
    pub updated_at: Option<String>,
}

/// Reads one page of `/user/repos`. `signedOut` when GitHub no longer accepts the token.
pub fn parse_repos(status: u16, v: &Value) -> Result<Vec<Repo>, &'static str> {
    match status {
        200 => {}
        401 => return Err("signedOut"),
        _ => return Err("server"),
    }
    let text = |r: &Value, k: &str| r[k].as_str().map(str::to_string);
    Ok(v.as_array()
        .into_iter()
        .flatten()
        .filter_map(|r| {
            Some(Repo {
                full_name: text(r, "full_name")?,
                name: text(r, "name")?,
                owner: text(&r["owner"], "login")?,
                private: r["private"].as_bool().unwrap_or(false),
                default_branch: text(r, "default_branch").unwrap_or_else(|| "main".into()),
                clone_url: text(r, "clone_url")?,
                description: text(r, "description"),
                updated_at: text(r, "pushed_at").or_else(|| text(r, "updated_at")),
            })
        })
        .collect())
}

/// Your repositories (owned, collaborator and organization), most recently pushed first; up to 500.
pub async fn repos(token: &str) -> Result<Vec<Repo>, &'static str> {
    let http = client()?;
    let mut out = Vec::new();
    for page in 1..=5 {
        let (status, v) = send(
            http.get(format!(
                "{API_BASE}/user/repos?per_page=100&sort=pushed&page={page}"
            ))
            .bearer_auth(token),
        )
        .await?;
        let batch = parse_repos(status, &v)?;
        let done = batch.len() < 100;
        out.extend(batch);
        if done {
            break;
        }
    }
    Ok(out)
}

/// `owner/name`, as GitHub allows them; anything else could smuggle a path or a flag into `git clone`.
pub fn valid_full_name(full_name: &str) -> bool {
    let ok = |part: &str| {
        !part.is_empty()
            && !part.starts_with('.')
            && !part.starts_with('-')
            && part
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    };
    matches!(full_name.split_once('/'), Some((owner, name)) if ok(owner) && ok(name))
}

/// Standard base64 (for git's basic-auth header).
pub fn base64(bytes: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(T[(n >> (18 - 6 * i)) as usize & 63] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// `git clone` of `https://github.com/<full_name>.git` into `dest`. The token reaches git through environment
/// config only (`GIT_CONFIG_*`), so it is not in the process arguments and is not written to the clone's
/// `.git/config`; the clone's `origin` is the plain https URL.
pub fn clone(token: &str, full_name: &str, dest: &std::path::Path) -> Result<(), String> {
    if !valid_full_name(full_name) {
        return Err(format!("not a GitHub repository: {full_name}"));
    }
    let header = format!(
        "AUTHORIZATION: basic {}",
        base64(format!("x-access-token:{token}").as_bytes())
    );
    let out = std::process::Command::new("git")
        .arg("clone")
        .arg("--")
        .arg(format!("https://github.com/{full_name}.git"))
        .arg(dest)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_CONFIG_COUNT", "1")
        .env("GIT_CONFIG_KEY_0", "http.https://github.com/.extraheader")
        .env("GIT_CONFIG_VALUE_0", header)
        .output()
        .map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => {
                "Git is not installed. Install it (xcode-select --install), then try again.".into()
            }
            _ => e.to_string(),
        })?;
    if out.status.success() {
        return Ok(());
    }
    let err = String::from_utf8_lossy(&out.stderr);
    let last = err
        .lines()
        .rev()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("git clone failed");
    Err(last.trim().trim_start_matches("fatal: ").to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_codes_and_misconfiguration() {
        let ok = json!({"device_code": "dc", "user_code": "ABCD-1234",
            "verification_uri": "https://github.com/login/device", "expires_in": 899, "interval": 5});
        assert_eq!(
            parse_start(200, &ok),
            Ok(DeviceCode {
                device_code: "dc".into(),
                user_code: "ABCD-1234".into(),
                verification_uri: "https://github.com/login/device".into(),
                verification_uri_complete: None,
                interval: 5,
                expires_in: 899,
            })
        );
        // What github.com answers for an unknown client id.
        assert_eq!(
            parse_start(404, &json!({"error": "Not Found"})),
            Err("badClientId")
        );
        assert_eq!(
            parse_start(400, &json!({"error": "device_flow_disabled"})),
            Err("deviceFlowDisabled")
        );
        assert_eq!(parse_start(500, &Value::Null), Err("server"));
    }

    #[test]
    fn polls() {
        let p = |v: Value| parse_poll(200, &v);
        assert_eq!(
            p(json!({"error": "authorization_pending"})),
            Ok(Poll::Pending)
        );
        assert_eq!(
            p(json!({"error": "slow_down", "interval": 10})),
            Ok(Poll::SlowDown(10))
        );
        assert_eq!(p(json!({"error": "expired_token"})), Ok(Poll::Expired));
        assert_eq!(
            p(json!({"error": "incorrect_device_code"})),
            Ok(Poll::Expired)
        );
        assert_eq!(p(json!({"error": "access_denied"})), Ok(Poll::Denied));
        assert_eq!(
            p(json!({"access_token": "gho_x", "token_type": "bearer"})),
            Ok(Poll::Token("gho_x".into()))
        );
        assert_eq!(
            p(json!({"error": "device_flow_disabled"})),
            Err("deviceFlowDisabled")
        );
        assert_eq!(
            poll_json(&Poll::SlowDown(10)),
            json!({"status": "slowDown", "interval": 10})
        );
    }

    #[test]
    fn profiles() {
        assert_eq!(
            parse_profile(200, &json!({"login": "octocat", "name": "The Octocat"})),
            Ok(Profile {
                login: "octocat".into(),
                name: "The Octocat".into()
            })
        );
        assert_eq!(
            parse_profile(200, &json!({"login": "octocat", "name": null})),
            Ok(Profile {
                login: "octocat".into(),
                name: "octocat".into()
            })
        );
        assert_eq!(parse_profile(401, &Value::Null), Err("server"));
    }

    #[test]
    fn repositories() {
        let page = json!([
            {"full_name": "octocat/hello", "name": "hello", "owner": {"login": "octocat"}, "private": true,
             "default_branch": "trunk", "clone_url": "https://github.com/octocat/hello.git",
             "description": null, "pushed_at": "2026-11-20T10:00:00Z"},
            {"name": "broken"}
        ]);
        assert_eq!(
            parse_repos(200, &page),
            Ok(vec![Repo {
                full_name: "octocat/hello".into(),
                name: "hello".into(),
                owner: "octocat".into(),
                private: true,
                default_branch: "trunk".into(),
                clone_url: "https://github.com/octocat/hello.git".into(),
                description: None,
                updated_at: Some("2026-11-20T10:00:00Z".into()),
            }])
        );
        assert_eq!(parse_repos(401, &Value::Null), Err("signedOut"));
    }

    #[test]
    fn clone_names_and_auth_header() {
        assert!(valid_full_name("octocat/Hello-World"));
        assert!(valid_full_name("mindify-ai/astra.web"));
        for bad in ["octocat", "a/b/c", "../x", "-x/y", "x/--upload-pack=evil", "x/.git", "x y/z", ""] {
            assert!(!valid_full_name(bad), "{bad}");
        }
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"x-access-token:abc"), "eC1hY2Nlc3MtdG9rZW46YWJj");
    }

    #[test]
    fn the_built_in_client_id_is_public_and_no_secret_ships() {
        assert!(CLIENT_ID.starts_with("Ov23") || option_env!("ASTRA_GITHUB_CLIENT_ID").is_some());
        let src = include_str!("github.rs");
        assert!(!src.contains(concat!("client", "_secret")));
    }
}
