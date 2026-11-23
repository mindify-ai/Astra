//! WebSocket transport for the sync client (MessagePack binary frames).

use astra_protocol::{ClientFrame, ServerFrame};
use astra_sync::codec::{self, Format};
use astra_sync::{Connector, SyncError, Transport};
use futures::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::Message;

/// Connects to `/v1/sync` over `ws://` or `wss://` (rustls).
#[derive(Debug, Clone)]
pub struct WsConnector {
    url: String,
    format: Format,
}

impl WsConnector {
    /// MessagePack frames to `url` (a relay base URL or the full sync URL; see [`sync_url`]).
    pub fn new(url: impl Into<String>) -> Self {
        WsConnector {
            url: sync_url(&url.into()),
            format: Format::MsgPack,
        }
    }

    /// JSON frames (the URL should carry `?format=json`).
    pub fn json(url: impl Into<String>) -> Self {
        WsConnector {
            url: sync_url(&url.into()),
            format: Format::Json,
        }
    }
}

/// Normalizes a relay address to its sync endpoint, so the same setting works for every client:
/// `http(s)://` becomes `ws(s)://`, and a bare host (no path) gets `/v1/sync`.
/// `https://sync.example.com` → `wss://sync.example.com/v1/sync`; a URL that already has a path is kept.
pub fn sync_url(url: &str) -> String {
    let url = url.trim();
    let url = if let Some(rest) = url.strip_prefix("https://") {
        format!("wss://{rest}")
    } else if let Some(rest) = url.strip_prefix("http://") {
        format!("ws://{rest}")
    } else {
        url.to_string()
    };
    let (base, query) = match url.split_once('?') {
        Some((b, q)) => (b.to_string(), Some(q.to_string())),
        None => (url, None),
    };
    let after_scheme = base.split_once("://").map_or(base.as_str(), |(_, r)| r);
    let has_path = after_scheme
        .split_once('/')
        .is_some_and(|(_, p)| !p.is_empty());
    let mut out = if has_path {
        base
    } else {
        format!("{}/v1/sync", base.trim_end_matches('/'))
    };
    if let Some(q) = query {
        out.push('?');
        out.push_str(&q);
    }
    out
}

#[async_trait::async_trait]
impl Connector for WsConnector {
    async fn connect(&self) -> astra_sync::Result<Transport> {
        let (ws, _) = tokio_tungstenite::connect_async(self.url.as_str())
            .await
            .map_err(|e| SyncError::Transport(e.to_string()))?;
        let (sink, stream) = ws.split();
        let format = self.format;
        let sink = sink
            .sink_map_err(|e| SyncError::Transport(e.to_string()))
            .with(move |frame: ClientFrame| async move {
                let bytes = codec::encode(format, &frame)?;
                Ok::<Message, SyncError>(match format {
                    Format::MsgPack => Message::Binary(bytes.into()),
                    Format::Json => {
                        Message::Text(String::from_utf8_lossy(&bytes).into_owned().into())
                    }
                })
            });
        let stream = stream.filter_map(move |msg| async move {
            match msg {
                Ok(Message::Binary(b)) => Some(codec::decode::<ServerFrame>(format, &b)),
                Ok(Message::Text(t)) => {
                    Some(codec::decode::<ServerFrame>(Format::Json, t.as_bytes()))
                }
                Ok(Message::Close(_)) => Some(Err(SyncError::Closed)),
                Ok(_) => None,
                Err(e) => Some(Err(SyncError::Transport(e.to_string()))),
            }
        });
        Ok(Transport {
            sink: Box::pin(sink),
            stream: Box::pin(stream),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::sync_url;

    #[test]
    fn normalizes_relay_addresses() {
        assert_eq!(
            sync_url("https://sync.example.com"),
            "wss://sync.example.com/v1/sync"
        );
        assert_eq!(
            sync_url("http://127.0.0.1:8787/"),
            "ws://127.0.0.1:8787/v1/sync"
        );
        assert_eq!(
            sync_url("ws://127.0.0.1:8787"),
            "ws://127.0.0.1:8787/v1/sync"
        );
        assert_eq!(sync_url("wss://x.dev/v1/sync"), "wss://x.dev/v1/sync");
        assert_eq!(
            sync_url("ws://h:1?format=json"),
            "ws://h:1/v1/sync?format=json"
        );
    }
}
