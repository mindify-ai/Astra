//! The normalized client state: one map per entity kind, plus ephemeral presence.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize, Serializer};

use crate::entities::*;

/// Serialize a `Record<id, Entity>` map, adding each entity's `kind`.
fn ser_tagged<S, T>(map: &BTreeMap<String, T>, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
    T: Clone + Into<Entity>,
{
    use serde::ser::SerializeMap;
    let mut out = serializer.serialize_map(Some(map.len()))?;
    for (id, value) in map {
        let entity: Entity = value.clone().into();
        out.serialize_entry(id, &entity)?;
    }
    out.end()
}

/// Normalized state. Entity maps are `Record<id, Entity>`; every entity carries its `kind`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct State {
    pub workspace_id: String,
    pub me: String,
    #[serde(serialize_with = "ser_tagged", default)]
    pub people: BTreeMap<String, Person>,
    #[serde(serialize_with = "ser_tagged", default)]
    pub repositories: BTreeMap<String, Repository>,
    #[serde(serialize_with = "ser_tagged", default)]
    pub agents: BTreeMap<String, Agent>,
    #[serde(serialize_with = "ser_tagged", default)]
    pub missions: BTreeMap<String, Mission>,
    #[serde(serialize_with = "ser_tagged", default)]
    pub mission_agents: BTreeMap<String, MissionAgent>,
    #[serde(serialize_with = "ser_tagged", default)]
    pub tasks: BTreeMap<String, Task>,
    #[serde(serialize_with = "ser_tagged", default)]
    pub messages: BTreeMap<String, Message>,
    #[serde(serialize_with = "ser_tagged", default)]
    pub approvals: BTreeMap<String, Approval>,
    #[serde(serialize_with = "ser_tagged", default)]
    pub results: BTreeMap<String, MissionResult>,
    #[serde(serialize_with = "ser_tagged", default)]
    pub merges: BTreeMap<String, Merge>,
    #[serde(serialize_with = "ser_tagged", default)]
    pub memberships: BTreeMap<String, Membership>,
    #[serde(serialize_with = "ser_tagged", default)]
    pub agent_types: BTreeMap<String, AgentType>,
    #[serde(default)]
    pub presence: Vec<Presence>,
}

macro_rules! dispatch {
    ($self:ident, $kind:expr, |$map:ident| $body:expr) => {
        match $kind {
            EntityKind::Person => {
                let $map = &$self.people;
                $body
            }
            EntityKind::Repository => {
                let $map = &$self.repositories;
                $body
            }
            EntityKind::Agent => {
                let $map = &$self.agents;
                $body
            }
            EntityKind::Mission => {
                let $map = &$self.missions;
                $body
            }
            EntityKind::MissionAgent => {
                let $map = &$self.mission_agents;
                $body
            }
            EntityKind::Task => {
                let $map = &$self.tasks;
                $body
            }
            EntityKind::Message => {
                let $map = &$self.messages;
                $body
            }
            EntityKind::Approval => {
                let $map = &$self.approvals;
                $body
            }
            EntityKind::Result => {
                let $map = &$self.results;
                $body
            }
            EntityKind::Merge => {
                let $map = &$self.merges;
                $body
            }
            EntityKind::Membership => {
                let $map = &$self.memberships;
                $body
            }
            EntityKind::AgentType => {
                let $map = &$self.agent_types;
                $body
            }
        }
    };
}

macro_rules! dispatch_mut {
    ($self:ident, $kind:expr, |$map:ident| $body:expr) => {
        match $kind {
            EntityKind::Person => {
                let $map = &mut $self.people;
                $body
            }
            EntityKind::Repository => {
                let $map = &mut $self.repositories;
                $body
            }
            EntityKind::Agent => {
                let $map = &mut $self.agents;
                $body
            }
            EntityKind::Mission => {
                let $map = &mut $self.missions;
                $body
            }
            EntityKind::MissionAgent => {
                let $map = &mut $self.mission_agents;
                $body
            }
            EntityKind::Task => {
                let $map = &mut $self.tasks;
                $body
            }
            EntityKind::Message => {
                let $map = &mut $self.messages;
                $body
            }
            EntityKind::Approval => {
                let $map = &mut $self.approvals;
                $body
            }
            EntityKind::Result => {
                let $map = &mut $self.results;
                $body
            }
            EntityKind::Merge => {
                let $map = &mut $self.merges;
                $body
            }
            EntityKind::Membership => {
                let $map = &mut $self.memberships;
                $body
            }
            EntityKind::AgentType => {
                let $map = &mut $self.agent_types;
                $body
            }
        }
    };
}

