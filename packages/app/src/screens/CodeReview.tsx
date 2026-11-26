import { type DiffFile, type Message, type RaceResult, type State, sendMessage } from '@astra/client'
import {
  AgentGlyph,
  Avatar,
  Badge,
  Button,
  buttonClass,
  ChevronLeftIcon,
  cx,
  Data,
  DiffCode,
  DiffGutterButton,
  DiffLineNo,
  diffGroundClass,
  Eyebrow,
  Field,
  formatDiffStats,
  inlineLinkClass,
  Markdown,
  ResizeHandle,
  Spec,
  Status,
  TextArea,
  useResizable,
} from '@astra/ui'
import { Link } from '@tanstack/react-router'
import { Fragment, type ReactNode, useEffect, useMemo, useRef, useState } from 'react'
import { useAstraState, useClient } from '../context'
import { agentOf, canApprove, canSteer, hhmm, members, missionNo, roleLetter, roleName } from '../select'
import { MergeNotice, useResultMerge } from '../shared/git'
import {
  anchorOf,
  fileStats,
  formatChangeRequest,
  formatLineComment,
  groupByLine,
  type LineAnchor,
  lineCommentsOf,
  lineRef,
  type NumberedLine,
  numberLines,
  type ReviewComment,
  splitRows,
} from '../shared/review'
import { messagesOf } from '../shared/thread'
import { AppHeader, Page, ShareButton } from '../shell/AppHeader'
import { MissionNotFound, useMissionPresence } from './common'
import { checksStatus, useVerdict } from './Review'

type Layout = 'unified' | 'split'

/** "line 3" on the new side, "removed line 2" on the old. */
const lineName = (a: LineAnchor) => (a.side === 'old' ? `removed line ${a.line}` : `line ${a.line}`)

const plusMinus = (f: DiffFile) => {
  const st = fileStats(f)
  return `+${st.additions} −${st.deletions}`
}

/** Which files this viewer has marked viewed, per result. Kept in localStorage when it is available. */
function useViewed(resultId: string) {
  const key = `astra:review:viewed:${resultId}`
  const [viewed, setViewed] = useState<Set<string>>(() => {
    try {
      const raw = window.localStorage.getItem(key)
      const list: unknown = raw ? JSON.parse(raw) : []
      return new Set(Array.isArray(list) ? list.filter((p): p is string => typeof p === 'string') : [])
    } catch {
      return new Set()
    }
  })
  useEffect(() => {
    try {
      window.localStorage.setItem(key, JSON.stringify([...viewed]))
    } catch {
      // Private windows and blocked storage: viewed marks last for this visit only.
    }
  }, [key, viewed])
  return [viewed, setViewed] as const
}

function authorOf(s: State, m: Message): { avatar: ReactNode; name: string } {
  if (m.author.type === 'person') {
    const p = s.people[m.author.personId]
    return { avatar: <Avatar initials={p?.initials ?? '?'} size={24} />, name: p?.name ?? m.author.personId }
  }
  const ma = s.missionAgents[m.author.missionAgentId]
  return {
    avatar: <AgentGlyph letter={ma ? roleLetter(ma.role) : 'A'} className="size-6" />,
    name: ma ? roleName[ma.role] : 'Agent',
  }
}

function CommentItem({ item, showRef }: { item: ReviewComment; showRef?: boolean }) {
  const s = useAstraState()
  const { avatar, name } = authorOf(s, item.message)
  return (
    <li className="flex gap-3">
      {avatar}
      <div className="flex min-w-0 flex-col gap-1">
        <div className="flex flex-wrap items-baseline gap-2">
          <b className="font-semibold">{name}</b>
          <Spec>
            {item.message.author.type === 'person' && item.message.author.personId === s.me ? 'YOU · ' : ''}
            {hhmm(item.message.at)}
            {showRef ? ` · ${lineRef(item.comment)}` : ''}
          </Spec>
        </div>
        <Markdown>{item.comment.text}</Markdown>
      </div>
    </li>
  )
}

