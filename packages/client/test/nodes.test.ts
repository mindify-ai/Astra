import { describe, expect, it } from 'vitest'
import seed from '../../../fixtures/seed.json'
import starter from '../../../fixtures/starter.json'
import { connectAgent, createMission, testAgent } from '../src/actions'
import { MockClient } from '../src/mock'
import { DEFAULT_NODE, defaultRunner, runnerNodes } from '../src/nodes'
import { emptyState } from '../src/reducer'
import type { SeedFile } from '../src/store'
import type { AgentConnection, Command, Op } from '../src/types'

const client = (nodeId?: string) => new MockClient({ seed: seed as SeedFile, latency: 0, nodeId })

/** Every command op the client sent, with the node it was addressed to. */
function commands(c: MockClient): { node: string; command: Command }[] {
  const out: { node: string; command: Command }[] = []
  const submit = c.submit.bind(c)
  c.submit = async (bodies) => {
    const ops: Op[] = await submit(bodies)
    for (const op of ops)
      if (op.body.type === 'command') out.push({ node: op.body.node, command: op.body.command })
    return ops
  }
  return out
}

describe('runner nodes', () => {
  it('lists the workspace runner nodes by name', () => {
    const s = client().getState()
    expect(runnerNodes(s).map((n) => n.name)).toEqual(['build-01', "Mark's MacBook Pro"])
  })

  it('defaults to this machine, then a node of mine, then any node', () => {
    const s = client().getState()
    expect(defaultRunner(s, 'n-mark-desktop')).toBe('n-mark-desktop')
    expect(defaultRunner(s, 'n-browser')).toBe('n-build-01')
    const lin = { ...s, me: 'p-lin' }
    expect(defaultRunner(lin, null)).toBe('n-build-01')
    expect(defaultRunner(emptyState('ws', 'p-me'), null)).toBe(DEFAULT_NODE)
  })

  it('attaches an agent to a server node: the command goes there and the node takes it', async () => {
    const c = client('n-mark-desktop')
    const sent = commands(c)
    const outcome = await connectAgent(c, 'a-opencode', { nodeId: 'n-build-01', commandPolicy: 'auto' })
    expect(outcome).toEqual({ ok: true, error: null })
    expect(sent).toEqual([{ node: 'n-build-01', command: { type: 'connectAgent', agentId: 'a-opencode' } }])
    const a = c.getState().agents['a-opencode']
    expect(a).toMatchObject({
      nodeId: 'n-build-01',
      runsOn: 'server',
      status: 'connected',
      commandPolicy: 'auto',
    })

    await testAgent(c, 'a-opencode')
    expect(sent[1]?.node).toBe('n-build-01')
  })

  it('an agent on this machine runs locally', async () => {
    const c = client('n-mark-desktop')
    await connectAgent(c, 'a-opencode', { nodeId: 'n-mark-desktop' })
    expect(c.getState().agents['a-opencode']).toMatchObject({ nodeId: 'n-mark-desktop', runsOn: 'local' })
  })

  it('publishes a catalog agent the workspace has never seen (put, not patch)', async () => {
    const c = new MockClient({ seed: { ...(seed as SeedFile), entities: [] }, latency: 0 })
    const catalog = (starter as SeedFile).entities.find(
      (e): e is AgentConnection => e.kind === 'agent' && e.id === 'a-codex',
    )
    expect(c.getState().agents['a-codex']).toBeUndefined()
    expect(await connectAgent(c, 'a-codex', { nodeId: 'n-build-01' }, catalog)).toEqual({
      ok: true,
      error: null,
    })
    expect(c.getState().agents['a-codex']).toMatchObject({
      name: 'Codex',
      nodeId: 'n-build-01',
      status: 'connected',
    })
  })

  it('a mission runs on the node of its first enabled agent', async () => {
    const c = client('n-mark-desktop')
    await connectAgent(c, 'a-opencode', { nodeId: 'n-build-01' })
    const sent = commands(c)
    const id = await createMission(c, {
      goal: 'Add a health check.',
      mode: 'split',
      repoId: Object.keys(c.getState().repositories)[0] ?? 'r-1',
      baseBranch: 'main',
      agents: [
        { agentId: 'a-claude-code', role: 'planner', enabled: false },
        { agentId: 'a-opencode', role: 'builder', enabled: true },
      ],
    })
    expect(c.getState().missions[id]?.nodeId).toBe('n-build-01')
    expect(sent[0]).toEqual({ node: 'n-build-01', command: { type: 'startMission', missionId: id } })
  })
})
