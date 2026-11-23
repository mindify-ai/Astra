// Protocol types. Mirrors docs/PROTOCOL.md and crates/astra-protocol (serde camelCase, explicit nulls).
// `src/generated/protocol.ts` is exported from Rust; `test/protocol-compat.test.ts` keeps the two in step.

export type Id = string
export type Timestamp = string

export interface Person {
  kind: 'person'
  id: Id
  name: string
  initials: string
}

export type RepoSource = 'github' | 'gitlab' | 'local'
export interface Repository {
  kind: 'repository'
  id: Id
  name: string
  source: RepoSource
  defaultBranch: string
  path: string | null
  remoteUrl: string | null
}

export type Adapter =
  | { type: 'acp'; command: string; args: string[] }
  | { type: 'langgraph'; endpoint: string; graph: string }
  | { type: 'command'; command: string; args: string[] }
  | { type: 'builtin'; provider: 'auto' | 'cloud' | 'local'; model: string | null }

export type AgentCategory = 'codingAgent' | 'harness' | 'protocol' | 'local'
export type CommandPolicy = 'askMe' | 'askSteerers' | 'auto'
export type ConnectionStatus = 'connected' | 'settingUp' | 'notConnected' | 'offline'
export interface AgentConnection {
  kind: 'agent'
  id: Id
  name: string
  vendor: string
  category: AgentCategory
  transport: string
  adapter: Adapter | null
  runsOn: 'local' | 'server'
  nodeId: Id | null
  commandPolicy: CommandPolicy
  status: ConnectionStatus
  requiresNetwork: boolean
}

export interface DiffStats {
  files: number
  additions: number
  deletions: number
}

export type MissionMode = 'split' | 'race'
export type MissionStatus = 'draft' | 'working' | 'needsYou' | 'done' | 'merged'
export interface Mission {
  kind: 'mission'
  id: Id
  number: number
  title: string
  shortName: string
  goal: string
  mode: MissionMode
  repoId: Id
  baseBranch: string
  branch: string
  status: MissionStatus
  approvalPolicy: 'owner' | 'steerers'
  ownerId: Id
  nodeId: Id | null
  stats: DiffStats | null
  createdAt: Timestamp
}

export type Role = 'planner' | 'builder' | 'tester' | 'reviewer' | 'writer'
export type AgentRunStatus = 'idle' | 'queued' | 'working' | 'needsYou' | 'done' | 'offline'
export interface LogLine {
  verb: string
  target: string
}
export interface MissionAgent {
  kind: 'missionAgent'
  id: Id
  missionId: Id
  agentId: Id
  role: Role
  enabled: boolean
  status: AgentRunStatus
  statusDetail: string | null
  steeredBy: Id | null
  task: string | null
  startedAt: Timestamp | null
  branch: string | null
  latest: LogLine[]
}

export type TaskColumn = 'queued' | 'working' | 'needsYou' | 'done'
export interface Task {
  kind: 'task'
  id: Id
  missionId: Id
  title: string
  role: Role
  missionAgentId: Id | null
  column: TaskColumn
  note: string | null
  mergedBy: Id | null
  approvalId: Id | null
  order: number
}

export type Author = { type: 'person'; personId: Id } | { type: 'agent'; missionAgentId: Id }
export interface PlanStep {
  text: string
  state: 'done' | 'active' | 'pending'
}
export interface Message {
  kind: 'message'
  id: Id
  missionId: Id
  author: Author
  steering: Id | null
  to: Id | null
  body: string
  plan: PlanStep[]
  log: LogLine[]
  approvalId: Id | null
  at: Timestamp
}

export interface Approval {
  kind: 'approval'
  id: Id
  missionId: Id
  missionAgentId: Id
  prompt: string
  command: string | null
  state: 'pending' | 'approved' | 'denied'
  decidedBy: Id | null
  decidedAt: Timestamp | null
}

export type Checks = 'passed' | 'failed' | 'pending' | 'none'
export interface DiffFile {
  path: string
  lines: { kind: 'ctx' | 'add' | 'del'; text: string }[]
}
export interface RaceResult {
  kind: 'result'
  id: Id
  missionId: Id
  label: string
  missionAgentId: Id
  branch: string
  stats: DiffStats
  checks: Checks
  reviewerPick: boolean
  diff: DiffFile[]
}

export interface MergeRecord {
  kind: 'merge'
  id: Id
  missionId: Id
  resultId: Id | null
  into: string
  stats: DiffStats
  checks: Checks
  approvedBy: Id
  note: string | null
  keptBranches: string[]
  at: Timestamp
}

export type Access = 'owner' | 'steer' | 'watch'
export interface Membership {
  kind: 'membership'
  id: Id
  missionId: Id
  personId: Id
  access: Access
}

export type Entity =
  | Person
  | Repository
  | AgentConnection
  | Mission
  | MissionAgent
  | Task
  | Message
  | Approval
  | RaceResult
  | MergeRecord
  | Membership

export type EntityKind = Entity['kind']
export type EntityOf<K extends EntityKind> = Extract<Entity, { kind: K }>

export interface Presence {
  personId: Id
  missionId: Id | null
  activity: string
  at: Timestamp
}

export interface State {
  workspaceId: Id
  me: Id
  people: Record<Id, Person>
  repositories: Record<Id, Repository>
  agents: Record<Id, AgentConnection>
  missions: Record<Id, Mission>
  missionAgents: Record<Id, MissionAgent>
  tasks: Record<Id, Task>
  messages: Record<Id, Message>
  approvals: Record<Id, Approval>
  results: Record<Id, RaceResult>
  merges: Record<Id, MergeRecord>
  memberships: Record<Id, Membership>
  presence: Presence[]
}

export type Command =
  | { type: 'startMission'; missionId: Id }
  | { type: 'approve'; approvalId: Id }
  | { type: 'deny'; approvalId: Id }
  | { type: 'sendMessage'; missionId: Id; to: Id | null; body: string }
  | { type: 'takeSteering'; missionAgentId: Id }
  | { type: 'releaseSteering'; missionAgentId: Id }
  | { type: 'pause'; missionAgentId: Id }
  | { type: 'resume'; missionAgentId: Id }
  | { type: 'merge'; resultId: Id }
  | { type: 'connectAgent'; agentId: Id }
  | { type: 'testAgent'; agentId: Id }

export type OpBody =
  | { type: 'put'; entity: Entity }
  | { type: 'patch'; entityKind: EntityKind; id: Id; fields: Record<string, unknown> }
  | { type: 'delete'; entityKind: EntityKind; id: Id }
  | { type: 'command'; node: Id; command: Command }
  | { type: 'commandResult'; commandOp: Id; ok: boolean; error: string | null }

export interface Op {
  id: Id
  hlc: string
  actor: Id
  body: OpBody
}

export interface Snapshot {
  state: State
  cursor: number
}

export interface Connectivity {
  online: boolean
  relay: 'connected' | 'disconnected' | 'disabled'
}

/** Which State map holds each entity kind. */
export const collectionOf = {
  person: 'people',
  repository: 'repositories',
  agent: 'agents',
  mission: 'missions',
  missionAgent: 'missionAgents',
  task: 'tasks',
  message: 'messages',
  approval: 'approvals',
  result: 'results',
  merge: 'merges',
  membership: 'memberships',
} as const satisfies Record<EntityKind, keyof State>
