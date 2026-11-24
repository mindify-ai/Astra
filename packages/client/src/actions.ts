import { ulid } from './hlc'
import type { AstraClient, CommandOutcome } from './store'
import type {
  Access,
  AgentType,
  Command,
  Entity,
  Id,
  MissionAgent,
  MissionMode,
  Op,
  OpBody,
  Role,
  State,
} from './types'

/** Runner node that executes commands for a mission. Falls back to the local desktop node id. */
export function nodeForMission(state: State, missionId: Id): Id {
  return state.missions[missionId]?.nodeId ?? DEFAULT_NODE
}
export const DEFAULT_NODE = 'n-local'

const command = (node: Id, cmd: Command): OpBody => ({ type: 'command', node, command: cmd })

/** The outcome of the command op among `ops` (null if the node did not answer in time). */
async function outcomeOf(client: AstraClient, ops: Op[]): Promise<CommandOutcome | null> {
  const cmd = ops.find((o) => o.body.type === 'command')
  return cmd ? client.commandResult(cmd.id) : null
}

export function slugify(text: string, max = 32): string {
  const slug = text
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-+|-+$/g, '')
  return slug.slice(0, max).replace(/-+$/, '') || 'mission'
}

export interface MissionDraft {
  goal: string
  title?: string
  mode: MissionMode
  repoId: Id
  baseBranch: string
  branch?: string
  agents: { agentId: Id; role: Role; enabled: boolean; agentTypeId?: Id | null }[]
  people?: { personId: Id; access: Access }[]
  nodeId?: Id
}

/** Short title from a goal: first sentence, without a trailing period. */
export function titleFromGoal(goal: string): string {
  const first = goal.trim().split(/(?<=[.!?])\s/)[0] ?? goal
  return first.replace(/[.!?]+$/, '').slice(0, 80) || 'New mission'
}

/** Puts the mission, its agents and memberships, then asks the node to start it. Returns the mission id. */
export async function createMission(client: AstraClient, draft: MissionDraft): Promise<Id> {
  const state = client.getState()
  const number = Math.max(0, ...Object.values(state.missions).map((m) => m.number)) + 1
  const id = `m-${String(number).padStart(3, '0')}-${ulid().slice(-6).toLowerCase()}`
  const title = draft.title ?? titleFromGoal(draft.goal)
  const branch = draft.branch?.trim() || `astra/${slugify(title)}`
  const nodeId = draft.nodeId ?? Object.values(state.agents).find((a) => a.nodeId)?.nodeId ?? DEFAULT_NODE
  const words = title.split(/\s+/)
  const entities: Entity[] = [
    {
      kind: 'mission',
      id,
      number,
      title,
      shortName: words.slice(-2).join(' ').toUpperCase(),
      goal: draft.goal,
      mode: draft.mode,
      repoId: draft.repoId,
      baseBranch: draft.baseBranch,
      branch,
      status: 'draft',
      approvalPolicy: 'owner',
      ownerId: state.me,
      nodeId,
      stats: null,
      createdAt: new Date().toISOString(),
      archivedAt: null,
    },
    ...draft.agents.map(
      (a, i): MissionAgent => ({
        kind: 'missionAgent',
        id: `${id}-ma-${i}`,
        missionId: id,
        agentId: a.agentId,
        role: a.role,
        enabled: a.enabled,
        status: 'queued',
        statusDetail: null,
        steeredBy: null,
        task: null,
        startedAt: null,
        branch: null,
        latest: [],
        agentTypeId: a.agentTypeId ?? null,
      }),
    ),
    { kind: 'membership', id: `${id}:${state.me}`, missionId: id, personId: state.me, access: 'owner' },
    ...(draft.people ?? [])
      .filter((p) => p.personId !== state.me)
      .map(
        (p): Entity => ({
          kind: 'membership',
          id: `${id}:${p.personId}`,
          missionId: id,
          personId: p.personId,
          access: p.access,
        }),
      ),
    {
      kind: 'message',
      id: `${id}-goal`,
      missionId: id,
      author: { type: 'person', personId: state.me },
      steering: null,
      to: null,
      body: draft.goal,
      plan: [],
      log: [],
      approvalId: null,
      at: new Date().toISOString(),
    },
  ]
  await client.submit([
    ...entities.map((entity): OpBody => ({ type: 'put', entity })),
    command(nodeId, { type: 'startMission', missionId: id }),
  ])
  return id
}

