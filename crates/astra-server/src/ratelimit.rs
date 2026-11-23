//! Per-client token-bucket rate limiting.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Mutex;
use std::time::Instant;

use axum::extract::{ConnectInfo, Request, State};
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use crate::AppState;

/// A token bucket per client key.
pub struct Limiter {
    per_min: f64,
    burst: f64,
    buckets: Mutex<HashMap<String, (f64, Instant)>>,
}

impl Limiter {
    /// `per_min` requests per minute, bursting up to `per_min / 4` (at least 10).
    pub fn new(per_min: u32) -> Self {
        let per_min = f64::from(per_min.max(1));
        Limiter { per_min, burst: (per_min / 4.0).max(10.0), buckets: Mutex::new(HashMap::new()) }
    }

    /// Take one token for `key`.
    pub fn check(&self, key: &str) -> bool {
        let now = Instant::now();
        let Ok(mut b) = self.buckets.lock() else { return true };
        if b.len() > 50_000 {
            let per_sec = self.per_min / 60.0;
            let burst = self.burst;
            b.retain(|_, (tokens, at)| *tokens + now.duration_since(*at).as_secs_f64() * per_sec < burst);
        }
        let (tokens, at) = b.entry(key.to_string()).or_insert((self.burst, now));
        *tokens = (*tokens + now.duration_since(*at).as_secs_f64() * self.per_min / 60.0).min(self.burst);
        *at = now;
        if *tokens >= 1.0 {
            *tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

/// Client key: first `X-Forwarded-For` address (behind a load balancer), else the peer address.
fn client_key(req: &Request) -> String {
    if let Some(xff) = req.headers().get("x-forwarded-for").and_then(|v| v.to_str().ok()) {
        if let Some(first) = xff.split(',').next() {
            return first.trim().to_string();
        }
    }
    req.extensions().get::<ConnectInfo<SocketAddr>>().map(|c| c.0.ip().to_string()).unwrap_or_default()
}

/// Axum middleware.
pub async fn middleware(State(state): State<AppState>, req: Request, next: Next) -> Response {
    if !state.limiter.check(&client_key(&req)) {
        return (StatusCode::TOO_MANY_REQUESTS, axum::Json(serde_json::json!({"error": "rateLimited"}))).into_response();
    }
    next.run(req).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bucket_limits() {
        let l = Limiter::new(60);
        let ok = (0..20).filter(|_| l.check("a")).count();
        assert_eq!(ok, 15);
        assert!(l.check("b"));
    }
}