/** A text box with a primary and a cancel button; ⌘/Ctrl+Enter submits, Escape cancels. */
function Composer({
  id,
  label,
  submitLabel,
  placeholder,
  onSubmit,
  onCancel,
}: {
  /** Set when a surrounding Field labels the box; otherwise pass `label`. */
  id?: string
  label?: string
  submitLabel: string
  placeholder: string
  onSubmit: (text: string) => Promise<void>
  onCancel: () => void
}) {
  const [text, setText] = useState('')
  const [busy, setBusy] = useState(false)
  const box = useRef<HTMLDivElement>(null)
  useEffect(() => box.current?.querySelector('textarea')?.focus(), [])
  const submit = async () => {
    if (!text.trim() || busy) return
    setBusy(true)
    try {
      await onSubmit(text)
      setText('')
    } finally {
      setBusy(false)
    }
  }
  return (
    <div ref={box} className="flex flex-col gap-2">
      <TextArea
        id={id}
        aria-label={label}
        placeholder={placeholder}
        value={text}
        onChange={(e) => setText(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) {
            e.preventDefault()
            void submit()
          } else if (e.key === 'Escape') {
            e.preventDefault()
            onCancel()
          }
        }}
        className="w-full text-body-sm"
      />
      <div className="flex items-center gap-4">
        <Button size="sm" onClick={() => void submit()} disabled={!text.trim() || busy}>
          {submitLabel}
        </Button>
        <Button variant="text" size="sm" onClick={onCancel}>
          Cancel
        </Button>
      </div>
    </div>
  )
}

interface LineThreadProps {
  path: string
  anchor: LineAnchor
  comments: ReviewComment[]
  composing: boolean
  mayComment: boolean
  colSpan: number
  onOpen: (key: string) => void
  onCancel: () => void
  onSubmit: (anchor: LineAnchor, text: string) => Promise<void>
}

/** Inline comments under a line, and the composer when it is open there. */
function LineThread({
  path,
  anchor,
  comments,
  composing,
  mayComment,
  colSpan,
  onOpen,
  onCancel,
  onSubmit,
}: LineThreadProps) {
  if (comments.length === 0 && !composing) return null
  const where = `${path} ${lineName(anchor)}`
  return (
    <tr>
      <td colSpan={colSpan} className="border-y border-line bg-surface p-0">
        <div className="flex max-w-[760px] flex-col gap-3 px-4 py-3">
          {comments.length > 0 ? (
            <ul aria-label={`Comments on ${where}`} className="m-0 flex list-none flex-col gap-3 p-0">
              {comments.map((c) => (
                <CommentItem key={c.message.id} item={c} />
              ))}
            </ul>
          ) : null}
          {composing ? (
            <Composer
              label={`Comment on ${where}`}
              submitLabel="Comment"
              placeholder="Leave a comment on this line"
              onSubmit={(text) => onSubmit(anchor, text)}
              onCancel={onCancel}
            />
          ) : mayComment ? (
            <Button variant="text" size="sm" className="self-start" onClick={() => onOpen(lineRef(anchor))}>
              Reply
            </Button>
          ) : null}
        </div>
      </td>
    </tr>
  )
}

interface FileDiffProps {
  file: DiffFile
  layout: Layout
  collapsed: boolean
  viewed: boolean
  active: boolean
  comments: Map<string, ReviewComment[]>
  composer: string | null
  mayComment: boolean
  onToggle: () => void
  onOpen: (key: string) => void
  onCancel: () => void
  onSubmit: (anchor: LineAnchor, text: string) => Promise<void>
  sectionRef: (el: HTMLElement | null) => void
}

