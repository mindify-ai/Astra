//! Ops and commands. See `docs/PROTOCOL.md` ("Ops").

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::entities::{Entity, EntityKind};
use crate::hlc::Hlc;

/// A side effect addressed to a runner node (tagged by `type`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum Command {
    StartMission {
        mission_id: String,
    },
    Approve {
        approval_id: String,
    },
    Deny {
        approval_id: String,
    },
    SendMessage {
        mission_id: String,
        to: Option<String>,
        body: String,
    },
    TakeSteering {
        mission_agent_id: String,
    },
    ReleaseSteering {
        mission_agent_id: String,
    },
    Pause {
        mission_agent_id: String,
    },
    Resume {
        mission_agent_id: String,
    },
    Merge {
        result_id: String,
    },
    ConnectAgent {
        agent_id: String,
    },
    TestAgent {
        agent_id: String,
    },
    /// Run a mission agent's role on a different connected agent (another vendor or harness). A running
    /// session is stopped and its task handed over in the same worktree.
    SwitchAgent {
        mission_agent_id: String,
        agent_id: String,
    },
    /// Start a mission agent the client just put on a mission that is already under way.
    AddAgent {
        mission_agent_id: String,
    },
    /// Take an agent off its mission: its session stops and the mission carries on without it.
    RemoveAgent {
        mission_agent_id: String,
    },
}

/// The payload of an op (tagged by `type`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum OpBody {
    /// Create or replace an entity (LWW per field).
    Put { entity: Entity },
    /// Update some fields (LWW per field). Ignored if the entity is unknown.
    Patch {
        entity_kind: EntityKind,
        id: String,
        fields: BTreeMap<String, serde_json::Value>,
    },
    /// Tombstone an entity.
    Delete { entity_kind: EntityKind, id: String },
    /// A side effect for runner node `node`.
    Command { node: String, command: Command },
    /// Outcome of a command, appended by the node that executed it.
    CommandResult {
        command_op: String,
        ok: bool,
        error: Option<String>,
    },
}

impl OpBody {
    /// A patch body built from `(field, value)` pairs.
    pub fn patch<I, K>(entity_kind: EntityKind, id: impl Into<String>, fields: I) -> OpBody
    where
        I: IntoIterator<Item = (K, serde_json::Value)>,
        K: Into<String>,
    {
        OpBody::Patch {
            entity_kind,
            id: id.into(),
            fields: fields.into_iter().map(|(k, v)| (k.into(), v)).collect(),
        }
    }

    /// A put body.
    pub fn put(entity: impl Into<Entity>) -> OpBody {
        OpBody::Put {
            entity: entity.into(),
        }
    }
}

/// An op: `{id, hlc, actor, body}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Op {
    /// A ULID.
    pub id: String,
    /// Hybrid logical clock timestamp.
    pub hlc: Hlc,
    /// Person id or node id that made the op.
    pub actor: String,
    pub body: OpBody,
}

impl Op {
    /// A new op with a fresh ULID.
    pub fn new(hlc: Hlc, actor: impl Into<String>, body: OpBody) -> Op {
        Op {
            id: new_op_id(),
            hlc,
            actor: actor.into(),
            body,
        }
    }

    /// The command and target node if this is a command op.
    pub fn as_command(&self) -> Option<(&str, &Command)> {
        match &self.body {
            OpBody::Command { node, command } => Some((node.as_str(), command)),
            _ => None,
        }
    }
}

/// A fresh ULID string.
pub fn new_op_id() -> String {
    ulid::Ulid::generate().to_string()
}
