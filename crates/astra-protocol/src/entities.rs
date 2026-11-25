//! Entity and value types. See `docs/PROTOCOL.md` ("Entities").

use serde::{Deserialize, Serialize};

macro_rules! string_enum {
    ($(#[$m:meta])* $name:ident { $($variant:ident),+ $(,)? }) => {
        $(#[$m])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        #[serde(rename_all = "camelCase")]
        pub enum $name { $($variant),+ }
    };
}

string_enum!(
    /// Where a repository is hosted.
    RepoSource { Github, Gitlab, Local }
);
string_enum!(
    /// Agent catalogue category.
    AgentCategory { CodingAgent, Harness, Protocol, Local }
);
string_enum!(
    /// Where an agent runs.
    RunsOn { Local, Server }
);
string_enum!(
    /// When an agent's commands need approval.
    CommandPolicy { AskMe, AskSteerers, Auto }
);
string_enum!(
    /// Connection status of an agent.
    AgentStatus { Connected, SettingUp, NotConnected, Offline }
);
string_enum!(
    /// How a mission distributes work.
    MissionMode { Split, Race }
);
string_enum!(
    /// Mission lifecycle.
    MissionStatus { Draft, Working, NeedsYou, Done, Merged }
);
string_enum!(
    /// Who may decide approvals in a mission.
    ApprovalPolicy { Owner, Steerers }
);
string_enum!(
    /// Role of an agent inside a mission.
    Role { Planner, Builder, Tester, Reviewer, Writer }
);
string_enum!(
    /// Status of a mission agent.
    MissionAgentStatus { Idle, Queued, Working, NeedsYou, Done, Offline }
);
string_enum!(
    /// Board column of a task.
    TaskColumn { Queued, Working, NeedsYou, Done }
);
string_enum!(
    /// Approval decision state.
    ApprovalState { Pending, Approved, Denied }
);
string_enum!(
    /// CI / check outcome.
    Checks { Passed, Failed, Pending, None }
);
string_enum!(
    /// Mission membership access level.
    Access { Owner, Steer, Watch }
);
string_enum!(
    /// State of a plan step.
    PlanState { Done, Active, Pending }
);
string_enum!(
    /// Kind of a unified-diff line.
    DiffLineKind { Ctx, Add, Del }
);
string_enum!(
    /// Model routing policy of the built-in agent.
    ProviderPolicy { Auto, Cloud, Local }
);
string_enum!(
    /// Relay connectivity reported to local clients.
    RelayStatus { Connected, Disconnected, Disabled }
);

/// How Astra talks to an agent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum Adapter {
    /// Agent Client Protocol over the child's stdio.
    Acp { command: String, args: Vec<String> },
    /// LangGraph Server HTTP API.
    Langgraph { endpoint: String, graph: String },
    /// Any command: prompt on stdin, text on stdout.
    Command { command: String, args: Vec<String> },
    /// The built-in Astra agent.
    Builtin {
        provider: ProviderPolicy,
        model: Option<String>,
    },
}

/// Author of a message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum Author {
    Person { person_id: String },
    Agent { mission_agent_id: String },
}

/// One activity line (`edit src/x.ts`, `run pnpm test`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogLine {
    pub verb: String,
    pub target: String,
}

/// One step of an agent plan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanStep {
    pub text: String,
    pub state: PlanState,
}

/// Diff size summary.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiffStats {
    pub files: u32,
    pub additions: u32,
    pub deletions: u32,
}

/// One line of a file diff.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiffLine {
    pub kind: DiffLineKind,
    pub text: String,
}

/// The diff of one file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiffFile {
    pub path: String,
    pub lines: Vec<DiffLine>,
}

/// A person.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Person {
    pub id: String,
    pub name: String,
    pub initials: String,
}

/// A repository.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Repository {
    pub id: String,
    pub name: String,
    pub source: RepoSource,
    pub default_branch: String,
    pub path: Option<String>,
    pub remote_url: Option<String>,
}

/// A connected (or connectable) agent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Agent {
    pub id: String,
    pub name: String,
    pub vendor: String,
    pub category: AgentCategory,
    pub transport: String,
    pub adapter: Option<Adapter>,
    pub runs_on: RunsOn,
    pub node_id: Option<String>,
    pub command_policy: CommandPolicy,
    pub status: AgentStatus,
    pub requires_network: bool,
}

