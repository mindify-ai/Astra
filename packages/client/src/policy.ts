import type { Id, State } from './types'

// The runner node's authorization rules (crates/astra-runtime/src/policy.rs), shared by the mock node and
// the UI so screens only offer what a real node will accept.

/** May `actor` decide this mission's approvals and merges? The owner always; members with steer access when
 * the mission's approval policy is `steerers` or the agent's command policy is `askSteerers`. */
export function mayDecide(s: State, missionId: Id, actor: Id, missionAgentId?: Id | null): boolean {
  const m = s.missions[missionId]
  if (!m) return false
  if (m.ownerId === actor) return true
  const ma = missionAgentId ? s.missionAgents[missionAgentId] : undefined
  const agentPolicy = ma ? s.agents[ma.agentId]?.commandPolicy : undefined
  const steerersAllowed = m.approvalPolicy === 'steerers' || agentPolicy === 'askSteerers'
  const access = s.memberships[`${missionId}:${actor}`]?.access
  return steerersAllowed && (access === 'steer' || access === 'owner')
}

/** May `actor` steer agents or message them? Owner or steer access. */
export function maySteer(s: State, missionId: Id, actor: Id): boolean {
  if (s.missions[missionId]?.ownerId === actor) return true
  const access = s.memberships[`${missionId}:${actor}`]?.access
  return access === 'steer' || access === 'owner'
}
