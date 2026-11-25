import { describe, expect, it } from 'vitest'
import seed from '../../../fixtures/seed.json'
import { approve, connectAgent, createMission, merge, sendMessage, testAgent } from '../src/actions'
import { MockClient } from '../src/mock'
import type { SeedFile } from '../src/store'

const client = () => new MockClient({ seed: seed as SeedFile, latency: 0 })

describe('mock node', () => {
  it('connect and test resolve with the node answer', async () => {
    const c = client()
    expect(await connectAgent(c, 'a-opencode', { name: 'OpenCode' })).toEqual({ ok: true, error: null })
    expect(c.getState().agents['a-opencode']?.status).toBe('connected')
    expect(await testAgent(c, 'a-opencode')).toEqual({ ok: true, error: null })
  })

  it('a command with no answer resolves to null after the timeout', async () => {
    expect(await client().commandResult('op-never-sent', 5)).toBeNull()
  })

  it('approves a pending command and moves the task', async () => {
    const c = client()
    await approve(c, 'ap-0042', true)
    const s = c.getState()
    expect(s.approvals['ap-0042']).toMatchObject({ state: 'approved', decidedBy: 'p-mark' })
    expect(s.missionAgents['ma-014-tester']?.status).toBe('working')
    expect(s.tasks['t-014-migrate']?.column).toBe('working')
  })

  it('denying closes the linked task with a note', async () => {
    const c = client()
    await approve(c, 'ap-0042', false)
    const s = c.getState()
    expect(s.approvals['ap-0042']?.state).toBe('denied')
    expect(s.tasks['t-014-migrate']).toMatchObject({ column: 'done', note: 'Denied by p-mark' })
  })

  it('rejects steering a builder held by someone else', async () => {
    const c = client()
    const before = Object.keys(c.getState().messages).length
    const outcome = await sendMessage(c, 'm-014', 'ma-014-builder', 'Use the Redis store')
    // The human message is written; the node refuses to hand over steering held by Lin.
    expect(Object.keys(c.getState().messages).length).toBe(before + 1)
    expect(c.getState().missionAgents['ma-014-builder']?.steeredBy).toBe('p-lin')
    expect(outcome?.ok).toBe(false)
    expect(outcome?.error).toMatch(/steering/i)
  })

  it('answers a side question in its side thread without taking steering', async () => {
    const c = client()
    // Lin holds the builder; a side question to it is still answered, and steering doesn't move.
    const outcome = await sendMessage(c, 'm-014', 'ma-014-builder', 'What are you on?', 'side-1')
    expect(outcome).toEqual({ ok: true, error: null })
    const s = c.getState()
    expect(s.missionAgents['ma-014-builder']?.steeredBy).toBe('p-lin')
    const side = Object.values(s.messages).filter((m) => m.side === 'side-1')
    expect(side.map((m) => m.author)).toEqual([
      { type: 'person', personId: 'p-mark' },
      { type: 'agent', missionAgentId: 'ma-014-builder' },
    ])
    expect(side[0]).toMatchObject({ steering: null, to: 'ma-014-builder' })
  })

  it('lets steerers approve only when the agent or mission allows it', async () => {
    const c = client()
    const lin = (bodyType: 'approve') => [
      {
        id: `x-${bodyType}`,
        hlc: '9999999999999999:000000:n',
        actor: 'p-lin',
        body: {
          type: 'command' as const,
          node: 'n-mark-desktop',
          command: { type: bodyType, approvalId: 'ap-0042' },
        },
      },
    ]
    // Lin can steer, but mission 014 lets only its owner approve.
    ;(c as unknown as { receive: (ops: unknown[]) => void; send: (ops: unknown[]) => Promise<void> }).send(
      lin('approve'),
    )
    expect(c.getState().approvals['ap-0042']?.state).toBe('pending')
  })

  it('merges a race result and keeps the other branch', async () => {
    const c = client()
    await merge(c, 'res-015-a')
    const s = c.getState()
    expect(s.missions['m-015']?.status).toBe('merged')
    expect(s.merges['m-015']).toMatchObject({
      resultId: 'res-015-a',
      keptBranches: ['astra/pricing-cache-b'],
      approvedBy: 'p-mark',
    })
    expect(s.tasks['t-015-pick']).toMatchObject({ column: 'done', mergedBy: 'p-mark' })
  })

  it('creates and starts a race mission end to end', async () => {
    const c = client()
    const id = await createMission(c, {
      goal: 'Speed up the search endpoint.',
      mode: 'race',
      repoId: 'r-gateway',
      baseBranch: 'main',
      agents: [
        { agentId: 'a-codex', role: 'builder', enabled: true },
        { agentId: 'a-claude-code', role: 'builder', enabled: true },
        { agentId: 'a-gemini-cli', role: 'reviewer', enabled: true },
      ],
    })
    const s = c.getState()
    expect(s.missions[id]).toMatchObject({
      number: 16,
      title: 'Speed up the search endpoint',
      branch: 'astra/speed-up-the-search-endpoint',
      status: 'needsYou',
    })
    const results = Object.values(s.results).filter((r) => r.missionId === id)
    expect(results.map((r) => r.label)).toEqual(['A', 'B'])
    await merge(c, results[0]!.id)
    expect(c.getState().missions[id]?.status).toBe('merged')
  })

  it('creates a split mission with a plan', async () => {
    const c = client()
    const id = await createMission(c, {
      goal: 'Add audit logging.',
      mode: 'split',
      repoId: 'r-atlas',
      baseBranch: 'main',
      agents: [
        { agentId: 'a-claude-code', role: 'planner', enabled: true },
        { agentId: 'a-codex', role: 'builder', enabled: true },
      ],
    })
    const s = c.getState()
    expect(s.missions[id]?.status).toBe('working')
    const plan = Object.values(s.messages).find((m) => m.missionId === id && m.plan.length > 0)
    expect(plan?.plan).toHaveLength(3)
  })
})
