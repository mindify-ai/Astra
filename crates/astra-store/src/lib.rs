//! SQLite (WAL) persistence for an Astra node: the op log, field clocks and the projected entity state.
//!
//! The in-memory [`State`] is always the reducer output of every op in the log (plus seeded entities).

use std::collections::HashSet;
use std::path::Path;

use astra_protocol::{
    Clocks, Effect, Entity, EntityClock, EntityKind, Hlc, Op, OpBody, Seed, State, apply_body,
};
use rusqlite::{Connection, OptionalExtension, Transaction, params};

/// Store errors.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("sqlite: {0}")]
    Sql(#[from] rusqlite::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("corrupt store: {0}")]
    Corrupt(String),
}

/// Result alias.
pub type Result<T> = std::result::Result<T, StoreError>;

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS meta (
  key   TEXT PRIMARY KEY,
  value TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS ops (
  seq          INTEGER PRIMARY KEY AUTOINCREMENT,
  id           TEXT NOT NULL UNIQUE,
  hlc          TEXT NOT NULL,
  actor        TEXT NOT NULL,
  body         TEXT NOT NULL,
  local        INTEGER NOT NULL,
  acked        INTEGER NOT NULL,
  server_seq   INTEGER,
  command_node TEXT,
  command_op   TEXT
);
CREATE INDEX IF NOT EXISTS ops_pending ON ops(acked, local);
CREATE INDEX IF NOT EXISTS ops_command_node ON ops(command_node);
CREATE INDEX IF NOT EXISTS ops_command_op ON ops(command_op);
CREATE TABLE IF NOT EXISTS entities (
  kind      TEXT NOT NULL,
  id        TEXT NOT NULL,
  json      TEXT,
  visible   INTEGER NOT NULL,
  tombstone TEXT,
  PRIMARY KEY (kind, id)
);
CREATE TABLE IF NOT EXISTS field_clocks (
  kind  TEXT NOT NULL,
  id    TEXT NOT NULL,
  field TEXT NOT NULL,
  hlc   TEXT NOT NULL,
  PRIMARY KEY (kind, id, field)
);
"#;

/// Where an appended op came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    /// Made on this node (or by a local client): must be pushed upstream.
    Local,
    /// Received from upstream: already durable there.
    Remote,
}

/// The node's persistent store.
pub struct Store {
    conn: Connection,
    state: State,
    clocks: Clocks,
}

impl std::fmt::Debug for Store {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Store").field("entities", &self.state.len()).finish()
    }
}

impl Store {
    /// Open (or create) a store at `path`.
    pub fn open(path: impl AsRef<Path>) -> Result<Store> {
        let conn = Connection::open(path)?;
        Self::init(conn)
    }

    /// An in-memory store (tests, ephemeral clients).
    pub fn open_in_memory() -> Result<Store> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Store> {
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.execute_batch(SCHEMA)?;
        let mut store = Store { conn, state: State::default(), clocks: Clocks::new() };
        store.reload()?;
        Ok(store)
    }

