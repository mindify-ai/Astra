import {
  type GitCommit,
  type GitStatus,
  gitError,
  type MergeCheck,
  memoryGit,
  nextVersion,
  type PullRequest,
  resolveConflicts,
} from '@astra/client'
import { Button, Data, Eyebrow, Select, Spec, Status, TextInput } from '@astra/ui'
import { useNavigate } from '@tanstack/react-router'
import { useCallback, useEffect, useMemo, useState } from 'react'
import { useAstraState, useClient, useConnectivity, useHost, useUi } from '../context'
import { canApprove, gitMergeRows } from '../select'
import { MergeNotice, useResultMerge } from '../shared/git'
import { AppHeader, Page } from '../shell/AppHeader'

type Row = ReturnType<typeof gitMergeRows>[number]
function MergeRow({
  row,
  check,
  run,
  root,
  git,
  offline,
  busy,
}: {
  row: Row
  check?: MergeCheck
  run: (fn: () => Promise<void>) => void
  root: string
  git: ReturnType<typeof memoryGit>
  offline: boolean
  busy: boolean
}) {
  const client = useClient()
  const s = useAstraState()
  const ui = useUi()
  const host = useHost()
  const navigate = useNavigate()
  const action = useResultMerge(row.result)
  const conflicts = check ? !check.clean : row.pr?.mergeable === 'conflicts'
  const clean = check?.clean || row.pr?.mergeable === 'clean'
  const allowed = !row.result || canApprove(s, s.me, row.result.missionId, row.result.missionAgentId)
  return (
    <article className="flex flex-col gap-3 border-b border-line py-4">
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div className="min-w-0">
          <h3 className="m-0 text-heading">
            {row.title}
            {row.pr ? ` · #${row.pr.number}` : ''}
          </h3>
          <Data className="break-all text-ink-muted">
            {row.branch} → {row.into}
          </Data>
        </div>
        <Status tone={conflicts ? 'danger' : clean ? 'success' : 'working'}>
          {conflicts
            ? `CONFLICTS${check?.conflicts.length ? ` · ${check.conflicts.length} FILES` : ''}`
            : clean
              ? 'CLEAN'
              : 'CHECKING'}
        </Status>
      </div>
      {check?.conflicts.length ? <Spec className="break-all">{check.conflicts.join(' · ')}</Spec> : null}
      <div className="flex flex-wrap gap-2">
        {row.result ? (
          <Button
            size="sm"
            disabled={busy || action.busy || !allowed || !clean || conflicts}
            onClick={action.doMerge}
          >
            MERGE
          </Button>
        ) : null}
        {row.pr ? (
          <Button
            size="sm"
            variant="outline"
            onClick={() =>
              host.openUrl
                ? host.openUrl(row.pr!.url)
                : window.open(row.pr!.url, '_blank', 'noopener,noreferrer')
            }
          >
            VIEW
          </Button>
        ) : (
          <Button
            size="sm"
            variant="outline"
            disabled={busy || offline}
            onClick={() =>
              run(async () => {
                await git.createPullRequest(root, {
                  head: row.branch,
                  base: row.into,
                  title: row.title,
                  body: 'Ready for review.',
                })
              })
            }
          >
            OPEN PR
          </Button>
        )}
        {conflicts && row.pr?.localHead !== false ? (
          <Button
            size="sm"
            variant="outline"
            disabled={busy || action.busy}
            onClick={() =>
              run(async () => {
                const id = await resolveConflicts(client, {
                  repoId: ui.repoId,
                  branch: row.branch,
                  into: row.into,
                  files: check?.conflicts ?? [],
                })
                await navigate({ to: '/missions/$missionId', params: { missionId: id } })
              })
            }
          >
            AUTO-RESOLVE
          </Button>
        ) : null}
      </div>
      <MergeNotice action={action} />
    </article>
  )
}

