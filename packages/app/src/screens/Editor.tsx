import { type FileEntry, type FilesHost, memoryFiles, type State } from '@astra/client'
import {
  buttonClass,
  ChevronLeftIcon,
  CloseIcon,
  cx,
  Data,
  Eyebrow,
  FileIcon,
  ResizeHandle,
  Spec,
  SpecStatus,
  useResizable,
} from '@astra/ui'
import { openSearchPanel } from '@codemirror/search'
import type { EditorState } from '@codemirror/state'
import type { EditorView } from '@codemirror/view'
import { Link } from '@tanstack/react-router'
import { type KeyboardEvent, type ReactNode, useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { useAstraState, useHost, useUi } from '../context'
import { CodeEditor, type Cursor, languageFor } from '../shared/codeEditor'
import { AppHeader, Page } from '../shell/AppHeader'

const message = (err: unknown) => (err instanceof Error ? err.message : String(err))
const baseName = (path: string) => path.split('/').pop() ?? path
const parentOf = (path: string) => path.split('/').slice(0, -1).join('/')

/**
 * The web demo's files: each file a mission on this repository changed, as its race result left it (context
 * and added lines). The reviewer's pick wins when two results touch the same file. Null when there are none.
 */
export function demoFiles(
  s: Pick<State, 'missions' | 'results'>,
  repoId: string,
): Record<string, string> | null {
  const missions = new Set(
    Object.values(s.missions)
      .filter((m) => m.repoId === repoId)
      .map((m) => m.id),
  )
  const results = Object.values(s.results)
    .filter((r) => missions.has(r.missionId))
    .sort((a, b) => Number(a.reviewerPick) - Number(b.reviewerPick))
  const out: Record<string, string> = {}
  for (const r of results) {
    for (const f of r.diff) {
      out[f.path] = f.lines
        .filter((l) => l.kind !== 'del')
        .map((l) => l.text)
        .join('\n')
    }
  }
  return Object.keys(out).length > 0 ? out : null
}

function safeMemoryFiles(initial: Record<string, string>): FilesHost | null {
  try {
    return memoryFiles(initial)
  } catch {
    return null
  }
}

/** The code editor for the current repository: explorer, open-file tabs, editor and status bar. */
export function Editor() {
  const s = useAstraState()
  const ui = useUi()
  const host = useHost()
  const repo = s.repositories[ui.repoId]
  const { missions, results } = s
  const demo = useMemo(() => {
    if (host.files || !repo) return null
    const initial = demoFiles({ missions, results }, repo.id)
    return initial ? safeMemoryFiles(initial) : null
  }, [host.files, repo, missions, results])

  let body: ReactNode
  if (!repo) {
    body = (
      <Empty title="No repository yet." action>
        Add a repository to browse and edit its files.
      </Empty>
    )
  } else if (host.files && repo.path) {
    body = <Workspace key={repo.id} files={host.files} root={repo.path} repoName={repo.name} demo={false} />
  } else if (host.files) {
    body = (
      <Empty title="This repository has no local folder." action>
        The editor opens files on this machine. Add {repo.name} as a local folder to edit it here.
      </Empty>
    )
  } else if (demo) {
    body = <Workspace key={repo.id} files={demo} root={repo.path ?? repo.name} repoName={repo.name} demo />
  } else {
    body = (
      <Empty title="Editing files needs the desktop app.">
        Open {repo.name} in Astra for desktop to browse and edit its files. Files that missions change show up
        here too.
      </Empty>
    )
  }

  return (
    <Page className="h-dvh" header={<AppHeader repoSelect nav={null} />}>
      <h1 className="sr-only">Code editor</h1>
      {body}
    </Page>
  )
}

function Empty({ title, action, children }: { title: string; action?: boolean; children: ReactNode }) {
  return (
    <main className="mx-auto flex w-full max-w-[720px] flex-col gap-6 px-6 py-24">
      <Eyebrow>Code</Eyebrow>
      <h2 className="m-0 text-title">{title}</h2>
      <p className="m-0 text-ink-muted">{children}</p>
      {action ? (
        <Link
          to="/setup/repo"
          search={{ source: 'local' }}
          className={buttonClass('primary', 'md', 'self-start')}
        >
          ADD LOCAL FOLDER
        </Link>
      ) : null}
    </main>
  )
}

// ── Workspace ──────────────────────────────────────────────────────────────

type Dir =
  | { status: 'loading' }
  | { status: 'ready'; entries: FileEntry[] }
  | { status: 'error'; error: string }

interface Doc {
  path: string
  status: 'loading' | 'ready' | 'error'
  text: string
  /** The text as last read or saved; the tab is dirty while `text` differs. */
  saved: string
  error: string | null
  saving: boolean
  saveError: string | null
}

const isDirty = (d: Doc) => d.status === 'ready' && d.text !== d.saved

function Workspace({
  files,
  root,
  repoName,
  demo,
}: {
  files: FilesHost
  root: string
  repoName: string
  demo: boolean
}) {
  const [dirs, setDirs] = useState<Record<string, Dir>>({})
  const [expanded, setExpanded] = useState<ReadonlySet<string>>(() => new Set())
  const [docs, setDocs] = useState<Doc[]>([])
  const [active, setActive] = useState<string | null>(null)
  const [armedClose, setArmedClose] = useState<string | null>(null)
  const [cursor, setCursor] = useState<Cursor>({ line: 1, col: 1 })
  /** Each open file's editor state (undo history, selection, folds), kept while another tab shows. */
  const states = useRef(new Map<string, EditorState>()).current
  const view = useRef<EditorView | null>(null)
  const [explorerWidth, explorerHandle] = useResizable({
    storageKey: 'astra.size.editor.explorer',
    initial: 260,
    min: 180,
    max: 560,
    axis: 'x',
  })
  const requested = useRef(new Set<string>())
  const docsRef = useRef(docs)
  docsRef.current = docs

  const loadDir = useCallback(
    (dir: string) => {
      if (requested.current.has(dir)) return
      requested.current.add(dir)
      setDirs((d) => ({ ...d, [dir]: { status: 'loading' } }))
      files.list(root, dir).then(
        (entries) => setDirs((d) => ({ ...d, [dir]: { status: 'ready', entries } })),
        (err: unknown) => {
          requested.current.delete(dir)
          setDirs((d) => ({ ...d, [dir]: { status: 'error', error: message(err) } }))
        },
      )
    },
    [files, root],
  )

  useEffect(() => loadDir(''), [loadDir])

  const toggleDir = (path: string) => {
    const open = !expanded.has(path)
    setExpanded((prev) => {
      const next = new Set(prev)
      if (open) next.add(path)
      else next.delete(path)
      return next
    })
    if (open) loadDir(path)
  }

  const patchDoc = useCallback(
    (path: string, patch: Partial<Doc>) =>
      setDocs((ds) => ds.map((d) => (d.path === path ? { ...d, ...patch } : d))),
    [],
  )

  const openFile = (path: string) => {
    setActive(path)
    setCursor({ line: 1, col: 1 })
    if (docsRef.current.some((d) => d.path === path)) return
    setDocs((ds) => [
      ...ds,
      { path, status: 'loading', text: '', saved: '', error: null, saving: false, saveError: null },
    ])
    files.read(root, path).then(
      (text) => patchDoc(path, { status: 'ready', text, saved: text }),
      (err: unknown) => patchDoc(path, { status: 'error', error: message(err) }),
    )
  }

  const closeDoc = (path: string) => {
    const doc = docsRef.current.find((d) => d.path === path)
    if (doc && isDirty(doc) && armedClose !== path) {
      setArmedClose(path)
      return
    }
    setArmedClose(null)
    states.delete(path)
    const rest = docsRef.current.filter((d) => d.path !== path)
    const at = docsRef.current.findIndex((d) => d.path === path)
    setDocs(rest)
    if (active === path) setActive(rest[Math.min(at, rest.length - 1)]?.path ?? null)
  }

  const save = useCallback(
    async (path: string) => {
      const doc = docsRef.current.find((d) => d.path === path)
      if (doc?.status !== 'ready' || doc.saving) return
      const text = doc.text
      patchDoc(path, { saving: true, saveError: null })
      try {
        await files.write(root, path, text)
        patchDoc(path, { saving: false, saved: text })
      } catch (err) {
        patchDoc(path, { saving: false, saveError: message(err) })
      }
    },
    [files, root, patchDoc],
  )

  const doc = docs.find((d) => d.path === active) ?? null

  // ⌘S / Ctrl+S saves the open file. ⌘F / Ctrl+F finds in it: the editor handles that itself when it has
  // focus; from anywhere else on the screen it opens the editor's search.
  useEffect(() => {
    const onKey = (e: globalThis.KeyboardEvent) => {
      if (!(e.metaKey || e.ctrlKey) || e.altKey || e.shiftKey) return
      const key = e.key.toLowerCase()
      if (key === 's') {
        e.preventDefault()
        if (active) void save(active)
      } else if (key === 'f' && !e.defaultPrevented && view.current) {
        e.preventDefault()
        openSearchPanel(view.current)
        view.current.dom.querySelector<HTMLInputElement>('[main-field]')?.focus()
      }
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [active, save])

  const rootDir = dirs['']
  return (
    <main className="flex min-h-0 grow">
      <aside
        aria-label="Explorer"
        style={{ width: explorerWidth }}
        className="relative box-border flex shrink-0 flex-col border-r border-line bg-surface-alt"
      >
        <ResizeHandle {...explorerHandle} label="Resize explorer" />
        <div className="flex h-9 shrink-0 items-center gap-3 border-b border-line px-4">
          <Eyebrow as="h2">Explorer</Eyebrow>
          <Spec className="truncate" title={root}>
            {repoName}
          </Spec>
        </div>
        <div className="min-h-0 grow overflow-auto py-2">
          {!rootDir || rootDir.status === 'loading' ? (
            <p className="m-0 px-4 py-2 text-ink-muted">Loading files…</p>
          ) : rootDir.status === 'error' ? (
            <p role="alert" className="m-0 px-4 py-2 text-danger">
              {rootDir.error}
            </p>
          ) : rootDir.entries.length === 0 ? (
            <p className="m-0 px-4 py-2 text-ink-muted">This folder is empty.</p>
          ) : (
            <Tree
              dirs={dirs}
              expanded={expanded}
              active={active}
              onToggle={toggleDir}
              onOpen={openFile}
              onCollapse={(path) => expanded.has(path) && toggleDir(path)}
            />
          )}
        </div>
      </aside>
      <section aria-label="Editor" className="flex min-w-0 grow flex-col">
        {demo ? (
          <div className="flex shrink-0 flex-wrap items-center gap-x-4 gap-y-1 border-b border-line bg-surface-alt px-4 py-2">
            <SpecStatus tone="needsYou">DEMO FILES · CHANGES STAY IN THIS TAB</SpecStatus>
            <span className="text-body-sm text-ink-muted">
              Built from this repository's mission results. Nothing is written to disk; reloading discards
              edits.
            </span>
          </div>
        ) : null}
        <div
          role="tablist"
          aria-label="Open files"
          className="flex h-9 shrink-0 items-stretch overflow-x-auto border-b border-line bg-surface-alt"
        >
          {docs.map((d) => {
            const selected = d.path === active
            const dirty = isDirty(d)
            const armed = armedClose === d.path
            return (
              <span
                key={d.path}
                className={cx(
                  'flex shrink-0 items-center gap-1 border-r border-line pr-1 pl-3',
                  selected ? 'bg-surface text-ink' : 'text-ink-muted',
                )}
              >
                <button
                  type="button"
                  role="tab"
                  aria-selected={selected}
                  aria-controls="code-editor-pane"
                  title={d.path}
                  onClick={() => {
                    setArmedClose(null)
                    setActive(d.path)
                  }}
                  className="flex h-full items-center gap-2 border-0 bg-transparent p-0 font-mono text-data text-inherit"
                >
                  <FileIcon name={baseName(d.path)} size={14} />
                  {baseName(d.path)}
                  {dirty ? (
                    <span aria-hidden="true" className="inline-block size-1.5 rounded-full bg-ink" />
                  ) : null}
                  {dirty ? <span className="sr-only">, unsaved</span> : null}
                </button>
                {armed ? (
                  <button
                    type="button"
                    aria-label={`Discard changes to ${baseName(d.path)}`}
                    onClick={() => closeDoc(d.path)}
                    onBlur={() => setArmedClose(null)}
                    className={buttonClass('text', 'sm', 'h-7 px-2 text-danger')}
                  >
                    DISCARD
                  </button>
                ) : (
                  <button
                    type="button"
                    aria-label={`Close ${baseName(d.path)}`}
                    onClick={() => closeDoc(d.path)}
                    className="flex size-6 items-center justify-center rounded-sm border-0 bg-transparent p-0 text-ink-muted hover:bg-surface-raised hover:text-ink"
                  >
                    <CloseIcon size={14} />
                  </button>
                )}
              </span>
            )
          })}
        </div>
        <div id="code-editor-pane" role="tabpanel" className="flex min-h-0 grow flex-col">
          {!doc ? (
            <div className="flex grow flex-col items-center justify-center gap-2 bg-surface-sunken p-6">
              <p className="m-0 text-ink-muted">Open a file from the explorer.</p>
              <Spec>⌘S / CTRL S SAVES · ⌘F / CTRL F FINDS</Spec>
            </div>
          ) : doc.status === 'loading' ? (
            <p className="m-0 grow bg-surface-sunken p-6 text-ink-muted">Opening {doc.path}…</p>
          ) : doc.status === 'error' ? (
            <div className="grow bg-surface-sunken p-6">
              <p role="alert" className="m-0 text-danger">
                {doc.path} can't be opened: {doc.error}
              </p>
            </div>
          ) : (
            <CodeEditor
              key={doc.path}
              path={doc.path}
              text={doc.text}
              states={states}
              viewRef={view}
              onChange={(text) => {
                setArmedClose(null)
                patchDoc(doc.path, { text })
              }}
              onCursor={setCursor}
            />
          )}
        </div>
        <StatusBar doc={doc} cursor={cursor} />
      </section>
    </main>
  )
}

// ── Explorer tree ──────────────────────────────────────────────────────────

type Row =
  | { kind: 'entry'; entry: FileEntry; level: number }
  | { kind: 'note'; id: string; level: number; text: string; error?: boolean }

function visibleRows(dirs: Record<string, Dir>, expanded: ReadonlySet<string>): Row[] {
  const rows: Row[] = []
  const walk = (dir: string, level: number) => {
    const d = dirs[dir]
    if (!d || d.status === 'loading') {
      rows.push({ kind: 'note', id: `${dir}:loading`, level, text: 'Loading…' })
      return
    }
    if (d.status === 'error') {
      rows.push({ kind: 'note', id: `${dir}:error`, level, text: d.error, error: true })
      return
    }
    if (d.entries.length === 0) rows.push({ kind: 'note', id: `${dir}:empty`, level, text: 'Empty folder' })
    for (const entry of d.entries) {
      rows.push({ kind: 'entry', entry, level })
      if (entry.kind === 'dir' && expanded.has(entry.path)) walk(entry.path, level + 1)
    }
  }
  walk('', 1)
  return rows
}

function Tree({
  dirs,
  expanded,
  active,
  onToggle,
  onOpen,
  onCollapse,
}: {
  dirs: Record<string, Dir>
  expanded: ReadonlySet<string>
  active: string | null
  onToggle: (path: string) => void
  onOpen: (path: string) => void
  onCollapse: (path: string) => void
}) {
  const rows = visibleRows(dirs, expanded)
  const entries = rows.flatMap((r) => (r.kind === 'entry' ? [r] : []))
  const [focused, setFocused] = useState<string | null>(null)
  const refs = useRef(new Map<string, HTMLDivElement>())
  const keyboard = useRef(false)
  const tabStop = entries.some((r) => r.entry.path === focused) ? focused : (entries[0]?.entry.path ?? null)

  useEffect(() => {
    if (keyboard.current && tabStop) refs.current.get(tabStop)?.focus()
    keyboard.current = false
  }, [tabStop])

  const activate = (entry: FileEntry) => (entry.kind === 'dir' ? onToggle(entry.path) : onOpen(entry.path))

  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    const at = entries.findIndex((r) => r.entry.path === tabStop)
    const row = entries[at]
    if (!row) return
    const { entry } = row
    const move = (i: number) => {
      const next = entries[Math.max(0, Math.min(entries.length - 1, i))]
      if (next) {
        keyboard.current = true
        setFocused(next.entry.path)
      }
    }
    switch (e.key) {
      case 'ArrowDown':
        move(at + 1)
        break
      case 'ArrowUp':
        move(at - 1)
        break
      case 'Home':
        move(0)
        break
      case 'End':
        move(entries.length - 1)
        break
      case 'ArrowRight':
        if (entry.kind !== 'dir') return
        if (!expanded.has(entry.path)) onToggle(entry.path)
        else if (entries[at + 1]?.level === row.level + 1) move(at + 1)
        break
      case 'ArrowLeft':
        if (entry.kind === 'dir' && expanded.has(entry.path)) onCollapse(entry.path)
        else if (row.level > 1) move(entries.findIndex((r) => r.entry.path === parentOf(entry.path)))
        break
      case 'Enter':
      case ' ':
        activate(entry)
        break
      default:
        return
    }
    e.preventDefault()
  }

  return (
    <div role="tree" aria-label="Files" onKeyDown={onKeyDown}>
      {rows.map((r) => {
        const indent = { paddingLeft: `${8 + (r.level - 1) * 14}px` }
        if (r.kind === 'note') {
          return (
            <div
              key={r.id}
              role="treeitem"
              aria-level={r.level}
              aria-disabled="true"
              aria-selected={false}
              tabIndex={-1}
              style={indent}
              className={cx(
                'flex h-7 items-center pr-3 text-body-sm',
                r.error ? 'text-danger' : 'text-ink-muted',
              )}
            >
              <span className="w-5 shrink-0" />
              <span className="truncate">{r.text}</span>
            </div>
          )
        }
        const { entry } = r
        const dir = entry.kind === 'dir'
        const open = dir && expanded.has(entry.path)
        return (
          <div
            key={entry.path}
            ref={(el) => {
              if (el) refs.current.set(entry.path, el)
              else refs.current.delete(entry.path)
            }}
            role="treeitem"
            aria-level={r.level}
            aria-expanded={dir ? open : undefined}
            aria-selected={entry.path === active}
            tabIndex={entry.path === tabStop ? 0 : -1}
            title={entry.path}
            style={indent}
            onClick={() => {
              setFocused(entry.path)
              activate(entry)
            }}
            onKeyDown={() => {}}
            className={cx(
              'flex h-7 cursor-pointer select-none items-center pr-3 text-body-sm',
              entry.path === active ? 'bg-signal-soft text-ink' : 'text-ink hover:bg-surface-raised',
            )}
          >
            <span className="flex w-4 shrink-0 items-center text-ink-muted">
              {dir ? <ChevronLeftIcon size={12} className={open ? '-rotate-90' : 'rotate-180'} /> : null}
            </span>
            <FileIcon name={entry.name} folder={dir} open={open} className="mr-1.5" />
            <span className={cx('truncate', dir ? 'text-ink' : 'font-mono text-data')}>{entry.name}</span>
          </div>
        )
      })}
    </div>
  )
}

// ── Status bar ─────────────────────────────────────────────────────────────

function StatusBar({ doc, cursor }: { doc: Doc | null; cursor: Cursor }) {
  let status: ReactNode = null
  if (doc?.status === 'ready') {
    status = doc.saving ? (
      <SpecStatus tone="working">SAVING</SpecStatus>
    ) : doc.saveError ? (
      <span role="alert" className="flex min-w-0 items-center gap-3">
        <SpecStatus tone="danger">SAVE FAILED</SpecStatus>
        <span className="truncate text-body-sm text-ink-muted" title={doc.saveError}>
          {doc.saveError}
        </span>
      </span>
    ) : isDirty(doc) ? (
      <SpecStatus tone="needsYou">UNSAVED</SpecStatus>
    ) : (
      <SpecStatus tone="success">SAVED</SpecStatus>
    )
  }
  return (
    <footer className="flex h-8 shrink-0 items-center gap-4 border-t border-line bg-surface-alt px-4">
      {doc ? (
        <>
          <Data className="min-w-0 truncate text-ink-muted" title={doc.path}>
            {doc.path}
          </Data>
          <span className="grow" />
          <Spec>{languageFor(doc.path).label}</Spec>
          {doc.status === 'ready' ? (
            <Spec>
              LN {cursor.line}, COL {cursor.col}
            </Spec>
          ) : null}
          {status}
        </>
      ) : (
        <Spec>NO FILE OPEN</Spec>
      )}
    </footer>
  )
}
