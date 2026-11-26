import type { State } from './types'

export interface GitStatus {
  branch: string
  upstream: string | null
  ahead: number
  behind: number
  changed: { path: string; state: string }[]
}
export interface GitCommit {
  id: string
  message: string
  author: string
  at: string
}
export interface MergeCheck {
  clean: boolean
  conflicts: string[]
}
export interface PullRequest {
  number: number
  title: string
  head: string
  base: string
  url: string
  state: 'open' | 'closed'
  mergeable: 'clean' | 'conflicts' | 'unknown'
  /** Fork PRs can be viewed, but cannot be resolved in the local repository. */
  localHead?: boolean
}
export interface PullRequestDraft {
  head: string
  base: string
  title: string
  body: string
}
/** Local git access. Credentials stay in the host; no operation uses the relay. */
export interface GitHost {
  status(root: string): Promise<GitStatus>
  log(root: string, limit: number): Promise<GitCommit[]>
  branches(root: string): Promise<string[]>
  tags(root: string): Promise<string[]>
  /** Commits all changed files, including new files and deletions. */
  commit(root: string, message: string): Promise<void>
  pull(root: string): Promise<MergeCheck>
  push(root: string): Promise<void>
  createTag(root: string, name: string, message: string): Promise<void>
  /** In-memory merge only; never modifies the index or working tree. */
  checkMerge(root: string, branch: string, into: string): Promise<MergeCheck>
  pullRequests(root: string): Promise<PullRequest[]>
  createPullRequest(root: string, draft: PullRequestDraft): Promise<PullRequest>
}
export type GitErrorCode = 'signedOut' | 'network' | 'noUpstream' | 'dirty' | 'other'
const messages: Record<GitErrorCode, string> = {
  signedOut: 'Sign in to GitHub to continue.',
  network: 'Cannot reach the remote. Check your connection and try again.',
  noUpstream: 'This branch has no upstream. Set its upstream in Git before pulling or pushing.',
  dirty: 'Commit or stash your changes before pulling.',
  other: 'Git could not complete the action.',
}
export class GitError extends Error {
  constructor(
    readonly code: GitErrorCode,
    message = messages[code],
  ) {
    super(message)
    this.name = 'GitError'
  }
}
export function gitError(e: unknown): GitError {
  if (e instanceof GitError) return e
  const message = e instanceof Error ? e.message : String(e)
  return message in messages ? new GitError(message as GitErrorCode) : new GitError('other', message)
}
const version = (tag: string) =>
  /^v?(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/.exec(tag)?.slice(1).map(Number)
export function nextVersion(tags: string[], bump: 'patch' | 'minor' | 'major'): string {
  const latest = tags
    .map(version)
    .filter((v): v is number[] => !!v)
    .sort((a, b) => b[0]! - a[0]! || b[1]! - a[1]! || b[2]! - a[2]!)[0]
  if (!latest) return 'v0.1.0'
  const [major = 0, minor = 0, patch = 0] = latest
  return bump === 'major'
    ? `v${major + 1}.0.0`
    : bump === 'minor'
      ? `v${major}.${minor + 1}.0`
      : `v${major}.${minor}.${patch + 1}`
}

/** Demo git, partitioned by repository id. The first result has a visible conflict. */
export function memoryGit(state: State): GitHost {
  const repos = new Map<string, { status: GitStatus; log: GitCommit[]; tags: string[]; prs: PullRequest[] }>()
  const repoOf = (root: string) =>
    state.repositories[root] ??
    Object.values(state.repositories).find((r) => r.path === root) ??
    Object.values(state.repositories)[0]
  const results = (root: string) =>
    Object.values(state.results).filter((r) => state.missions[r.missionId]?.repoId === repoOf(root)?.id)
  const get = (root: string) => {
    const key = repoOf(root)?.id ?? root
    let data = repos.get(key)
    if (!data) {
      data = {
        status: {
          branch: repoOf(root)?.defaultBranch ?? 'main',
          upstream: 'origin/main',
          ahead: 0,
          behind: 0,
          changed: [{ path: 'README.md', state: 'modified' }],
        },
        log: Object.values(state.missions)
          .filter((m) => m.repoId === key)
          .map((m) => ({
            id: m.id,
            message: m.title,
            author: state.people[m.ownerId]?.name ?? 'You',
            at: m.createdAt,
          })),
        tags: ['v0.2.0', 'v0.1.0'],
        prs: [],
      }
      repos.set(key, data)
    }
    return data
  }
  return {
    async status(root) {
      return structuredClone(get(root).status)
    },
    async log(root, limit) {
      return get(root).log.slice(0, limit)
    },
    async tags(root) {
      return [...get(root).tags]
    },
    async branches(root) {
      return [get(root).status.branch, ...results(root).map((r) => r.branch)]
    },
    async commit(root, message) {
      if (!message.trim()) throw new Error('Enter a commit message.')
      const data = get(root)
      if (!data.status.changed.length) throw new Error('No changes to commit.')
      data.log.unshift({
        id: `demo-${data.log.length}`,
        message,
        author: 'You',
        at: new Date().toISOString(),
      })
      data.status = { ...data.status, changed: [], ahead: data.status.ahead + 1 }
    },
    async pull() {
      return { clean: true, conflicts: [] }
    },
    async push(root) {
      get(root).status.ahead = 0
    },
    async createTag(root, name) {
      if (get(root).tags.includes(name)) throw new Error('That version already exists.')
      get(root).tags.unshift(name)
    },
    async checkMerge(root, branch) {
      const first = results(root)[0]
      const conflicts = first?.branch === branch ? first.diff.map((f) => f.path) : []
      return { clean: !conflicts.length, conflicts }
    },
    async pullRequests(root) {
      return [...get(root).prs]
    },
    async createPullRequest(root, draft) {
      const pr: PullRequest = {
        ...draft,
        number: get(root).prs.length + 1,
        url: `https://github.com/${repoOf(root)?.name}/pull/${get(root).prs.length + 1}`,
        state: 'open',
        mergeable: 'unknown',
        localHead: true,
      }
      get(root).prs.push(pr)
      return pr
    },
  }
}