/** Shared content keeps local git behavior identical on desktop and phones. */
export function GitContent({ mobile = false }: { mobile?: boolean }) {
  const client = useClient()
  const s = useAstraState()
  const ui = useUi()
  const host = useHost()
  const online = useConnectivity().online
  const repo = s.repositories[ui.repoId]
  const root = repo?.path ?? repo?.id ?? ''
  const git = useMemo(
    () => (repo?.path && host.git ? host.git : memoryGit(client.getState())),
    [client, host.git, repo?.path],
  )
  const [status, setStatus] = useState<GitStatus | null>(null)
  const [history, setHistory] = useState<GitCommit[]>([])
  const [tags, setTags] = useState<string[]>([])
  const [prs, setPrs] = useState<PullRequest[]>([])
  const [checks, setChecks] = useState<Record<string, MergeCheck>>({})
  const [notice, setNotice] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)
  const [message, setMessage] = useState('')
  const [versionOpen, setVersionOpen] = useState(false)
  const [bump, setBump] = useState<'patch' | 'minor' | 'major'>('patch')
  const rows = useMemo(() => gitMergeRows(s, ui.repoId, prs), [s, ui.repoId, prs])
  const refresh = useCallback(async () => {
    const local = await Promise.all([git.status(root), git.log(root, 20), git.tags(root)])
    setStatus(local[0])
    setHistory(local[1])
    setTags(local[2])
    if (online) {
      try {
        setPrs(await git.pullRequests(root))
      } catch (e) {
        setNotice(gitError(e).message)
      }
    }
  }, [git, root, online])
  useEffect(() => {
    let active = true
    setStatus(null)
    setChecks({})
    setPrs([])
    setNotice(null)
    setMessage('')
    setVersionOpen(false)
    // Guard results when changing repositories or leaving the screen.
    void (async () => {
      const local = await Promise.allSettled([
        git.status(root),
        git.log(root, 20),
        git.tags(root),
        online ? git.pullRequests(root) : Promise.resolve([]),
      ])
      if (!active) return
      const [status, history, tags, prs] = local
      if (status.status === 'fulfilled') setStatus(status.value)
      if (history.status === 'fulfilled') setHistory(history.value)
      if (tags.status === 'fulfilled') setTags(tags.value)
      if (prs.status === 'fulfilled') setPrs(prs.value)
      for (const item of local) if (item.status === 'rejected') setNotice(gitError(item.reason).message)
    })()
    return () => {
      active = false
    }
  }, [git, root, online])
  useEffect(() => {
    let active = true
    setChecks({})
    void Promise.all(
      rows
        .filter((r) => r.pr?.localHead !== false)
        .map(async (r) => {
          try {
            const check = await git.checkMerge(root, r.branch, r.into)
            if (active) setChecks((prev) => ({ ...prev, [r.key]: check }))
          } catch (e) {
            if (active) setNotice(gitError(e).message)
          }
        }),
    )
    return () => {
      active = false
    }
  }, [git, root, rows])
  const run = (fn: () => Promise<void>) => {
    if (busy) return
    setBusy(true)
    setNotice(null)
    void fn()
      .then(refresh)
      .catch((e) => setNotice(gitError(e).message))
      .finally(() => setBusy(false))
  }
  if (!repo) return <p className="text-ink-muted">Choose a repository to view Git.</p>
  const suggested = nextVersion(tags, bump)
  return (
    <div className="flex flex-col gap-6">
      <div className="flex flex-col gap-2">
        <Eyebrow>Git</Eyebrow>
        <h1 className="m-0 text-title">{repo.name}</h1>
        <Spec>
          {status ? `${status.branch} · ↑${status.ahead} ↓${status.behind}` : 'Loading repository…'}
          {!online ? ' · OFFLINE' : ''}
        </Spec>
        <p className="m-0 text-ink-muted">Review changes, merge work and mark a version.</p>
      </div>
      {notice ? (
        <p role="alert" className="m-0 border border-line p-3">
          {notice}
        </p>
      ) : null}
      {!mobile ? (
        <section aria-label="Changes" className="flex flex-col gap-3">
          <h2 className="m-0 text-heading">Changes</h2>
          {status?.changed.length ? (
            status.changed.map((f) => (
              <div
                key={f.path}
                className="flex flex-wrap justify-between gap-2 border-b border-line-subtle py-2"
              >
                <Data className="break-all">{f.path}</Data>
                <Spec>{f.state.toUpperCase()}</Spec>
              </div>
            ))
          ) : (
            <p className="m-0 text-ink-muted">Working tree is clean.</p>
          )}
          <Spec>COMMIT INCLUDES ALL CHANGED FILES</Spec>
          <TextInput
            aria-label="Commit message"
            placeholder="Describe your changes"
            value={message}
            onChange={(e) => setMessage(e.target.value)}
          />
          <div className="flex flex-wrap gap-2">
            <Button
              size="sm"
              disabled={busy || !message.trim() || !status?.changed.length}
              onClick={() =>
                run(async () => {
                  await git.commit(root, message.trim())
                  setMessage('')
                })
              }
            >
              COMMIT
            </Button>
            <Button
              size="sm"
              variant="outline"
              disabled={busy || !online || !status}
              onClick={() =>
                run(async () => {
                  const result = await git.pull(root)
                  if (!result.clean)
                    throw new Error(
                      `Pull needs a merge. Conflicts: ${result.conflicts.join(', ') || 'branches have diverged'}. No working files were changed.`,
                    )
                })
              }
            >
              PULL
            </Button>
            <Button
              size="sm"
              variant="outline"
              disabled={busy || !online || !status}
              onClick={() => run(() => git.push(root))}
            >
              PUSH
            </Button>
          </div>
        </section>
      ) : null}
      <section aria-label="Ready to merge">
        <h2 className="m-0 text-heading">Ready to merge</h2>
        <p className="text-ink-muted">
          Auto-resolve starts an agent mission. Review its result before merging.
        </p>
        {rows.map((row) => (
          <MergeRow
            key={row.key}
            row={row}
            check={checks[row.key]}
            root={root}
            git={git}
            offline={!online}
            busy={busy}
            run={run}
          />
        ))}
        {!rows.length ? <p className="text-ink-muted">No work waiting to merge.</p> : null}
      </section>
      {!mobile ? (
        <>
          <section aria-label="Versions" className="flex flex-col gap-3">
            <div className="flex items-center justify-between gap-3">
              <h2 className="m-0 text-heading">Versions</h2>
              <Button
                size="sm"
                variant="outline"
                disabled={busy || !status}
                onClick={() => setVersionOpen((v) => !v)}
              >
                NEW VERSION
              </Button>
            </div>
            <p className="m-0 text-ink-muted">A version marks the current commit with a local tag.</p>
            {versionOpen ? (
              <div className="flex flex-wrap items-center gap-3">
                <Select
                  aria-label="Version increment"
                  value={bump}
                  onChange={(e) => setBump(e.target.value as typeof bump)}
                >
                  <option value="patch">PATCH · {nextVersion(tags, 'patch')}</option>
                  <option value="minor">MINOR · {nextVersion(tags, 'minor')}</option>
                  <option value="major">MAJOR · {nextVersion(tags, 'major')}</option>
                </Select>
                <Button
                  size="sm"
                  disabled={busy}
                  onClick={() =>
                    run(async () => {
                      await git.createTag(root, suggested, `Release ${suggested}`)
                      setVersionOpen(false)
                    })
                  }
                >
                  CREATE {suggested}
                </Button>
              </div>
            ) : null}
            {tags.map((tag) => (
              <Data key={tag} className="border-b border-line-subtle py-2">
                {tag}
              </Data>
            ))}
          </section>
          <section aria-label="History" className="flex flex-col gap-3">
            <h2 className="m-0 text-heading">History</h2>
            {history.map((c) => (
              <div
                key={c.id}
                className="flex flex-wrap items-baseline justify-between gap-2 border-b border-line-subtle py-2"
              >
                <span>{c.message}</span>
                <Spec>
                  {c.id.slice(0, 8)} · {c.author} · {c.at.slice(0, 10)}
                </Spec>
              </div>
            ))}
          </section>
        </>
      ) : null}
    </div>
  )
}
export function Git() {
  return (
    <Page header={<AppHeader repoSelect nav="git" />}>
      <main className="mx-auto box-border w-full max-w-5xl grow px-8 pt-6 pb-8">
        <GitContent />
      </main>
    </Page>
  )
}
