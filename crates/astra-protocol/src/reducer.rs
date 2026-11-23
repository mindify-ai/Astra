//! The pure LWW reducer shared (rule for rule) with the TypeScript client.
//!
//! Rules (`fixtures/ops-conformance.json`):
//! - Initial entities carry clock `0` (older than any op).
//! - A put sets every field it contains to the put's hlc, except fields whose existing clock is newer.
//! - A patch sets each listed field whose existing clock is older than the patch hlc. Patches to entities
//!   that were never seen are ignored.
//! - A delete with hlc newer than the entity's newest field clock removes it and records a tombstone;
//!   later-arriving ops with older hlc are ignored, newer puts resurrect.
//! - Ops with an id already applied are skipped.
//! - `command` / `commandResult` ops do not change entity state.
//!
//! Convergence detail: a deleted entity's last data and field clocks are retained (hidden). Ops that reach a
//! deleted entity still update the hidden fields by the same LWW rule, and an entity is visible iff it has no
//! tombstone or its newest field clock is newer than the tombstone. This makes the outcome independent of
//! arrival order (a concurrent patch newer than the delete revives the entity whichever arrives first).

use std::collections::{BTreeMap, HashMap, HashSet};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::entities::{Entity, EntityKind};
use crate::hlc::Hlc;
use crate::ops::{Op, OpBody};
use crate::state::State;

/// Per-entity clock record.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EntityClock {
    /// Clock of each field written by an op (absent = zero).
    pub fields: BTreeMap<String, Hlc>,
    /// Newest delete seen.
    pub tombstone: Option<Hlc>,
    /// Last known data of a deleted (invisible) entity.
    pub hidden: Option<Map<String, Value>>,
}

impl EntityClock {
    fn field(&self, name: &str) -> Hlc {
        self.fields.get(name).cloned().unwrap_or_else(Hlc::zero)
    }

    /// Newest field clock (zero if none).
    pub fn newest(&self) -> Hlc {
        self.fields.values().max().cloned().unwrap_or_else(Hlc::zero)
    }

    fn visible(&self) -> bool {
        match &self.tombstone {
            None => true,
            Some(t) => self.newest() > *t,
        }
    }
}

/// Field clocks and applied op ids, kept next to [`State`].
#[derive(Debug, Clone, Default)]
pub struct Clocks {
    /// Clock records by (kind, id).
    pub entities: HashMap<(EntityKind, String), EntityClock>,
    /// Ids of ops already applied.
    pub applied: HashSet<String>,
}

impl Clocks {
    /// Empty clocks.
    pub fn new() -> Clocks {
        Clocks::default()
    }

    /// The clock record of an entity.
    pub fn entity(&self, kind: EntityKind, id: &str) -> Option<&EntityClock> {
        self.entities.get(&(kind, id.to_string()))
    }
}

/// What applying an op did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    /// The op id was already applied.
    Duplicate,
    /// Applied, but no entity record changed (commands, ignored patches, stale writes).
    NoChange,
    /// This entity's data, clocks or visibility changed.
    Changed(EntityKind, String),
}

/// Apply one op, skipping already-applied op ids.
pub fn apply(state: &mut State, clocks: &mut Clocks, op: &Op) -> Effect {
    if !clocks.applied.insert(op.id.clone()) {
        return Effect::Duplicate;
    }
    apply_body(state, clocks, &op.hlc, &op.body)
}

/// Apply an op body without the duplicate check (the caller dedups, e.g. against a database).
pub fn apply_body(state: &mut State, clocks: &mut Clocks, hlc: &Hlc, body: &OpBody) -> Effect {
    match body {
        OpBody::Put { entity } => {
            let kind = entity.kind();
            let id = entity.id().to_string();
            let mut incoming = entity.to_json();
            incoming.remove("kind");
            write_fields(state, clocks, kind, id, hlc, incoming, true)
        }
        OpBody::Patch { entity_kind, id, fields } => {
            let mut incoming: Map<String, Value> = fields.clone().into_iter().collect();
            incoming.remove("kind");
            incoming.remove("id");
            write_fields(state, clocks, *entity_kind, id.clone(), hlc, incoming, false)
        }
        OpBody::Delete { entity_kind, id } => delete(state, clocks, *entity_kind, id.clone(), hlc),
        OpBody::Command { .. } | OpBody::CommandResult { .. } => Effect::NoChange,
    }
}

fn current_data(state: &State, clock: Option<&EntityClock>, kind: EntityKind, id: &str) -> Option<Map<String, Value>> {
    if let Some(e) = state.get(kind, id) {
        let mut m = e.to_json();
        m.remove("kind");
        return Some(m);
    }
    clock.and_then(|c| c.hidden.clone())
}

fn write_fields(
    state: &mut State,
    clocks: &mut Clocks,
    kind: EntityKind,
    id: String,
    hlc: &Hlc,
    incoming: Map<String, Value>,
    is_put: bool,
) -> Effect {
    let key = (kind, id.clone());
    let existing_clock = clocks.entities.get(&key);
    let current = current_data(state, existing_clock, kind, &id);

    let mut clock = existing_clock.cloned().unwrap_or_default();
    let mut data = match current {
        Some(d) => d,
        // A put older than a tombstone is still recorded (hidden): a later concurrent patch may revive it.
        None if is_put => Map::new(),
        // Patch to an entity never seen: ignored.
        None => return Effect::NoChange,
    };

    let mut changed = false;
    for (field, value) in incoming {
        if field == "id" {
            continue;
        }
        if clock.field(&field) < *hlc {
            clock.fields.insert(field.clone(), hlc.clone());
            data.insert(field, value);
            changed = true;
        }
    }
    if !changed {
        return Effect::NoChange;
    }
    data.insert("id".into(), Value::String(id.clone()));

    // Validate the merged record; an invalid write is ignored deterministically.
    let entity = match Entity::from_json(kind, data.clone()) {
        Ok(e) => e,
        Err(_) => return Effect::NoChange,
    };
    if clock.visible() {
        clock.hidden = None;
        state.insert(entity);
    } else {
        state.remove(kind, &id);
        clock.hidden = Some(data);
    }
    clocks.entities.insert(key, clock);
    Effect::Changed(kind, id)
}

fn delete(state: &mut State, clocks: &mut Clocks, kind: EntityKind, id: String, hlc: &Hlc) -> Effect {
    let key = (kind, id.clone());
    let mut clock = clocks.entities.get(&key).cloned().unwrap_or_default();
    if clock.tombstone.as_ref().is_some_and(|t| t >= hlc) {
        return Effect::NoChange;
    }
    clock.tombstone = Some(hlc.clone());
    if !clock.visible() {
        if let Some(entity) = state.remove(kind, &id) {
            let mut m = entity.to_json();
            m.remove("kind");
            clock.hidden = Some(m);
        }
    }
    clocks.entities.insert(key, clock);
    Effect::Changed(kind, id)
}

/// A reducer bundling state and clocks.
#[derive(Debug, Clone, Default)]
pub struct Reducer {
    pub state: State,
    pub clocks: Clocks,
}

impl Reducer {
    /// A reducer over an initial state (initial entities carry clock zero).
    pub fn new(state: State) -> Reducer {
        Reducer { state, clocks: Clocks::new() }
    }

    /// Apply one op.
    pub fn apply(&mut self, op: &Op) -> Effect {
        apply(&mut self.state, &mut self.clocks, op)
    }

    /// Apply ops in order.
    pub fn apply_all<'a>(&mut self, ops: impl IntoIterator<Item = &'a Op>) {
        for op in ops {
            self.apply(op);
        }
    }
}
