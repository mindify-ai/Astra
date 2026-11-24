import type { Id, Role, State } from '@astra/client'
import { Select } from '@astra/ui'
import { agentTypes, connectedAgents, roleName, roles } from '../select'

/** Who fills a slot on a mission: a role (or a custom agent type built on one) run by an agent. */
export interface AgentChoice {
  role: Role
  agentId: Id
  agentTypeId: Id | null
}

const typeValue = (c: Pick<AgentChoice, 'role' | 'agentTypeId'>) =>
  c.agentTypeId ? `type:${c.agentTypeId}` : `role:${c.role}`

/** Picking a custom type sets its role and, when that agent is connected, its default agent. */
export function chooseType(s: State, current: AgentChoice, value: string): AgentChoice {
  if (value.startsWith('type:')) {
    const t = s.agentTypes[value.slice(5)]
    if (!t) return current
    const agentId = s.agents[t.agentId]?.status === 'connected' ? t.agentId : current.agentId
    return { role: t.role, agentId, agentTypeId: t.id }
  }
  return { ...current, role: value.slice(5) as Role, agentTypeId: null }
}

/** A choice to start from: a builder on the first connected agent. */
export function firstChoice(s: State): AgentChoice | null {
  const agent = connectedAgents(s)[0]
  return agent ? { role: 'builder', agentId: agent.id, agentTypeId: null } : null
}

/** Role or custom agent type. */
export function TypeSelect({
  s,
  value,
  onChange,
  label,
  id,
}: {
  s: State
  value: AgentChoice
  onChange: (next: AgentChoice) => void
  label?: string
  id?: string
}) {
  const types = agentTypes(s)
  return (
    <Select
      id={id}
      size="sm"
      aria-label={label}
      value={typeValue(value)}
      onChange={(e) => onChange(chooseType(s, value, e.target.value))}
    >
      <optgroup label="Roles">
        {roles.map((r) => (
          <option key={r} value={`role:${r}`}>
            {roleName[r]}
          </option>
        ))}
      </optgroup>
      {types.length > 0 ? (
        <optgroup label="Your agent types">
          {types.map((t) => (
            <option key={t.id} value={`type:${t.id}`}>
              {t.name}
            </option>
          ))}
        </optgroup>
      ) : null}
    </Select>
  )
}

/** The agent (vendor CLI, harness or protocol) that runs the role. Only connected agents can be picked. */
export function HarnessSelect({
  s,
  value,
  onChange,
  label,
  id,
}: {
  s: State
  value: Id
  onChange: (agentId: Id) => void
  label?: string
  id?: string
}) {
  const agents = connectedAgents(s)
  const current = s.agents[value]
  return (
    <Select
      id={id}
      size="sm"
      mono
      aria-label={label}
      value={value}
      onChange={(e) => onChange(e.target.value)}
    >
      {current && current.status !== 'connected' ? (
        <option value={current.id} disabled>
          {current.name} (not connected)
        </option>
      ) : null}
      {agents.map((a) => (
        <option key={a.id} value={a.id}>
          {a.name}
        </option>
      ))}
    </Select>
  )
}
