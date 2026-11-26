import { describe, expect, it, vi } from 'vitest'
import {
  demoSeed,
  gitError,
  MockClient,
  memoryGit,
  nextVersion,
  resolveConflicts,
  stateFromSeed,
} from '../src'

describe('git host', () => {
  it('suggests numeric semantic versions and ignores other tags', () => {
    expect(nextVersion([], 'major')).toBe('v0.1.0')
    expect(nextVersion(['release', 'v1.2.3-beta', 'v01.2.3'], 'patch')).toBe('v0.1.0')
    expect(nextVersion(['v1.2.9', 'v1.2.10', 'v1.1.20'], 'patch')).toBe('v1.2.11')
    expect(nextVersion(['1.2.3'], 'minor')).toBe('v1.3.0')
    expect(nextVersion(['v1.2.3'], 'major')).toBe('v2.0.0')
  })
  it('maps host errors to useful messages', () => {
    expect(gitError('dirty').code).toBe('dirty')
    expect(gitError(new Error('noUpstream')).code).toBe('noUpstream')
    expect(gitError('network').message).toContain('connection')
    expect(gitError('signedOut').code).toBe('signedOut')
    expect(gitError(new Error('No changes')).message).toBe('No changes')
  })
  it('previews conflicts without mutating status; commits, versions and PRs stay per repository', async () => {
    const s = stateFromSeed(demoSeed)
    const git = memoryGit(s)
    const result = s.results['res-015-a']!
    const root = s.missions[result.missionId]!.repoId
    const before = await git.status(root)
    const check = await git.checkMerge(root, result.branch, 'main')
    expect(check.clean).toBe(false)
    expect(check.conflicts).toEqual(result.diff.map((f) => f.path))
    expect(await git.checkMerge(root, 'unrelated', 'main')).toEqual({ clean: true, conflicts: [] })
    expect(await git.status(root)).toEqual(before)
    await git.commit(root, 'Update readme')
    expect((await git.status(root)).changed).toEqual([])
    expect((await git.status('r-atlas')).changed.length).toBeGreaterThan(0)
    await git.createTag(root, 'v1.0.0', 'Release')
    expect(await git.tags('r-atlas')).not.toContain('v1.0.0')
    const pr = await git.createPullRequest(root, {
      head: result.branch,
      base: 'main',
      title: 'Review',
      body: '',
    })
    expect(await git.pullRequests(root)).toEqual([pr])
    expect(await git.pullRequests('r-atlas')).toEqual([])
  })
  it('starts a single builder repair mission from the conflicting branch, without merging or pushing', async () => {
    const client = new MockClient({ seed: demoSeed, latency: 0 })
    const submit = vi.spyOn(client, 'submit')
    const id = await resolveConflicts(client, {
      repoId: 'r-gateway',
      branch: 'astra/feature',
      into: 'main',
      files: ['src/a.ts', 'README.md'],
    })
    const s = client.getState()
    expect(s.missions[id]).toMatchObject({
      baseBranch: 'astra/feature',
      title: 'Resolve conflicts: astra/feature → main',
      mode: 'split',
    })
    expect(s.missions[id]?.goal).toContain("git merge 'main'")
    expect(s.missions[id]?.goal).toContain('src/a.ts')
    expect(s.missions[id]?.goal).toContain('README.md')
    expect(s.missions[id]?.goal).toContain('Do not push')
    expect(submit.mock.calls[0]?.[0]).toContainEqual(
      expect.objectContaining({ type: 'command', command: { type: 'startMission', missionId: id } }),
    )
    expect(Object.values(s.missionAgents).filter((a) => a.missionId === id)).toHaveLength(1)
    expect(s.merges[id]).toBeUndefined()
    client.dispose()
  })
  it('explains when no connected agent can run a repair', async () => {
    const seed = structuredClone(demoSeed)
    for (const e of seed.entities) if (e.kind === 'agent') e.status = 'notConnected'
    const client = new MockClient({ seed, latency: 0 })
    await expect(
      resolveConflicts(client, { repoId: 'r-gateway', branch: 'feature', into: 'main', files: [] }),
    ).rejects.toThrow('Connect an agent')
    client.dispose()
  })
})
