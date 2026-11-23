//! Frame encoding. MessagePack uses named fields (maps), so frames are self-describing like JSON.

use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::{Result, SyncError};

/// Wire format of a connection.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Format {
    /// MessagePack in binary WebSocket messages (default).
    #[default]
    MsgPack,
    /// JSON in text WebSocket messages (`?format=json`).
    Json,
}

impl Format {
    /// Parse the `format` query parameter.
    pub fn from_query(value: Option<&str>) -> Format {
        match value {
            Some("json") => Format::Json,
            _ => Format::MsgPack,
        }
    }
}

/// Encode a frame.
pub fn encode<T: Serialize>(format: Format, frame: &T) -> Result<Vec<u8>> {
    match format {
        Format::MsgPack => {
            rmp_serde::to_vec_named(frame).map_err(|e| SyncError::Codec(e.to_string()))
        }
        Format::Json => serde_json::to_vec(frame).map_err(|e| SyncError::Codec(e.to_string())),
    }
}

/// Decode a frame.
pub fn decode<T: DeserializeOwned>(format: Format, bytes: &[u8]) -> Result<T> {
    match format {
        Format::MsgPack => {
            rmp_serde::from_slice(bytes).map_err(|e| SyncError::Codec(e.to_string()))
        }
        Format::Json => serde_json::from_slice(bytes).map_err(|e| SyncError::Codec(e.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use astra_protocol::{ClientFrame, ServerFrame};

    #[test]
    fn both_formats_roundtrip() {
        for f in [Format::MsgPack, Format::Json] {
            let frame = ClientFrame::Hello {
                workspace_id: "w".into(),
                node_id: "n".into(),
                cursor: 9,
                token: None,
            };
            assert_eq!(
                decode::<ClientFrame>(f, &encode(f, &frame).expect("enc")).expect("dec"),
                frame
            );
            let frame = ServerFrame::Ack {
                ids: vec!["a".into()],
            };
            assert_eq!(
                decode::<ServerFrame>(f, &encode(f, &frame).expect("enc")).expect("dec"),
                frame
            );
        }
        assert!(decode::<ServerFrame>(Format::Json, b"{\"t\":\"nope\"}").is_err());
    }
}
