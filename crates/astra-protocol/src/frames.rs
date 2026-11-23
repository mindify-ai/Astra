//! Sync wire frames (tagged by `t`). See `docs/PROTOCOL.md` ("Sync wire protocol").

use serde::{Deserialize, Serialize};

use crate::entities::Presence;
use crate::ops::Op;

/// Frames sent by a client (or runner node) to a sync server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum ClientFrame {
    /// First frame: identify and ask for every op after `cursor`.
    Hello { workspace_id: String, node_id: String, cursor: u64, token: Option<String> },
    /// Ops created locally.
    Push { ops: Vec<Op> },
    /// This connection's current presence entries (replaces earlier ones).
    Presence { presence: Vec<Presence> },
}

/// Frames sent by a sync server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum ServerFrame {
    /// Accepted the hello; `cursor` is the server's head at that moment.
    Welcome { cursor: u64 },
    /// Ops in server order; `cursor` is the sequence number of the last op.
    Ops { ops: Vec<Op>, cursor: u64 },
    /// These pushed op ids are durably stored.
    Ack { ids: Vec<String> },
    /// Everyone's presence in the workspace.
    Presence { presence: Vec<Presence> },
    /// Fatal or request error.
    Error { message: String },
}
