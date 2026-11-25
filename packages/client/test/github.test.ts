import { describe, expect, it } from 'vitest'
import { AuthError } from '../src/auth'
import { githubError, MockGithub, repositoryFromGithub } from '../src/github'

describe('github', () => {
  it('maps shell errors to auth errors, keeping messages', () => {
    expect(githubError('signedOut')).toMatchObject({ code: 'signedOut' })
    expect(githubError('deviceFlowDisabled')).toMatchObject({ code: 'deviceFlowDisabled' })
    const e = githubError('/Users/you/Astra/x already exists. Pick another folder to clone into.')
    expect(e).toBeInstanceOf(AuthError)
    expect(e).toMatchObject({ code: 'server', message: expect.stringContaining('already exists') })
  })

  it('turns a clone into a repository entity', () => {
    const [repo] = MockGithub.sample
    expect(repositoryFromGithub(repo!, '/Users/you/Astra/hello-world')).toEqual({
      kind: 'repository',
      id: 'r-gh-octocat-hello-world',
      name: 'octocat/hello-world',
      source: 'github',
      defaultBranch: 'main',
      path: '/Users/you/Astra/hello-world',
      remoteUrl: 'https://github.com/octocat/hello-world.git',
    })
  })

  it('a signed-out mock refuses to list', async () => {
    await expect(new MockGithub(null).repos()).rejects.toMatchObject({ code: 'signedOut' })
  })
})