/// A mission: one goal, several agents.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Mission {
    pub id: String,
    pub number: i64,
    pub title: String,
    pub short_name: String,
    pub goal: String,
    pub mode: MissionMode,
    pub repo_id: String,
    pub base_branch: String,
    pub branch: String,
    pub status: MissionStatus,
    pub approval_policy: ApprovalPolicy,
    pub owner_id: String,
    pub node_id: Option<String>,
    pub stats: Option<DiffStats>,
    pub created_at: String,
    /// When the owner archived it (hidden from the active lists, its agents stopped); null when active.
    #[serde(default)]
    pub archived_at: Option<String>,
}

/// An agent assigned to a mission with a role.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MissionAgent {
    pub id: String,
    pub mission_id: String,
    pub agent_id: String,
    pub role: Role,
    pub enabled: bool,
    pub status: MissionAgentStatus,
    pub status_detail: Option<String>,
    pub steered_by: Option<String>,
    pub task: Option<String>,
    pub started_at: Option<String>,
    pub branch: Option<String>,
    pub latest: Vec<LogLine>,
    /// The custom agent type this agent was added as (its instructions join the role's brief).
    #[serde(default)]
    pub agent_type_id: Option<String>,
}

/// A custom agent type: a base role on a default agent (vendor or harness), with its own instructions
/// ("Security reviewer" = reviewer on Codex, told to look for injection bugs).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentType {
    pub id: String,
    pub name: String,
    pub role: Role,
    pub agent_id: String,
    pub instructions: String,
}

/// A runner node (the desktop app or `astrad`) that runs agents for the workspace. Each runner node puts its
/// own entry when it starts; clients pick one as the place an agent runs (`agent.nodeId`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunnerNode {
    pub id: String,
    /// Display name: the machine's host name unless the node was given one.
    pub name: String,
    /// The person the node runs as (whose session it signed in to the relay with).
    pub owner_id: String,
    /// The operating system: `macos`, `linux`, `windows`, ...
    pub platform: String,
    /// When the node last started.
    pub started_at: String,
}

/// A board task.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Task {
    pub id: String,
    pub mission_id: String,
    pub title: String,
    pub role: Role,
    pub mission_agent_id: Option<String>,
    pub column: TaskColumn,
    pub note: Option<String>,
    pub merged_by: Option<String>,
    pub approval_id: Option<String>,
    pub order: i64,
}

/// A thread message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Message {
    pub id: String,
    pub mission_id: String,
    pub author: Author,
    pub steering: Option<String>,
    pub to: Option<String>,
    pub body: String,
    pub plan: Vec<PlanStep>,
    pub log: Vec<LogLine>,
    pub approval_id: Option<String>,
    pub at: String,
}

/// An approval request raised by an agent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Approval {
    pub id: String,
    pub mission_id: String,
    pub mission_agent_id: String,
    pub prompt: String,
    pub command: Option<String>,
    pub state: ApprovalState,
    pub decided_by: Option<String>,
    pub decided_at: Option<String>,
}

/// A mission result (one per raced builder, one for a split mission). Wire kind: `result`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MissionResult {
    pub id: String,
    pub mission_id: String,
    pub label: String,
    pub mission_agent_id: String,
    pub branch: String,
    pub stats: DiffStats,
    pub checks: Checks,
    pub reviewer_pick: bool,
    pub diff: Vec<DiffFile>,
}

/// A merge record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Merge {
    pub id: String,
    pub mission_id: String,
    pub result_id: Option<String>,
    pub into: String,
    pub stats: DiffStats,
    pub checks: Checks,
    pub approved_by: String,
    pub note: Option<String>,
    pub kept_branches: Vec<String>,
    pub at: String,
}

/// Mission membership. `id` is `"<missionId>:<personId>"`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Membership {
    pub id: String,
    pub mission_id: String,
    pub person_id: String,
    pub access: Access,
}

impl Membership {
    /// The canonical membership id.
    pub fn id_for(mission_id: &str, person_id: &str) -> String {
        format!("{mission_id}:{person_id}")
    }
}

/// Ephemeral presence (relayed, never stored in the op log).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Presence {
    pub person_id: String,
    pub mission_id: Option<String>,
    pub activity: String,
    pub at: String,
}

string_enum!(
    /// Discriminant of [`Entity`] (the `kind` field / `entityKind` in ops).
    EntityKind { Person, Repository, Agent, Mission, MissionAgent, Task, Message, Approval, Result, Merge, Membership, AgentType, Node }
);