export function approve(client: AstraClient, approvalId: Id, allow: boolean) {
  const state = client.getState()
  const a = state.approvals[approvalId]
  if (!a) return Promise.resolve([])
  return client.submit([
    command(nodeForMission(state, a.missionId), { type: allow ? 'approve' : 'deny', approvalId }),
  ])
}

/** Writes the human message, then asks the node to deliver it (steering the target agent if `to` is set). */
/** Posts a message to the thread and delivers it; resolves with the node's answer (it may restart an agent). */
export async function sendMessage(
  client: AstraClient,
  missionId: Id,
  to: Id | null,
  body: string,
): Promise<CommandOutcome | null> {
  const state = client.getState()
  const text = body.trim()
  if (!text) return null
  const ops = await client.submit([
    {
      type: 'put',
      entity: {
        kind: 'message',
        id: `msg-${ulid().toLowerCase()}`,
        missionId,
        author: { type: 'person', personId: state.me },
        steering: to,
        to,
        body: text,
        plan: [],
        log: [],
        approvalId: null,
        at: new Date().toISOString(),
      },
    },
    command(nodeForMission(state, missionId), { type: 'sendMessage', missionId, to, body: text }),
  ])
  return outcomeOf(client, ops)
}

export function merge(client: AstraClient, resultId: Id) {
  const state = client.getState()
  const r = state.results[resultId]
  if (!r) return Promise.resolve([])
  return client.submit([command(nodeForMission(state, r.missionId), { type: 'merge', resultId })])
}

export function agentCommand(
  client: AstraClient,
  missionAgentId: Id,
  type: 'pause' | 'resume' | 'takeSteering' | 'releaseSteering',
) {
  const state = client.getState()
  const ma = state.missionAgents[missionAgentId]
  if (!ma) return Promise.resolve([])
  return client.submit([command(nodeForMission(state, ma.missionId), { type, missionAgentId })])
}

/** Runs a mission agent's role on another connected agent (vendor or harness); resolves with the node's answer. */
export async function switchAgent(
  client: AstraClient,
  missionAgentId: Id,
  agentId: Id,
): Promise<CommandOutcome | null> {
  const state = client.getState()
  const ma = state.missionAgents[missionAgentId]
  if (!ma) return null
  const ops = await client.submit([
    command(nodeForMission(state, ma.missionId), { type: 'switchAgent', missionAgentId, agentId }),
  ])
  return outcomeOf(client, ops)
}

/** Puts a new agent on a mission in `role`, then asks the node to start it if the mission is under way. */
export async function addAgent(
  client: AstraClient,
  missionId: Id,
  role: Role,
  agentId: Id,
  agentTypeId: Id | null = null,
): Promise<{ missionAgentId: Id; outcome: CommandOutcome | null }> {
  const state = client.getState()
  const missionAgentId = `${missionId}-ma-${ulid().slice(-6).toLowerCase()}`
  const entity: MissionAgent = {
    kind: 'missionAgent',
    id: missionAgentId,
    missionId,
    agentId,
    role,
    enabled: true,
    status: 'queued',
    statusDetail: null,
    steeredBy: null,
    task: null,
    startedAt: null,
    branch: null,
    latest: [],
    agentTypeId,
  }
  const ops = await client.submit([
    { type: 'put', entity },
    command(nodeForMission(state, missionId), { type: 'addAgent', missionAgentId }),
  ])
  return { missionAgentId, outcome: await outcomeOf(client, ops) }
}

/** Saves a custom agent type (a base role on a default agent, with its own instructions). Returns its id. */
export async function saveAgentType(
  client: AstraClient,
  draft: { id?: Id; name: string; role: Role; agentId: Id; instructions: string },
): Promise<Id> {
  const id = draft.id ?? `at-${slugify(draft.name, 24) || 'type'}-${ulid().slice(-6).toLowerCase()}`
  const entity: AgentType = {
    kind: 'agentType',
    id,
    name: draft.name.trim(),
    role: draft.role,
    agentId: draft.agentId,
    instructions: draft.instructions.trim(),
  }
  await client.submit([{ type: 'put', entity }])
  return id
}

