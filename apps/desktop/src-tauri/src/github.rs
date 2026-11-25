//! Sign in with GitHub straight from the app (no sync server): GitHub's OAuth **device flow**, with the
//! app's public client id built in. The device flow needs no client secret, and none is shipped: anything in
//! the bundle can be read by whoever downloads it.
//!
//! GitHub's access token is used once, to read the profile (name and login), and then dropped. It is never
//! stored, synced or handed to the webview.

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
/// Only the public profile: the name and login become your name in Astra.
const SCOPE: &str = "read:user";

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

/// Polls once; on approval, reads the profile with the token and drops the token.
pub async fn poll(device_code: &str) -> Result<Result<Profile, Poll>, &'static str> {
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
    let user = send(http.get(format!("{API_BASE}/user")).bearer_auth(&token)).await;
    drop(token);
    let (status, v) = user?;
    parse_profile(status, &v).map(Ok)
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
    fn the_built_in_client_id_is_public_and_no_secret_ships() {
        assert!(CLIENT_ID.starts_with("Ov23") || option_env!("ASTRA_GITHUB_CLIENT_ID").is_some());
        let src = include_str!("github.rs");
        assert!(!src.contains(concat!("client", "_secret")));
    }
}
