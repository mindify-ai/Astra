import type { AgentConnection, Id, RunnerNode, State } from './types'

/** Runner node that executes commands when nothing else names one: the desktop app's node in a local workspace. */
export const DEFAULT_NODE = 'n-local'

/** Runner nodes of the workspace, by name. */
export function runnerNodes(state: State): RunnerNode[] {
  return Object.values(state.nodes).sort((a, b) => a.name.localeCompare(b.name) || a.id.localeCompare(b.id))
}

/**
 * Where an agent should run when nothing has chosen yet. In order: this client's own node (the desktop app);
 * a runner node of the signed-in person; any runner node; the node another agent already runs on.
 */
export function defaultRunner(state: State, localNodeId: Id | null): Id {
  if (localNodeId && state.nodes[localNodeId]) return localNodeId
  const nodes = runnerNodes(state)
  return (
    nodes.find((n) => n.ownerId === state.me)?.id ??
    nodes[0]?.id ??
    Object.values(state.agents).find((a) => a.nodeId)?.nodeId ??
    DEFAULT_NODE
  )
}

/** The runner node an agent runs on, or the default one if it has none yet. */
export function nodeForAgent(state: State, agent: AgentConnection | undefined, localNodeId: Id | null): Id {
  return agent?.nodeId ?? defaultRunner(state, localNodeId)
}