/** Deletes a custom agent type. Agents already added as this type keep their role and agent. */
export function deleteAgentType(client: AstraClient, agentTypeId: Id) {
  return client.submit([{ type: 'delete', entityKind: 'agentType', id: agentTypeId }])
}

/** Archives a mission (its agents stop and it leaves the active lists) or brings it back. Owner only. */
export async function archiveMission(
  client: AstraClient,
  missionId: Id,
  archived = true,
): Promise<CommandOutcome | null> {
  const state = client.getState()
  const ops = await client.submit([
    command(nodeForMission(state, missionId), { type: 'archiveMission', missionId, archived }),
  ])
  return outcomeOf(client, ops)
}

/** Deletes a mission and everything in it (its branches stay in the repository). Owner only. */
export async function deleteMission(client: AstraClient, missionId: Id): Promise<CommandOutcome | null> {
  const state = client.getState()
  const ops = await client.submit([
    command(nodeForMission(state, missionId), { type: 'deleteMission', missionId }),
  ])
  return outcomeOf(client, ops)
}

/** Takes an agent off its mission; resolves with the node's answer. */
export async function removeAgent(client: AstraClient, missionAgentId: Id): Promise<CommandOutcome | null> {
  const state = client.getState()
  const ma = state.missionAgents[missionAgentId]
  if (!ma) return null
  const ops = await client.submit([
    command(nodeForMission(state, ma.missionId), { type: 'removeAgent', missionAgentId }),
  ])
  return outcomeOf(client, ops)
}

/** Saves an agent's setup and asks its node to connect it; resolves with the node's answer. */
export async function connectAgent(
  client: AstraClient,
  agentId: Id,
  patch: Record<string, unknown>,
  node = DEFAULT_NODE,
): Promise<CommandOutcome | null> {
  const state = client.getState()
  const nodeId =
    state.agents[agentId]?.nodeId ?? Object.values(state.agents).find((a) => a.nodeId)?.nodeId ?? node
  const ops = await client.submit([
    { type: 'patch', entityKind: 'agent', id: agentId, fields: { ...patch, nodeId, status: 'settingUp' } },
    command(nodeId, { type: 'connectAgent', agentId }),
  ])
  return outcomeOf(client, ops)
}

/** Asks the agent's node whether it can reach the agent as set up now; resolves with the node's answer. */
export async function testAgent(client: AstraClient, agentId: Id): Promise<CommandOutcome | null> {
  const state = client.getState()
  const nodeId =
    state.agents[agentId]?.nodeId ?? Object.values(state.agents).find((a) => a.nodeId)?.nodeId ?? DEFAULT_NODE
  return outcomeOf(client, await client.submit([command(nodeId, { type: 'testAgent', agentId })]))
}

export function setMembership(client: AstraClient, missionId: Id, personId: Id, access: Access | null) {
  const id = `${missionId}:${personId}`
  if (access === null) return client.submit([{ type: 'delete', entityKind: 'membership', id }])
  return client.submit([{ type: 'put', entity: { kind: 'membership', id, missionId, personId, access } }])
}

export function setApprovalPolicy(client: AstraClient, missionId: Id, policy: 'owner' | 'steerers') {
  return client.submit([
    { type: 'patch', entityKind: 'mission', id: missionId, fields: { approvalPolicy: policy } },
  ])
}

/** Invites someone by email: adds them as a person (if new) and a member of the mission. */
export function invite(client: AstraClient, missionId: Id, email: string, access: Access) {
  const state = client.getState()
  const local = email.trim().split('@')[0] ?? ''
  if (!local) return Promise.resolve([])
  const name = local.charAt(0).toUpperCase() + local.slice(1)
  const existing = Object.values(state.people).find((p) => p.name.toLowerCase() === name.toLowerCase())
  const personId = existing?.id ?? `p-${slugify(local)}`
  const initials = (local.replace(/[^a-z]/gi, '').slice(0, 2) || 'XX').toUpperCase()
  const bodies: OpBody[] = []
  if (!existing) bodies.push({ type: 'put', entity: { kind: 'person', id: personId, name, initials } })
  bodies.push({
    type: 'put',
    entity: { kind: 'membership', id: `${missionId}:${personId}`, missionId, personId, access },
  })
  return client.submit(bodies)
}
