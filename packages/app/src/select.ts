import type {
  AgentConnection,
  AgentRunStatus,
  AgentType,
  Id,
  Mission,
  MissionAgent,
  MissionStatus,
  Person,
  Role,
  State,
  TaskColumn,
} from '@astra/client'
import { mayDecide, maySteer, starterSeed } from '@astra/client'
import type { Tone } from '@astra/ui'

export const roleName: Record<Role, string> = {
  planner: 'Planner',
  builder: 'Builder',
  tester: 'Tester',
  reviewer: 'Reviewer',
  writer: 'Writer',
}
export const roleVerb: Record<Role, string> = {
  planner: 'Plans',
  builder: 'Builds',
  tester: 'Tests',
  reviewer: 'Reviews',
  writer: 'Writes',
}
export const roleLetter = (r: Role) => roleName[r].charAt(0)

/** Every role, in pipeline order. */
export const roles: Role[] = ['planner', 'builder', 'tester', 'reviewer', 'writer']

/** Custom agent types, by name. */
export function agentTypes(s: State): AgentType[] {
  return Object.values(s.agentTypes).sort((a, b) => a.name.localeCompare(b.name))
}

/** Names for a mission's agents, numbered when one name repeats ("Builder", "Builder 2"): the same agent
 * (vendor or harness) may fill several roles, or the same role twice. */
export function agentNames(s: State, agents: MissionAgent[]): Record<Id, string> {
  const seen: Record<string, number> = {}
  const out: Record<Id, string> = {}
  for (const ma of agents) {
    const base = missionAgentName(s, ma)
    seen[base] = (seen[base] ?? 0) + 1
    out[ma.id] = seen[base] > 1 ? `${base} ${seen[base]}` : base
  }
  return out
}

/** What a mission agent is called: its custom type's name, or its role. */
export function missionAgentName(s: State, ma: Pick<MissionAgent, 'role' | 'agentTypeId'>): string {
  return (ma.agentTypeId && s.agentTypes[ma.agentTypeId]?.name) || roleName[ma.role]
}

/** "014" */
export const missionNo = (m: Pick<Mission, 'number'>) => String(m.number).padStart(3, '0')

export const agentStatus: Record<AgentRunStatus, { tone: Tone; label: string }> = {
  idle: { tone: 'idle', label: 'Idle' },
  queued: { tone: 'idle', label: 'Queued' },
  working: { tone: 'working', label: 'Working' },
  needsYou: { tone: 'needsYou', label: 'Needs you' },
  done: { tone: 'idle', label: 'Done' },
  offline: { tone: 'idle', label: 'Offline' },
}

export const missionStatus: Record<MissionStatus, { tone: Tone; label: string }> = {
  draft: { tone: 'idle', label: 'Draft' },
  working: { tone: 'working', label: 'Working' },
  needsYou: { tone: 'needsYou', label: 'Needs you' },
  done: { tone: 'idle', label: 'Done' },
  merged: { tone: 'success', label: 'Merged' },
}

export const columnStatus: Record<TaskColumn, { tone: Tone; label: string }> = {
  queued: { tone: 'idle', label: 'Queued' },
  working: { tone: 'working', label: 'Working' },
  needsYou: { tone: 'needsYou', label: 'Needs you' },
  done: { tone: 'idle', label: 'Done' },
}

/** "09:16" in the viewer's time zone. */
export function hhmm(iso: string | null): string {
  if (!iso) return ''
  const d = new Date(iso)
  return `${String(d.getHours()).padStart(2, '0')}:${String(d.getMinutes()).padStart(2, '0')}`
}

const byNumberDesc = (a: Mission, b: Mission) => b.number - a.number

/** Missions in sidebar order: working first, then needs-you, then the rest, newest first within each. */
export function orderedMissions(s: State, repoId?: Id): Mission[] {
  const rank: Record<MissionStatus, number> = { working: 0, needsYou: 1, draft: 2, done: 3, merged: 3 }
  return Object.values(s.missions)
    .filter((m) => !m.archivedAt && (!repoId || m.repoId === repoId))
    .sort((a, b) => rank[a.status] - rank[b.status] || byNumberDesc(a, b))
}

/** Archived missions, newest first. */
export function archivedMissions(s: State, repoId?: Id): Mission[] {
  return Object.values(s.missions)
    .filter((m) => m.archivedAt && (!repoId || m.repoId === repoId))
    .sort(byNumberDesc)
}

/** The agents taking part in a mission (removed ones stay in `s.missionAgents` for the thread's history). */
export function missionAgents(s: State, missionId: Id): MissionAgent[] {
  const order: Record<Role, number> = { planner: 0, builder: 1, tester: 2, reviewer: 3, writer: 4 }
  return Object.values(s.missionAgents)
    .filter((m) => m.missionId === missionId && m.enabled)
    .sort((a, b) => order[a.role] - order[b.role] || a.id.localeCompare(b.id))
}

export function members(s: State, missionId: Id): { person: Person; access: 'owner' | 'steer' | 'watch' }[] {
  const order = { owner: 0, steer: 1, watch: 2 }
  return Object.values(s.memberships)
    .filter((m) => m.missionId === missionId)
    .flatMap((m) => {
      const person = s.people[m.personId]
      return person ? [{ person, access: m.access }] : []
    })
    .sort((a, b) => order[a.access] - order[b.access])
}

export const agentOf = (s: State, ma: MissionAgent | undefined): AgentConnection | undefined =>
  ma ? s.agents[ma.agentId] : undefined

/** The mission the THREAD tab should open. */
export function currentMissionId(s: State, preferred: Id | null): Id | null {
  if (preferred && s.missions[preferred]) return preferred
  return orderedMissions(s)[0]?.id ?? null
}

const catalog = starterSeed.entities.filter((e): e is AgentConnection => e.kind === 'agent')

/**
 * Agents you can connect: the workspace's own, plus any from the built-in catalog it has not seen yet (a
 * workspace on a sync server starts empty). Connecting a catalog agent puts it into the workspace.
 */
export function agentCatalog(agents: State['agents']): AgentConnection[] {
  const known = new Set(catalog.map((a) => a.id))
  return [...catalog.map((a) => agents[a.id] ?? a), ...Object.values(agents).filter((a) => !known.has(a.id))]
}

const platformName: Record<string, string> = { macos: 'MACOS', linux: 'LINUX', windows: 'WINDOWS' }
/** `macos` → `MACOS`, for spec labels. */
export function platformLabel(platform: string): string {
  return platformName[platform] ?? platform.toUpperCase()
}

export function connectedAgents(s: State): AgentConnection[] {
  return Object.values(s.agents).filter((a) => a.status === 'connected')
}

export const personName = (s: State, id: Id | null) => (id ? (s.people[id]?.name ?? id) : '')

/** The runner node's rules (`@astra/client` policy): who may decide approvals/merges, and who may steer. */
export const canApprove = (s: State, personId: Id, missionId: Id, missionAgentId?: Id | null) =>
  mayDecide(s, missionId, personId, missionAgentId)
export const canSteer = (s: State, personId: Id, missionId: Id) => maySteer(s, missionId, personId)

/** Route for a mission row: picking results → review, merged → summary, otherwise the thread. */
export function missionRoute(
  s: State,
  m: Mission,
): '/missions/$missionId' | '/missions/$missionId/review' | '/missions/$missionId/merged' {
  if (m.status === 'merged' && s.merges[m.id]) return '/missions/$missionId/merged'
  if (m.mode === 'race' && Object.values(s.results).some((r) => r.missionId === m.id))
    return '/missions/$missionId/review'
  return '/missions/$missionId'
}
