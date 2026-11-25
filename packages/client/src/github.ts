import { type Auth, AuthError, type AuthErrorCode, type DeviceCode, type PollResult } from './auth'
import type { Repository } from './types'

/** One of your GitHub repositories. */
export interface GithubRepo {
  fullName: string
  name: string
  owner: string
  private: boolean
  defaultBranch: string
  cloneUrl: string
  description: string | null
  /** Last push, RFC 3339. */
  updatedAt: string | null
}

/**
 * GitHub on this machine (the desktop app): sign in with the app's own OAuth client (device flow, no server),
 * list your repositories and clone one. GitHub's token stays in the OS keychain behind this interface.
 */
export interface GithubHost {
  /** The account signed in on this app, or null. */
  readonly login: string | null
  /** The device flow against github.com. */
  readonly auth: Auth
  /** Your repositories, most recently pushed first. Rejects with `AuthError('signedOut')` without a session. */
  repos(): Promise<GithubRepo[]>
  /** Clones `owner/name` into `<parent>/<name>` (default parent `~/Astra`); resolves with the folder. */
  clone(fullName: string, parent: string | null): Promise<string>
  signOut(): Promise<void>
}

const knownCodes: AuthErrorCode[] = ['deviceFlowDisabled', 'badClientId', 'network', 'signedOut']

/** Maps an error from the shell (a code string, or a message) to an `AuthError`. */
export function githubError(e: unknown): AuthError {
  const text = typeof e === 'string' ? e : e instanceof Error ? e.message : String(e)
  const code = knownCodes.find((c) => c === text)
  return code ? new AuthError(code) : new AuthError('server', text)
}

/** The repository entity for a clone of `repo` at `path`. */
export function repositoryFromGithub(repo: GithubRepo, path: string): Repository {
  return {
    kind: 'repository',
    id: `r-gh-${repo.fullName.toLowerCase().replace(/[^a-z0-9]+/g, '-')}`,
    name: repo.fullName,
    source: 'github',
    defaultBranch: repo.defaultBranch,
    path,
    remoteUrl: repo.cloneUrl,
  }
}

/** The desktop app's GitHub, through the Rust shell. */
export function tauriGithub(login: string | null): GithubHost {
  const invoke = async <T>(cmd: string, args?: Record<string, unknown>): Promise<T> => {
    const core = await import('@tauri-apps/api/core')
    try {
      return await core.invoke<T>(cmd, args)
    } catch (e) {
      throw githubError(e)
    }
  }
  return {
    login,
    auth: {
      start: () => invoke<DeviceCode>('github_device_start'),
      poll: (_provider, deviceCode) => invoke<PollResult>('github_device_poll', { deviceCode }),
    },
    repos: () => invoke<GithubRepo[]>('github_repos'),
    clone: (fullName, parent) => invoke<string>('github_clone', { fullName, parent }),
    signOut: () => invoke<void>('github_sign_out'),
  }
}

/** A GitHub for tests and design review: a few repositories; cloning "succeeds" under `/Users/you/Astra`. */
export class MockGithub implements GithubHost {
  cloned: string[] = []
  constructor(
    readonly login: string | null = 'octocat',
    readonly auth: Auth = {
      start: () => Promise.reject(new AuthError('network')),
      poll: () => Promise.reject(new AuthError('network')),
    },
    private readonly list: GithubRepo[] = MockGithub.sample,
  ) {}

  static sample: GithubRepo[] = [
    {
      fullName: 'octocat/hello-world',
      name: 'hello-world',
      owner: 'octocat',
      private: false,
      defaultBranch: 'main',
      cloneUrl: 'https://github.com/octocat/hello-world.git',
      description: 'My first repository',
      updatedAt: '2026-11-20T10:00:00Z',
    },
    {
      fullName: 'octo-org/payments',
      name: 'payments',
      owner: 'octo-org',
      private: true,
      defaultBranch: 'trunk',
      cloneUrl: 'https://github.com/octo-org/payments.git',
      description: null,
      updatedAt: '2026-11-18T08:00:00Z',
    },
  ]

  async repos() {
    if (!this.login) throw new AuthError('signedOut')
    return this.list
  }

  async clone(fullName: string, parent: string | null) {
    this.cloned.push(fullName)
    return `${parent ?? '/Users/you/Astra'}/${fullName.split('/')[1]}`
  }

  async signOut() {}
}