function FileDiff(props: FileDiffProps) {
  const { file, layout, collapsed, viewed, active, comments, composer, mayComment } = props
  const lines = useMemo(() => numberLines(file.lines), [file.lines])
  const path = file.path

  const gutter = (l: NumberedLine, numbers: ReactNode) => {
    const a = anchorOf(path, l)
    return (
      <DiffGutterButton
        aria-label={`Comment on ${path} ${lineName(a)}`}
        title={mayComment ? undefined : 'Only people who can steer can comment'}
        disabled={!mayComment}
        onClick={() => props.onOpen(lineRef(a))}
      >
        {numbers}
      </DiffGutterButton>
    )
  }
  const thread = (l: NumberedLine, colSpan: number) => {
    const a = anchorOf(path, l)
    const key = lineRef(a)
    return (
      <LineThread
        path={path}
        anchor={a}
        comments={comments.get(key) ?? []}
        composing={composer === key}
        mayComment={mayComment}
        colSpan={colSpan}
        onOpen={props.onOpen}
        onCancel={props.onCancel}
        onSubmit={props.onSubmit}
      />
    )
  }

  // Comments that point at this file but at no line shown in this diff.
  const shown = new Set(lines.map((l) => lineRef(anchorOf(path, l))))
  const elsewhere: ReviewComment[] = []
  for (const [key, list] of comments) {
    if (!shown.has(key) && list[0]?.comment.path === path) elsewhere.push(...list)
  }
  const count = [...comments.values()].reduce(
    (n, list) => n + (list[0]?.comment.path === path ? list.length : 0),
    0,
  )

  return (
    <section
      ref={props.sectionRef}
      aria-label={path}
      className={cx('scroll-mt-4 border', active ? 'border-ink' : 'border-line')}
    >
      <h3 className="m-0 text-body-sm font-normal">
        <button
          type="button"
          aria-expanded={!collapsed}
          onClick={props.onToggle}
          className={cx(
            'flex w-full cursor-pointer items-center gap-3 border-0 bg-surface-raised px-4 py-2 text-left text-ink',
            !collapsed && 'border-b border-line',
          )}
        >
          <ChevronLeftIcon size={16} className={collapsed ? 'rotate-180' : '-rotate-90'} />
          <Data className="min-w-0 grow truncate">{path}</Data>
          {count > 0 ? <Spec>{count === 1 ? '1 comment' : `${count} comments`}</Spec> : null}
          {viewed ? <Spec>Viewed</Spec> : null}
          <Data className="shrink-0 text-ink-muted">{plusMinus(file)}</Data>
        </button>
      </h3>
      {collapsed ? null : (
        <>
          {elsewhere.length > 0 ? (
            <div className="flex flex-col gap-3 border-b border-line bg-surface px-4 py-3">
              <Eyebrow as="h4" className="m-0">
                On lines outside this diff
              </Eyebrow>
              <ul className="m-0 flex list-none flex-col gap-3 p-0">
                {elsewhere.map((c) => (
                  <CommentItem key={c.message.id} item={c} showRef />
                ))}
              </ul>
            </div>
          ) : null}
          <div className="overflow-x-auto bg-surface-sunken">
            <table className="min-w-full border-collapse" aria-label={`Diff of ${path}`}>
              <tbody>
                {layout === 'unified'
                  ? lines.map((l, i) => (
                      // biome-ignore lint/suspicious/noArrayIndexKey: diff lines have no identity beyond position
                      <Fragment key={i}>
                        <tr className={diffGroundClass[l.kind]}>
                          <td className="w-px p-0 align-top whitespace-nowrap">
                            {gutter(
                              l,
                              <>
                                <DiffLineNo n={l.old} />
                                <DiffLineNo n={l.new} />
                              </>,
                            )}
                          </td>
                          <DiffCode kind={l.kind} text={l.text} className="w-full" />
                        </tr>
                        {thread(l, 2)}
                      </Fragment>
                    ))
                  : splitRows(lines).map((row, i) => {
                      const { left, right } = row
                      const ctx = left?.kind === 'ctx'
                      return (
                        // biome-ignore lint/suspicious/noArrayIndexKey: diff rows have no identity beyond position
                        <Fragment key={i}>
                          <tr>
                            <td
                              className={cx(
                                'w-px p-0 align-top whitespace-nowrap',
                                left ? diffGroundClass[left.kind] : 'bg-surface-alt',
                              )}
                            >
                              {left && !ctx ? (
                                gutter(left, <DiffLineNo n={left.old} />)
                              ) : left ? (
                                <span className="block px-2 font-mono text-code text-ink-muted">
                                  <DiffLineNo n={left.old} />
                                </span>
                              ) : null}
                            </td>
                            <DiffCode
                              kind={left?.kind ?? null}
                              text={left?.text ?? ''}
                              className="w-1/2 border-r border-line"
                            />
                            <td
                              className={cx(
                                'w-px p-0 align-top whitespace-nowrap',
                                right ? diffGroundClass[right.kind] : 'bg-surface-alt',
                              )}
                            >
                              {right ? gutter(right, <DiffLineNo n={right.new} />) : null}
                            </td>
                            <DiffCode kind={right?.kind ?? null} text={right?.text ?? ''} className="w-1/2" />
                          </tr>
                          {left && !ctx ? thread(left, 4) : null}
                          {right ? thread(right, 4) : null}
                        </Fragment>
                      )
                    })}
              </tbody>
            </table>
          </div>
        </>
      )}
    </section>
  )
}

