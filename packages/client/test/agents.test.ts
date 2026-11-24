import { describe, expect, it } from 'vitest'
import seed from '../../../fixtures/seed.json'
import { addAgent, removeAgent, switchAgent } from '../src/actions'
import { MockClient } from '../src/mock'
import type { SeedFile } from '../src/store'

const client = () => new MockClient({ seed: seed as SeedFile, latency: 0 })

describe("changing a mission's agents", () => {
  it('switches a working builder to another harness, which takes over', async () => {
    const c = client()
    expect(await switchAgent(c, 'ma-014-builder', 'a-claude-code')).toEqual({ ok: true, error: null })
    const s = c.getState()
    expect(s.missionAgents['ma-014-builder']?.agentId).toBe('a-claude-code')
    expect(s.missionAgents['ma-014-builder']?.statusDetail).toBe('switched from Codex')
    expect(Object.values(s.messages).some((m) => m.body.startsWith('Taking over from Codex'))).toBe(true)
  })

  it('refuses agents that are not connected, and agents waiting on an approval', async () => {
    const c = client()
    expect(await switchAgent(c, 'ma-014-builder', 'a-cursor-cli')).toEqual({
      ok: false,
      error: 'Cursor CLI is not connected',
    })
    expect((await switchAgent(c, 'ma-014-tester', 'a-codex'))?.error).toBe(
      'decide its pending approval first',
    )
    expect(c.getState().missionAgents['ma-014-builder']?.agentId).toBe('a-codex')
  })

  it('adds a second builder to a mission under way and starts it', async () => {
    const c = client()
    const { missionAgentId, outcome } = await addAgent(c, 'm-014', 'builder', 'a-gemini-cli')
    expect(outcome).toEqual({ ok: true, error: null })
    const ma = c.getState().missionAgents[missionAgentId]
    expect(ma).toMatchObject({ role: 'builder', agentId: 'a-gemini-cli', enabled: true })
  })

  it('removes an agent but keeps it for the history, and never the last one', async () => {
    const c = client()
    expect(await removeAgent(c, 'ma-014-reviewer')).toEqual({ ok: true, error: null })
    expect(c.getState().missionAgents['ma-014-reviewer']).toMatchObject({
      enabled: false,
      status: 'done',
      statusDetail: 'removed',
    })
    expect((await removeAgent(c, 'ma-013-writer'))?.error).toBe('a mission needs at least one agent')
  })
})
