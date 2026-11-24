import { describe, expect, it } from 'vitest'
import seed from '../../../fixtures/seed.json'
import { archiveMission, deleteMission, sendMessage } from '../src/actions'
import { MockClient } from '../src/mock'
import type { SeedFile } from '../src/store'

const client = () => new MockClient({ seed: seed as SeedFile, latency: 0 })

describe('archiving and deleting missions', () => {
  it('archives a running mission: agents stop, approvals are denied, and it can come back', async () => {
    const c = client()
    expect(await archiveMission(c, 'm-014')).toEqual({ ok: true, error: null })
    let s = c.getState()
    expect(s.missions['m-014']?.archivedAt).not.toBeNull()
    expect(s.missions['m-014']?.status).toBe('needsYou')
    expect(s.missionAgents['ma-014-builder']).toMatchObject({
      status: 'idle',
      statusDetail: 'stopped: archived',
    })
    expect(s.approvals['ap-0042']?.state).toBe('denied')
    expect(await archiveMission(c, 'm-014', false)).toEqual({ ok: true, error: null })
    s = c.getState()
    expect(s.missions['m-014']?.archivedAt).toBeNull()
  })

  it('deletes a mission and everything in it', async () => {
    const c = client()
    expect(await deleteMission(c, 'm-015')).toEqual({ ok: true, error: null })
    const s = c.getState()
    expect(s.missions['m-015']).toBeUndefined()
    for (const m of [s.missionAgents, s.tasks, s.messages, s.results, s.memberships])
      expect(Object.values(m).some((e) => e.missionId === 'm-015')).toBe(false)
    expect(s.missions['m-014']).toBeDefined()
  })

  it('only the owner may archive or delete', async () => {
    const c = new MockClient({ seed: { ...(seed as SeedFile), me: 'p-lin' }, latency: 0 })
    expect((await archiveMission(c, 'm-014'))?.error).toMatch(/only the owner/)
    expect((await deleteMission(c, 'm-014'))?.error).toMatch(/only the owner/)
    // Messages still work: sending isn't blocked by a failed archive.
    expect(await sendMessage(c, 'm-014', null, 'still here')).toEqual({ ok: true, error: null })
  })
})
