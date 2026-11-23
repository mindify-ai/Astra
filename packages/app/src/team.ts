import type { AgentConnection, MissionMode, Role, State } from '@astra/client'

const preferred: Record<string, Role> = {
  'Claude Code': 'planner',
  Codex: 'builder',
  LangGraph: 'tester',
  'Gemini CLI': 'reviewer',
}
const fallback: Role[] = ['planner', 'builder', 'tester', 'reviewer']

/** Connected agents with a sensible default role each. In a race every agent builds, except a reviewer when there are three or more. */
export function defaultTeam(state: State, mode: MissionMode): { agent: AgentConnection; role: Role }[] {
  const connected = Object.values(state.agents).filter((a) => a.status === 'connected')
  if (mode === 'race') {
    const reviewer =
      connected.length >= 3 ? connected.find((a) => preferred[a.name] === 'reviewer') : undefined
    return connected.map((agent) => ({ agent, role: agent === reviewer ? 'reviewer' : 'builder' }))
  }
  const used = new Set<Role>()
  return connected.map((agent, i) => {
    let role = preferred[agent.name] ?? fallback[i % fallback.length]!
    if (used.has(role)) role = fallback.find((r) => !used.has(r)) ?? 'builder'
    used.add(role)
    return { agent, role }
  })
}
