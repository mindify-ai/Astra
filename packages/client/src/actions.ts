import { ulid } from './hlc'
import type { AstraClient } from './store'
import type { Access, Command, Entity, Id, MissionAgent, MissionMode, OpBody, Role, State } from './types'

/** Runner node that executes commands for a mission. Falls back to the local desktop node id. */
export function nodeForMission(state: State, missionId: Id): Id {
  return state.missions[missionId]?.nodeId ?? DEFAULT_NODE
}
export const DEFAULT_NODE = 'n-local'

const command = (node: Id, cmd: Command): OpBody => ({ type: 'command', node, command: cmd })

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
  agents: { agentId: Id; role: Role; enabled: boolean }[]
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
export function sendMessage(client: AstraClient, missionId: Id, to: Id | null, body: string) {
  const state = client.getState()
  const text = body.trim()
  if (!text) return Promise.resolve([])
  return client.submit([
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

export function connectAgent(
  client: AstraClient,
  agentId: Id,
  patch: Record<string, unknown>,
  node = DEFAULT_NODE,
) {
  const state = client.getState()
  const nodeId =
    state.agents[agentId]?.nodeId ?? Object.values(state.agents).find((a) => a.nodeId)?.nodeId ?? node
  return client.submit([
    { type: 'patch', entityKind: 'agent', id: agentId, fields: { ...patch, nodeId, status: 'settingUp' } },
    command(nodeId, { type: 'connectAgent', agentId }),
  ])
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