impl State {
    /// An empty state.
    pub fn new(workspace_id: impl Into<String>, me: impl Into<String>) -> State {
        State {
            workspace_id: workspace_id.into(),
            me: me.into(),
            ..State::default()
        }
    }

    /// Whether an entity exists.
    pub fn contains(&self, kind: EntityKind, id: &str) -> bool {
        dispatch!(self, kind, |m| m.contains_key(id))
    }

    /// A copy of an entity.
    pub fn get(&self, kind: EntityKind, id: &str) -> Option<Entity> {
        dispatch!(self, kind, |m| m.get(id).cloned().map(Into::into))
    }

    /// Insert or replace an entity.
    pub fn insert(&mut self, entity: Entity) {
        match entity {
            Entity::Person(e) => {
                self.people.insert(e.id.clone(), e);
            }
            Entity::Repository(e) => {
                self.repositories.insert(e.id.clone(), e);
            }
            Entity::Agent(e) => {
                self.agents.insert(e.id.clone(), e);
            }
            Entity::Mission(e) => {
                self.missions.insert(e.id.clone(), e);
            }
            Entity::MissionAgent(e) => {
                self.mission_agents.insert(e.id.clone(), e);
            }
            Entity::Task(e) => {
                self.tasks.insert(e.id.clone(), e);
            }
            Entity::Message(e) => {
                self.messages.insert(e.id.clone(), e);
            }
            Entity::Approval(e) => {
                self.approvals.insert(e.id.clone(), e);
            }
            Entity::Result(e) => {
                self.results.insert(e.id.clone(), e);
            }
            Entity::Merge(e) => {
                self.merges.insert(e.id.clone(), e);
            }
            Entity::Membership(e) => {
                self.memberships.insert(e.id.clone(), e);
            }
            Entity::AgentType(e) => {
                self.agent_types.insert(e.id.clone(), e);
            }
        }
    }

    /// Remove an entity, returning it.
    pub fn remove(&mut self, kind: EntityKind, id: &str) -> Option<Entity> {
        dispatch_mut!(self, kind, |m| m.remove(id).map(Into::into))
    }

    /// Every entity, in kind then id order.
    pub fn entities(&self) -> Vec<Entity> {
        let mut out = Vec::new();
        for kind in EntityKind::ALL {
            dispatch!(self, kind, |m| out
                .extend(m.values().cloned().map(Into::into)));
        }
        out
    }

    /// Number of entities of all kinds.
    pub fn len(&self) -> usize {
        EntityKind::ALL
            .iter()
            .map(|k| dispatch!(self, *k, |m| m.len()))
            .sum()
    }

    /// True when there are no entities.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Build a state from a seed document.
    pub fn from_seed(seed: Seed) -> State {
        let mut state = State::new(seed.workspace_id, seed.me);
        state.presence = seed.presence;
        for e in seed.entities {
            state.insert(e);
        }
        state
    }

    /// Flatten into a seed document (entities in kind then id order).
    pub fn to_seed(&self) -> Seed {
        Seed {
            workspace_id: self.workspace_id.clone(),
            me: self.me.clone(),
            presence: self.presence.clone(),
            entities: self.entities(),
        }
    }
}

/// The seed document format of `fixtures/seed.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Seed {
    pub workspace_id: String,
    pub me: String,
    pub presence: Vec<Presence>,
    pub entities: Vec<Entity>,
}

/// What `snapshot()` returns to local clients.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snapshot {
    pub state: State,
    pub cursor: u64,
}