function ResultNotFound({ missionId }: { missionId: string }) {
  return (
    <Page header={<AppHeader repoSelect nav={null} />}>
      <main className="mx-auto flex w-full max-w-[720px] flex-col gap-6 px-6 py-24">
        <Eyebrow>Code review</Eyebrow>
        <h1 className="m-0 text-title">This result is not here.</h1>
        <p className="m-0 text-ink-muted">The agents may still be working, or it was removed.</p>
        <Link
          to="/missions/$missionId/review"
          params={{ missionId }}
          className={buttonClass('primary', 'md', 'self-start')}
        >
          All results
        </Link>
      </main>
    </Page>
  )
}

/** Code review of one race result: file list, line-numbered diff, inline comments kept in the thread. */
export function CodeReview({ missionId, resultId }: { missionId: string; resultId: string }) {
  const s = useAstraState()
  const mission = s.missions[missionId]
  const found = s.results[resultId]
  const result = found?.missionId === missionId ? found : undefined
  useMissionPresence(missionId, `Reviewing result ${result?.label ?? ''}`.trim())
  if (!mission) return <MissionNotFound />
  if (!result) return <ResultNotFound missionId={missionId} />
  return <ReviewBody missionId={missionId} result={result} />
}

function ReviewBody({ missionId, result }: { missionId: string; result: RaceResult }) {
  const s = useAstraState()
  const verdict = useVerdict(missionId)
  const client = useClient()
  const mergeAction = useResultMerge(result)
  const mission = s.missions[missionId]
  const files = result.diff
  const label = result.label

  const [viewed, setViewed] = useViewed(result.id)
  const [collapsed, setCollapsed] = useState<Set<string>>(() => new Set(viewed))
  const [layout, setLayout] = useState<Layout>('unified')
  const [filesWidth, filesHandle] = useResizable({
    storageKey: 'astra.size.review.files',
    initial: 300,
    min: 200,
    max: 600,
    axis: 'x',
  })
  const [active, setActive] = useState(0)
  const [composer, setComposer] = useState<string | null>(null)
  const [requesting, setRequesting] = useState(false)
  const [notice, setNotice] = useState<string | null>(null)
  const sections = useRef<(HTMLElement | null)[]>([])

  const messages = messagesOf(s, missionId)
  const comments = lineCommentsOf(messages, label)
  const byLine = groupByLine(comments)
  const requestPrefix = formatChangeRequest(label, '').replace(/ — $/, '')
  const changesRequested = messages.some((m) => m.body.startsWith(requestPrefix))

  const jump = (i: number) => {
    setActive(i)
    sections.current[i]?.scrollIntoView?.({ block: 'start', behavior: 'smooth' })
  }

  // j / k move between files, unless the viewer is typing.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.metaKey || e.ctrlKey || e.altKey || e.defaultPrevented) return
      if (e.key !== 'j' && e.key !== 'k') return
      const t = e.target
      if (t instanceof Element && t.closest('input, textarea, select, [contenteditable="true"]')) return
      const next = Math.min(Math.max(active + (e.key === 'j' ? 1 : -1), 0), files.length - 1)
      if (next === active || next < 0) return
      e.preventDefault()
      setActive(next)
      sections.current[next]?.scrollIntoView?.({ block: 'start', behavior: 'smooth' })
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [active, files.length])

  if (!mission) return null
  const ma = s.missionAgents[result.missionAgentId]
  const agent = agentOf(s, ma)
  const people = members(s, missionId)
  const merged = mission.status === 'merged'
  const allowed = canApprove(s, s.me, missionId, result.missionAgentId)
  const mayComment = canSteer(s, s.me, missionId)
  const viewedCount = files.filter((f) => viewed.has(f.path)).length

  const setFileViewed = (path: string, on: boolean) => {
    const edit = (prev: Set<string>) => {
      const next = new Set(prev)
      if (on) next.add(path)
      else next.delete(path)
      return next
    }
    setViewed(edit)
    setCollapsed(edit)
  }
  const toggleCollapsed = (path: string) =>
    setCollapsed((prev) => {
      const next = new Set(prev)
      if (next.has(path)) next.delete(path)
      else next.add(path)
      return next
    })

  const postComment = async (anchor: LineAnchor, text: string) => {
    setNotice(null)
    const outcome = await sendMessage(client, missionId, null, formatLineComment(label, anchor, text))
    setComposer(null)
    if (outcome && !outcome.ok) setNotice(outcome.error ?? 'The comment was posted but not delivered.')
  }
  const requestChanges = async (text: string) => {
    setNotice(null)
    const outcome = await sendMessage(
      client,
      missionId,
      result.missionAgentId,
      formatChangeRequest(label, text),
    )
    setRequesting(false)
    if (outcome && !outcome.ok) setNotice(outcome.error ?? 'The request was posted but not delivered.')
  }

  return (
    <Page
      header={
        <AppHeader
          repoSelect
          nav={null}
          people={people.map((p) => p.person)}
          actions={<ShareButton missionId={missionId} />}
        />
      }
    >
      <MergeNotice action={mergeAction} />
      <section aria-label="Summary" className="flex flex-col gap-4 border-b border-line px-8 pt-6 pb-4">
        <div className="flex flex-wrap items-end justify-between gap-4">
          <div className="flex flex-col gap-2">
            <Link
              to="/missions/$missionId/review"
              params={{ missionId }}
              className={cx(inlineLinkClass, 'self-start')}
            >
              ← All results
            </Link>
            <Eyebrow>Mission {missionNo(mission)} · Code review</Eyebrow>
            <h1 className="m-0 text-title">Result {label}</h1>
            <p className="m-0 text-ink-muted">{mission.title}</p>
          </div>
          <div className="flex flex-wrap items-center gap-4">
            {merged ? <Status tone="success">Merged</Status> : null}
            {changesRequested && !merged ? <Status tone="needsYou">Changes requested</Status> : null}
            <Button
              variant="outline"
              size="sm"
              aria-expanded={requesting}
              onClick={() => setRequesting((v) => !v)}
              disabled={!mayComment || merged}
              title={mayComment ? undefined : 'Only people who can steer can request changes'}
            >
              Request changes
            </Button>
            <Button
              size="sm"
              onClick={mergeAction.doMerge}
              disabled={!allowed || merged || mergeAction.busy}
              title={allowed ? undefined : 'Only the owner can merge'}
            >
              Approve &amp; merge
            </Button>
          </div>
        </div>
        <div className="flex flex-wrap items-center gap-x-6 gap-y-2">
          <span>
            {ma ? roleName[ma.role] : 'Agent'} <Spec>{agent?.name}</Spec>
          </span>
          <Data className="text-ink-muted">{result.branch}</Data>
          <Data className="text-ink-muted">{formatDiffStats(result.stats)}</Data>
          {checksStatus(result)}
          {result.reviewerPick ? <Badge>Reviewer's pick</Badge> : null}
        </div>
        {verdict ? (
          <div className="flex gap-3 border border-line bg-surface-raised p-3">
            <AgentGlyph letter={verdict.ma ? roleLetter(verdict.ma.role) : 'R'} />
            <div className="flex min-w-0 flex-col gap-1">
              <div className="flex flex-wrap items-baseline gap-2">
                <b className="font-semibold">{verdict.ma ? roleName[verdict.ma.role] : 'Reviewer'}</b>
                <Spec>
                  {verdict.agent?.name} · {hhmm(verdict.message.at)}
                </Spec>
              </div>
              <Markdown className="max-w-[760px]">{verdict.message.body}</Markdown>
            </div>
          </div>
        ) : null}
        {requesting ? (
          <div className="max-w-[760px]">
            <Field label={`What should change in result ${label}?`}>
              {(id) => (
                <Composer
                  id={id}
                  submitLabel="Send request"
                  placeholder="Tell the agent what to fix before this can merge"
                  onSubmit={requestChanges}
                  onCancel={() => setRequesting(false)}
                />
              )}
            </Field>
          </div>
        ) : null}
        {notice ? <Status tone="danger">{notice}</Status> : null}
      </section>

      <div className="flex grow">
        <aside
          aria-label="Files"
          style={{ width: filesWidth }}
          className="sticky top-0 box-border flex shrink-0 flex-col gap-3 self-start border-r border-line bg-surface-alt px-3 py-6"
        >
          <ResizeHandle {...filesHandle} label="Resize file list" />
          <div className="flex items-baseline justify-between gap-2 px-2">
            <Eyebrow as="h2" className="m-0">
              Files · {files.length}
            </Eyebrow>
            <Spec>
              {viewedCount} of {files.length} viewed
            </Spec>
          </div>
          <ul className="m-0 flex list-none flex-col p-0">
            {files.map((f, i) => {
              const slash = f.path.lastIndexOf('/')
              const n = comments.filter((c) => c.comment.path === f.path).length
              return (
                <li
                  key={f.path}
                  className={cx('flex items-center gap-2 px-2 py-2', i === active && 'bg-signal-soft')}
                >
                  <input
                    type="checkbox"
                    aria-label={`Viewed ${f.path}`}
                    title="Viewed"
                    checked={viewed.has(f.path)}
                    onChange={(e) => setFileViewed(f.path, e.target.checked)}
                    className="m-0 shrink-0 accent-signal"
                  />
                  <button
                    type="button"
                    aria-current={i === active ? 'true' : undefined}
                    title={f.path}
                    onClick={() => jump(i)}
                    className="min-w-0 grow cursor-pointer truncate border-0 bg-transparent p-0 text-left text-ink"
                  >
                    {slash >= 0 ? <span className="text-ink-muted">{f.path.slice(0, slash + 1)}</span> : null}
                    {f.path.slice(slash + 1)}
                  </button>
                  {n > 0 ? <Spec>{n}</Spec> : null}
                  <Data className="shrink-0 text-ink-muted">{plusMinus(f)}</Data>
                </li>
              )
            })}
          </ul>
          {files.length > 1 ? <Spec className="px-2">J / K moves between files</Spec> : null}
        </aside>

        <main className="flex min-w-0 grow flex-col gap-4 px-8 pt-6 pb-8">
          <div className="flex flex-wrap items-center justify-between gap-4">
            <div className="flex items-baseline gap-3">
              <Eyebrow as="h2" className="m-0">
                Changes
              </Eyebrow>
              <Spec>{comments.length === 1 ? '1 comment' : `${comments.length} comments`}</Spec>
            </div>
            <fieldset className="m-0 flex rounded-sm border border-line-strong p-0">
              <legend className="sr-only">Diff layout</legend>
              {(['unified', 'split'] as const).map((v) => (
                <button
                  key={v}
                  type="button"
                  aria-pressed={layout === v}
                  onClick={() => setLayout(v)}
                  className={cx(
                    'h-8 cursor-pointer border-0 px-3 text-label uppercase',
                    layout === v
                      ? 'bg-invert text-on-invert'
                      : 'bg-transparent text-ink-muted hover:text-ink',
                  )}
                >
                  {v}
                </button>
              ))}
            </fieldset>
          </div>
          {files.length === 0 ? <p className="m-0 text-ink-muted">This result changes no files.</p> : null}
          {files.map((f, i) => (
            <FileDiff
              key={f.path}
              file={f}
              layout={layout}
              collapsed={collapsed.has(f.path)}
              viewed={viewed.has(f.path)}
              active={i === active}
              comments={byLine}
              composer={composer}
              mayComment={mayComment}
              onToggle={() => toggleCollapsed(f.path)}
              onOpen={setComposer}
              onCancel={() => setComposer(null)}
              onSubmit={postComment}
              sectionRef={(el) => {
                sections.current[i] = el
              }}
            />
          ))}
        </main>
      </div>
    </Page>
  )
}