impl EntityKind {
    /// All kinds.
    pub const ALL: [EntityKind; 13] = [
        EntityKind::Person,
        EntityKind::Repository,
        EntityKind::Agent,
        EntityKind::Mission,
        EntityKind::MissionAgent,
        EntityKind::Task,
        EntityKind::Message,
        EntityKind::Approval,
        EntityKind::Result,
        EntityKind::Merge,
        EntityKind::Membership,
        EntityKind::AgentType,
        EntityKind::Node,
    ];

    /// The wire string (`"missionAgent"`).
    pub fn as_str(self) -> &'static str {
        match self {
            EntityKind::Person => "person",
            EntityKind::Repository => "repository",
            EntityKind::Agent => "agent",
            EntityKind::Mission => "mission",
            EntityKind::MissionAgent => "missionAgent",
            EntityKind::Task => "task",
            EntityKind::Message => "message",
            EntityKind::Approval => "approval",
            EntityKind::Result => "result",
            EntityKind::Merge => "merge",
            EntityKind::Membership => "membership",
            EntityKind::AgentType => "agentType",
            EntityKind::Node => "node",
        }
    }

    /// Parse the wire string.
    pub fn parse(s: &str) -> Option<EntityKind> {
        EntityKind::ALL.into_iter().find(|k| k.as_str() == s)
    }
}

impl std::fmt::Display for EntityKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Any entity, tagged by `kind`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Entity {
    Person(Person),
    Repository(Repository),
    Agent(Agent),
    Mission(Mission),
    MissionAgent(MissionAgent),
    Task(Task),
    Message(Message),
    Approval(Approval),
    Result(MissionResult),
    Merge(Merge),
    Membership(Membership),
    AgentType(AgentType),
    Node(RunnerNode),
}

impl Entity {
    /// The entity's kind.
    pub fn kind(&self) -> EntityKind {
        match self {
            Entity::Person(_) => EntityKind::Person,
            Entity::Repository(_) => EntityKind::Repository,
            Entity::Agent(_) => EntityKind::Agent,
            Entity::Mission(_) => EntityKind::Mission,
            Entity::MissionAgent(_) => EntityKind::MissionAgent,
            Entity::Task(_) => EntityKind::Task,
            Entity::Message(_) => EntityKind::Message,
            Entity::Approval(_) => EntityKind::Approval,
            Entity::Result(_) => EntityKind::Result,
            Entity::Merge(_) => EntityKind::Merge,
            Entity::Membership(_) => EntityKind::Membership,
            Entity::AgentType(_) => EntityKind::AgentType,
            Entity::Node(_) => EntityKind::Node,
        }
    }

    /// The entity's id.
    pub fn id(&self) -> &str {
        match self {
            Entity::Person(e) => &e.id,
            Entity::Repository(e) => &e.id,
            Entity::Agent(e) => &e.id,
            Entity::Mission(e) => &e.id,
            Entity::MissionAgent(e) => &e.id,
            Entity::Task(e) => &e.id,
            Entity::Message(e) => &e.id,
            Entity::Approval(e) => &e.id,
            Entity::Result(e) => &e.id,
            Entity::Merge(e) => &e.id,
            Entity::Membership(e) => &e.id,
            Entity::AgentType(e) => &e.id,
            Entity::Node(e) => &e.id,
        }
    }

    /// The entity as a JSON object (including `kind`).
    pub fn to_json(&self) -> serde_json::Map<String, serde_json::Value> {
        match serde_json::to_value(self) {
            Ok(serde_json::Value::Object(map)) => map,
            // Entities always serialize to objects; keep this total without panicking.
            _ => serde_json::Map::new(),
        }
    }

    /// Parse an entity from a JSON object of the given kind (a `kind` field, if present, is overwritten).
    pub fn from_json(
        kind: EntityKind,
        mut map: serde_json::Map<String, serde_json::Value>,
    ) -> Result<Entity, serde_json::Error> {
        map.insert(
            "kind".into(),
            serde_json::Value::String(kind.as_str().into()),
        );
        serde_json::from_value(serde_json::Value::Object(map))
    }
}

macro_rules! entity_from {
    ($($ty:ident => $variant:ident),+ $(,)?) => {
        $(impl From<$ty> for Entity {
            fn from(e: $ty) -> Self { Entity::$variant(e) }
        })+
    };
}

entity_from!(
    Person => Person,
    Repository => Repository,
    Agent => Agent,
    Mission => Mission,
    MissionAgent => MissionAgent,
    Task => Task,
    Message => Message,
    Approval => Approval,
    MissionResult => Result,
    Merge => Merge,
    Membership => Membership,
    AgentType => AgentType,
    RunnerNode => Node,
);
