import {
  AuthError,
  addRepository,
  type GithubHost,
  type GithubRepo,
  repositoryFromGithub,
} from '@astra/client'
import { Button, buttonClass, Eyebrow, Spec, Status } from '@astra/ui'
import { Link } from '@tanstack/react-router'
import { useEffect, useMemo, useState } from 'react'
import { useAstraState, useClient, useHost, useUi } from '../context'

/** At most this many rows; search narrows the rest. */
const SHOWN = 50

const tilde = (path: string) => path.replace(/^\/(Users|home)\/[^/]+/, '~')

type Load = { kind: 'loading' } | { kind: 'ready'; repos: GithubRepo[] } | { kind: 'error'; message: string }

/**
 * Your GitHub repositories on the Repo screen (desktop app): search, then one click clones a repository into
 * `~/Astra` (or a folder you pick) and adds it to the workspace. Signed out, it offers to sign in.
 */
export function GithubRepos({ github, query }: { github: GithubHost; query: string }) {
  const state = useAstraState()
  const client = useClient()
  const host = useHost()
  const ui = useUi()
  const [signedIn, setSignedIn] = useState(github.login !== null)
  const [load, setLoad] = useState<Load>({ kind: 'loading' })
  const [parent, setParent] = useState<string | null>(null)
  const [cloning, setCloning] = useState<string | null>(null)
  const [note, setNote] = useState<{ tone: 'success' | 'danger'; text: string } | null>(null)

  useEffect(() => {
    if (!signedIn) return
    let live = true
    github.repos().then(
      (repos) => live && setLoad({ kind: 'ready', repos }),
      (e: unknown) => {
        if (!live) return
        if (e instanceof AuthError && e.code === 'signedOut') return setSignedIn(false)
        setLoad({
          kind: 'error',
          message:
            e instanceof AuthError && e.code === 'network'
              ? 'Astra could not reach GitHub. Check your connection.'
              : 'Astra could not list your GitHub repositories.',
        })
      },
    )
    return () => {
      live = false
    }
  }, [github, signedIn])

  const added = useMemo(() => {
    const byUrl = new Map<string, string>()
    for (const r of Object.values(state.repositories)) if (r.remoteUrl) byUrl.set(r.remoteUrl, r.id)
    return byUrl
  }, [state.repositories])

  if (!signedIn) {
    return (
      <section aria-labelledby="gh-repos" className="flex flex-col gap-3 border border-line p-4">
        <Eyebrow as="h2" id="gh-repos">
          Clone from GitHub
        </Eyebrow>
        <p className="m-0 text-ink-muted">
          Sign in with GitHub to pick one of your repositories and clone it here.
        </p>
        <Link
          to="/signin"
          search={{ provider: 'github' }}
          className={`${buttonClass('primary', 'md')} self-start`}
        >
          Sign in with GitHub →
        </Link>
      </section>
    )
  }

  const q = query.trim().toLowerCase()
  const matches =
    load.kind === 'ready'
      ? load.repos.filter(
          (r) => r.fullName.toLowerCase().includes(q) || r.description?.toLowerCase().includes(q),
        )
      : []

  const clone = async (repo: GithubRepo) => {
    setCloning(repo.fullName)
    setNote(null)
    try {
      const path = await github.clone(repo.fullName, parent)
      const entity = repositoryFromGithub(repo, path)
      await addRepository(client, entity)
      ui.setRepoId(entity.id)
      setNote({ tone: 'success', text: `Cloned ${repo.fullName} into ${tilde(path)}` })
    } catch (e) {
      setNote({
        tone: 'danger',
        text:
          e instanceof AuthError && e.code === 'signedOut'
            ? 'GitHub signed you out. Sign in again to clone.'
            : e instanceof AuthError && e.message !== e.code
              ? e.message
              : `Could not clone ${repo.fullName}.`,
      })
    } finally {
      setCloning(null)
    }
  }

  const pickParent = async () => {
    const picked = await host.pickFolder?.()
    if (picked) setParent(picked)
  }

  return (
    <section aria-labelledby="gh-repos" className="flex flex-col gap-2">
      <div className="flex flex-wrap items-baseline justify-between gap-x-4 gap-y-1">
        <Eyebrow as="h2" id="gh-repos">
          Your GitHub repositories{github.login ? ` · @${github.login}` : ''}
        </Eyebrow>
        <span className="flex items-baseline gap-2">
          <Spec>Clones go to {parent ? tilde(parent) : '~/Astra'}</Spec>
          {host.pickFolder ? (
            <Button variant="text" size="sm" onClick={pickParent}>
              Change
            </Button>
          ) : null}
        </span>
      </div>
      {load.kind === 'loading' ? <Status tone="working">Loading your repositories</Status> : null}
      {load.kind === 'error' ? (
        <p role="alert" className="m-0">
          <Status tone="danger">{load.message}</Status>
        </p>
      ) : null}
      {load.kind === 'ready' ? (
        <ul aria-label="GitHub repositories" className="m-0 list-none border-t border-line p-0">
          {matches.slice(0, SHOWN).map((r) => {
            const addedId = added.get(r.cloneUrl)
            return (
              <li
                key={r.fullName}
                className="flex min-h-12 flex-wrap items-center gap-x-4 gap-y-1 border-b border-line-subtle px-1 py-1"
              >
                <span className="flex flex-[1_1_240px] flex-col">
                  <span className="font-mono text-data">{r.fullName}</span>
                  {r.description ? <span className="text-[13px] text-ink-muted">{r.description}</span> : null}
                </span>
                <Spec className="flex-[0_0_70px]">{r.private ? 'PRIVATE' : 'PUBLIC'}</Spec>
                <span className="flex-[0_0_60px] font-mono text-data text-ink-muted">{r.defaultBranch}</span>
                <span className="flex flex-[0_0_96px] justify-end">
                  {cloning === r.fullName ? (
                    <Status tone="working">Cloning</Status>
                  ) : addedId ? (
                    <Button variant="text" size="sm" onClick={() => ui.setRepoId(addedId)}>
                      {ui.repoId === addedId ? 'Selected' : 'Select'}
                    </Button>
                  ) : (
                    <Button
                      variant="outline"
                      size="sm"
                      disabled={cloning !== null}
                      onClick={() => void clone(r)}
                      aria-label={`Clone ${r.fullName}`}
                    >
                      Clone
                    </Button>
                  )}
                </span>
              </li>
            )
          })}
          {matches.length === 0 ? (
            <li className="p-4 text-ink-muted">
              {load.repos.length === 0
                ? 'No repositories on this GitHub account yet.'
                : 'No repository matches that search.'}
            </li>
          ) : null}
          {matches.length > SHOWN ? (
            <li className="px-1 py-2">
              <Spec>
                Showing {SHOWN} of {matches.length}. Search to narrow.
              </Spec>
            </li>
          ) : null}
        </ul>
      ) : null}
      <div role="status" className="min-h-5 text-[13px]">
        {note ? <Status tone={note.tone}>{note.text}</Status> : null}
      </div>
    </section>
  )
}