    /// Rebuild the in-memory state and clocks from disk.
    pub fn reload(&mut self) -> Result<()> {
        let workspace_id = self.meta("workspace_id")?.unwrap_or_default();
        let me = self.meta("me")?.unwrap_or_default();
        let mut state = State::new(workspace_id, me);
        let mut clocks = Clocks::new();
        {
            let mut stmt = self.conn.prepare("SELECT kind, id, json, visible, tombstone FROM entities")?;
            let rows = stmt.query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, Option<String>>(2)?,
                    r.get::<_, bool>(3)?,
                    r.get::<_, Option<String>>(4)?,
                ))
            })?;
            for row in rows {
                let (kind, id, json, visible, tombstone) = row?;
                let kind = EntityKind::parse(&kind).ok_or_else(|| StoreError::Corrupt(format!("kind {kind}")))?;
                let data: Option<serde_json::Map<String, serde_json::Value>> =
                    json.map(|j| serde_json::from_str(&j)).transpose()?;
                let tombstone = tombstone
                    .map(|t| t.parse::<Hlc>().map_err(|e| StoreError::Corrupt(e.to_string())))
                    .transpose()?;
                let mut clock = EntityClock { tombstone, ..EntityClock::default() };
                match (visible, data) {
                    (true, Some(d)) => state.insert(Entity::from_json(kind, d)?),
                    (false, d) => clock.hidden = d,
                    (true, None) => return Err(StoreError::Corrupt(format!("{kind}/{id} has no data"))),
                }
                clocks.entities.insert((kind, id), clock);
            }
        }
        {
            let mut stmt = self.conn.prepare("SELECT kind, id, field, hlc FROM field_clocks")?;
            let rows = stmt.query_map([], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, String>(3)?))
            })?;
            for row in rows {
                let (kind, id, field, hlc) = row?;
                let kind = EntityKind::parse(&kind).ok_or_else(|| StoreError::Corrupt(format!("kind {kind}")))?;
                let hlc: Hlc = hlc.parse().map_err(|e: astra_protocol::HlcParseError| StoreError::Corrupt(e.to_string()))?;
                clocks.entities.entry((kind, id)).or_default().fields.insert(field, hlc);
            }
        }
        self.state = state;
        self.clocks = clocks;
        Ok(())
    }

    /// The current projected state (presence is not stored and is always empty here).
    pub fn state(&self) -> &State {
        &self.state
    }

    /// Clock record of an entity.
    pub fn entity_clock(&self, kind: EntityKind, id: &str) -> Option<&EntityClock> {
        self.clocks.entity(kind, id)
    }

    /// Read a metadata value.
    pub fn meta(&self, key: &str) -> Result<Option<String>> {
        Ok(self.conn.query_row("SELECT value FROM meta WHERE key = ?1", [key], |r| r.get(0)).optional()?)
    }

    /// Write a metadata value.
    pub fn set_meta(&mut self, key: &str, value: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO meta(key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    /// Set the workspace id and local person id (stored in meta and mirrored into the state).
    pub fn set_identity(&mut self, workspace_id: &str, me: &str) -> Result<()> {
        self.set_meta("workspace_id", workspace_id)?;
        self.set_meta("me", me)?;
        self.state.workspace_id = workspace_id.to_string();
        self.state.me = me.to_string();
        Ok(())
    }

    /// The server cursor (last server sequence number received).
    pub fn cursor(&self) -> Result<u64> {
        Ok(self.meta("cursor")?.and_then(|c| c.parse().ok()).unwrap_or(0))
    }

    /// Store the server cursor.
    pub fn set_cursor(&mut self, cursor: u64) -> Result<()> {
        self.set_meta("cursor", &cursor.to_string())
    }

    /// Insert seed entities with clock zero (ops always win over them). Skips entities already known.
    pub fn seed(&mut self, seed: &Seed) -> Result<usize> {
        let tx = self.conn.transaction()?;
        let mut n = 0;
        for entity in &seed.entities {
            let key = (entity.kind(), entity.id().to_string());
            if self.clocks.entities.contains_key(&key) || self.state.contains(key.0, &key.1) {
                continue;
            }
            self.state.insert(entity.clone());
            self.clocks.entities.insert(key.clone(), EntityClock::default());
            persist_entity(&tx, &self.state, &self.clocks, key.0, &key.1)?;
            n += 1;
        }
        tx.commit()?;
        if self.state.workspace_id.is_empty() {
            let (ws, me) = (seed.workspace_id.clone(), seed.me.clone());
            self.set_identity(&ws, &me)?;
        }
        Ok(n)
    }

    /// Append ops (idempotent by id) and apply them. Returns the ops that were new, in order.
    ///
    /// Remote ops that are already present (e.g. the echo of a local op) are marked acknowledged.
    pub fn append(&mut self, ops: &[Op], origin: Origin) -> Result<Vec<Op>> {
        let result = self.append_inner(ops, origin);
        if result.is_err() {
            // Keep memory consistent with disk after a failed transaction.
            self.reload()?;
        }
        result
    }

    fn append_inner(&mut self, ops: &[Op], origin: Origin) -> Result<Vec<Op>> {
        let tx = self.conn.transaction()?;
        let mut fresh = Vec::new();
        let mut seen = HashSet::new();
        for op in ops {
            if !seen.insert(op.id.as_str()) {
                continue;
            }
            let (command_node, command_op) = match &op.body {
                OpBody::Command { node, .. } => (Some(node.as_str()), None),
                OpBody::CommandResult { command_op, .. } => (None, Some(command_op.as_str())),
                _ => (None, None),
            };
            let local = origin == Origin::Local;
            let exists = tx.query_row("SELECT 1 FROM ops WHERE id = ?1", [&op.id], |_| Ok(())).optional()?.is_some();
            let inserted = if exists {
                0
            } else {
                tx.execute(
                "INSERT INTO ops (id, hlc, actor, body, local, acked, command_node, command_op)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    op.id,
                    op.hlc.to_string(),
                    op.actor,
                    serde_json::to_string(&op.body)?,
                    local,
                    !local,
                    command_node,
                    command_op
                ],
            )?
            };
            if inserted == 0 {
                if origin == Origin::Remote {
                    tx.execute("UPDATE ops SET acked = 1 WHERE id = ?1", [&op.id])?;
                }
                continue;
            }
            if let Effect::Changed(kind, id) = apply_body(&mut self.state, &mut self.clocks, &op.hlc, &op.body) {
                persist_entity(&tx, &self.state, &self.clocks, kind, &id)?;
            }
            fresh.push(op.clone());
        }
        tx.commit()?;
        Ok(fresh)
    }

    /// Whether an op id is in the log.
    pub fn contains_op(&self, id: &str) -> Result<bool> {
        Ok(self.conn.query_row("SELECT 1 FROM ops WHERE id = ?1", [id], |_| Ok(())).optional()?.is_some())
    }

    /// The local sequence number of the newest op (0 if empty).
    pub fn head_seq(&self) -> Result<u64> {
        let seq: Option<i64> = self.conn.query_row("SELECT MAX(seq) FROM ops", [], |r| r.get(0))?;
        Ok(seq.and_then(|s| u64::try_from(s).ok()).unwrap_or(0))
    }

    /// Ops with local sequence number greater than `local_seq`, oldest first, at most `limit`.
    pub fn ops_since(&self, local_seq: u64, limit: usize) -> Result<Vec<(u64, Op)>> {
        let mut stmt = self
            .conn
            .prepare("SELECT seq, id, hlc, actor, body FROM ops WHERE seq > ?1 ORDER BY seq LIMIT ?2")?;
        let rows = stmt.query_map(params![to_i64(local_seq), to_i64(limit as u64)], row_to_op)?;
        rows.map(|r| r.map_err(StoreError::from).and_then(|(seq, op)| Ok((seq, op?)))).collect()
    }

    /// Local ops not yet acknowledged by the server, oldest first.
    pub fn pending_outbound(&self) -> Result<Vec<Op>> {
        let mut stmt = self
            .conn
            .prepare("SELECT seq, id, hlc, actor, body FROM ops WHERE local = 1 AND acked = 0 ORDER BY seq")?;
        let rows = stmt.query_map([], row_to_op)?;
        rows.map(|r| r.map_err(StoreError::from).and_then(|(_, op)| op)).collect()
    }

    /// Mark ops as acknowledged by the server (optionally recording their server sequence).
    pub fn mark_acked(&mut self, ids: &[String], server_seq: Option<u64>) -> Result<()> {
        let tx = self.conn.transaction()?;
        for id in ids {
            tx.execute(
                "UPDATE ops SET acked = 1, server_seq = COALESCE(?2, server_seq) WHERE id = ?1",
                params![id, server_seq.map(to_i64)],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Whether a `commandResult` for `command_op` exists.
    pub fn has_command_result(&self, command_op: &str) -> Result<bool> {
        Ok(self
            .conn
            .query_row("SELECT 1 FROM ops WHERE command_op = ?1 LIMIT 1", [command_op], |_| Ok(()))
            .optional()?
            .is_some())
    }

    /// Command ops addressed to `node` that have no `commandResult` yet, oldest first.
    pub fn unexecuted_commands(&self, node: &str) -> Result<Vec<Op>> {
        let mut stmt = self.conn.prepare(
            "SELECT o.seq, o.id, o.hlc, o.actor, o.body FROM ops o
             WHERE o.command_node = ?1
               AND NOT EXISTS (SELECT 1 FROM ops r WHERE r.command_op = o.id)
             ORDER BY o.seq",
        )?;
        let rows = stmt.query_map([node], row_to_op)?;
        rows.map(|r| r.map_err(StoreError::from).and_then(|(_, op)| op)).collect()
    }

    /// Number of ops in the log.
    pub fn op_count(&self) -> Result<u64> {
        let n: i64 = self.conn.query_row("SELECT COUNT(*) FROM ops", [], |r| r.get(0))?;
        Ok(u64::try_from(n).unwrap_or(0))
    }
}

fn to_i64(v: u64) -> i64 {
    i64::try_from(v).unwrap_or(i64::MAX)
}

type OpRow = (u64, Result<Op>);

fn row_to_op(r: &rusqlite::Row<'_>) -> rusqlite::Result<OpRow> {
    let seq: i64 = r.get(0)?;
    let id: String = r.get(1)?;
    let hlc: String = r.get(2)?;
    let actor: String = r.get(3)?;
    let body: String = r.get(4)?;
    let op = (|| -> Result<Op> {
        Ok(Op {
            id,
            hlc: hlc.parse().map_err(|e: astra_protocol::HlcParseError| StoreError::Corrupt(e.to_string()))?,
            actor,
            body: serde_json::from_str(&body)?,
        })
    })();
    Ok((u64::try_from(seq).unwrap_or(0), op))
}

fn persist_entity(tx: &Transaction<'_>, state: &State, clocks: &Clocks, kind: EntityKind, id: &str) -> Result<()> {
    let clock = clocks.entity(kind, id).cloned().unwrap_or_default();
    let (json, visible) = match state.get(kind, id) {
        Some(e) => {
            let mut m = e.to_json();
            m.remove("kind");
            (Some(serde_json::to_string(&m)?), true)
        }
        None => (clock.hidden.as_ref().map(serde_json::to_string).transpose()?, false),
    };
    tx.execute(
        "INSERT INTO entities (kind, id, json, visible, tombstone) VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(kind, id) DO UPDATE SET json = excluded.json, visible = excluded.visible,
           tombstone = excluded.tombstone",
        params![kind.as_str(), id, json, visible, clock.tombstone.as_ref().map(|t| t.to_string())],
    )?;
    for (field, hlc) in &clock.fields {
        tx.execute(
            "INSERT INTO field_clocks (kind, id, field, hlc) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(kind, id, field) DO UPDATE SET hlc = excluded.hlc",
            params![kind.as_str(), id, field, hlc.to_string()],
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use astra_protocol::{Command, Person, Reducer};

    fn person(id: &str, name: &str) -> Entity {
        Entity::Person(Person { id: id.into(), name: name.into(), initials: name[..1].into() })
    }

    fn op(id: &str, hlc: u64, body: OpBody) -> Op {
        Op { id: id.into(), hlc: Hlc::new(hlc, 0, "n1"), actor: "n1".into(), body }
    }

    #[test]
    fn append_is_idempotent_and_tracks_outbound() {
        let mut s = Store::open_in_memory().expect("open");
        let a = op("a", 10, OpBody::put(Person { id: "p".into(), name: "A".into(), initials: "A".into() }));
        let fresh = s.append(std::slice::from_ref(&a), Origin::Local).expect("append");
        assert_eq!(fresh.len(), 1);
        assert!(s.append(std::slice::from_ref(&a), Origin::Local).expect("append").is_empty());
        assert_eq!(s.pending_outbound().expect("pending"), vec![a.clone()]);
        s.mark_acked(&["a".into()], Some(7)).expect("ack");
        assert!(s.pending_outbound().expect("pending").is_empty());
        // Remote echo of a pending local op acks it.
        let b = op("b", 11, OpBody::patch(EntityKind::Person, "p", [("name", "B".into())]));
        s.append(std::slice::from_ref(&b), Origin::Local).expect("append");
        assert!(s.append(std::slice::from_ref(&b), Origin::Remote).expect("append").is_empty());
        assert!(s.pending_outbound().expect("pending").is_empty());
        assert_eq!(s.head_seq().expect("head"), 2);
        assert_eq!(s.ops_since(1, 10).expect("since").len(), 1);
    }

    #[test]
    fn reopen_from_disk_restores_state_and_clocks() {
        let dir = tempfile::tempdir().expect("tmp");
        let path = dir.path().join("astra.db");
        let ops = vec![
            op("1", 100, OpBody::Put { entity: person("p-1", "One") }),
            op("2", 300, OpBody::patch(EntityKind::Person, "p-1", [("name", "Uno".into())])),
            op("3", 100, OpBody::Put { entity: person("p-2", "Two") }),
            op("4", 200, OpBody::Delete { entity_kind: EntityKind::Person, id: "p-2".into() }),
            op(
                "5",
                400,
                OpBody::Command {
                    node: "n-9".into(),
                    command: Command::StartMission { mission_id: "m".into() },
                },
            ),
        ];
        {
            let mut s = Store::open(&path).expect("open");
            s.set_identity("ws", "p-1").expect("identity");
            s.append(&ops, Origin::Remote).expect("append");
            s.set_cursor(5).expect("cursor");
        }
        let mut s = Store::open(&path).expect("reopen");
        assert_eq!(s.cursor().expect("cursor"), 5);
        assert_eq!(s.state().workspace_id, "ws");
        let mut reference = Reducer::new(State::new("ws", "p-1"));
        reference.apply_all(&ops);
        assert_eq!(s.state(), &reference.state);
        // Clocks survived: an older put does not overwrite the newer patch, a late patch stays hidden.
        s.append(
            &[
                op("6", 250, OpBody::Put { entity: person("p-1", "Old") }),
                op("7", 150, OpBody::patch(EntityKind::Person, "p-2", [("name", "late".into())])),
            ],
            Origin::Remote,
        )
        .expect("append");
        assert!(matches!(s.state().get(EntityKind::Person, "p-1"), Some(Entity::Person(p)) if p.name == "Uno"));
        assert!(s.state().get(EntityKind::Person, "p-2").is_none());
        assert_eq!(s.unexecuted_commands("n-9").expect("cmds").len(), 1);
        s.append(
            &[op("8", 500, OpBody::CommandResult { command_op: "5".into(), ok: true, error: None })],
            Origin::Local,
        )
        .expect("append");
        assert!(s.has_command_result("5").expect("has"));
        assert!(s.unexecuted_commands("n-9").expect("cmds").is_empty());
    }

    #[test]
    fn seed_loads_with_zero_clocks() {
        let mut s = Store::open_in_memory().expect("open");
        let seed = Seed {
            workspace_id: "ws".into(),
            me: "p".into(),
            presence: vec![],
            entities: vec![person("p-a", "Ann")],
        };
        assert_eq!(s.seed(&seed).expect("seed"), 1);
        assert_eq!(s.seed(&seed).expect("seed"), 0);
        s.append(&[op("x", 1, OpBody::patch(EntityKind::Person, "p-a", [("name", "Anna".into())]))], Origin::Local)
            .expect("append");
        assert!(matches!(s.state().get(EntityKind::Person, "p-a"), Some(Entity::Person(p)) if p.name == "Anna"));
        assert_eq!(s.state().me, "p");
    }
}
