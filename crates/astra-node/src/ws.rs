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
    /// MessagePack frames to `url`.
    pub fn new(url: impl Into<String>) -> Self {
        WsConnector { url: url.into(), format: Format::MsgPack }
    }

    /// JSON frames (the URL should carry `?format=json`).
    pub fn json(url: impl Into<String>) -> Self {
        WsConnector { url: url.into(), format: Format::Json }
    }
}

#[async_trait::async_trait]
impl Connector for WsConnector {
    async fn connect(&self) -> astra_sync::Result<Transport> {
        let (ws, _) = tokio_tungstenite::connect_async(self.url.as_str())
            .await
            .map_err(|e| SyncError::Transport(e.to_string()))?;
        let (sink, stream) = ws.split();
        let format = self.format;
        let sink = sink.sink_map_err(|e| SyncError::Transport(e.to_string())).with(move |frame: ClientFrame| async move {
            let bytes = codec::encode(format, &frame)?;
            Ok::<Message, SyncError>(match format {
                Format::MsgPack => Message::Binary(bytes.into()),
                Format::Json => Message::Text(String::from_utf8_lossy(&bytes).into_owned().into()),
            })
        });
        let stream = stream.filter_map(move |msg| async move {
            match msg {
                Ok(Message::Binary(b)) => Some(codec::decode::<ServerFrame>(format, &b)),
                Ok(Message::Text(t)) => Some(codec::decode::<ServerFrame>(Format::Json, t.as_bytes())),
                Ok(Message::Close(_)) => Some(Err(SyncError::Closed)),
                Ok(_) => None,
                Err(e) => Some(Err(SyncError::Transport(e.to_string()))),
            }
        });
        Ok(Transport { sink: Box::pin(sink), stream: Box::pin(stream) })
    }
}
