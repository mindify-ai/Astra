import { cx, Spec } from '@astra/ui'
import { useNavigate } from '@tanstack/react-router'
import { type KeyboardEvent, useEffect, useId, useMemo, useRef, useState } from 'react'
import { useAstraState, useHost, usePlatform, useUi } from '../context'
import { features } from '../features'
import { currentMissionId, missionNo, missionRoute, missionStatus, orderedMissions } from '../select'

interface Item {
  id: string
  group: 'Go to' | 'Missions' | 'Needs you' | 'Repositories' | 'Actions'
  label: string
  hint?: string
  keywords?: string
  run: () => void
}

/** ⌘K on macOS, Ctrl+K elsewhere. */
export const paletteShortcut = (mac: boolean) => (mac ? '⌘K' : 'CTRL K')

/** Header button that opens the palette (and shows its shortcut). */
export function PaletteButton() {
  const ui = useUi()
  const { platform } = usePlatform()
  return (
    <button
      type="button"
      aria-label="Command palette"
      aria-haspopup="dialog"
      title="Go to anything, run an action"
      onClick={() => ui.setPaletteOpen(true)}
      className="h-8 rounded-sm border border-line px-2 font-mono text-spec uppercase text-ink-muted hover:border-line-strong hover:text-ink"
    >
      {paletteShortcut(platform === 'mac' || platform === 'ios')}
    </button>
  )
}

/** Every term of the query must appear; label prefixes rank first. Lower is better, null is no match. */
function score(item: Item, terms: string[]): number | null {
  if (terms.length === 0) return 0
  const label = item.label.toLowerCase()
  const hay = `${label} ${item.group} ${item.hint ?? ''} ${item.keywords ?? ''}`.toLowerCase()
  if (!terms.every((t) => hay.includes(t))) return null
  const first = terms[0] ?? ''
  return label.startsWith(first) ? 0 : label.includes(first) ? 1 : 2
}

/** Quick navigation and actions, opened with ⌘K / Ctrl+K from anywhere. */
export function CommandPalette() {
  const ui = useUi()
  const host = useHost()
  const s = useAstraState()
  const { platform } = usePlatform()
  const navigate = useNavigate()
  const ref = useRef<HTMLDialogElement>(null)
  const input = useRef<HTMLInputElement>(null)
  const listId = useId()
  const [query, setQuery] = useState('')
  const [index, setIndex] = useState(0)
  const { paletteOpen: open, setPaletteOpen } = ui
  const mac = platform === 'mac' || platform === 'ios'

  useEffect(() => {
    const onKey = (e: globalThis.KeyboardEvent) => {
      if (e.key.toLowerCase() !== 'k' || e.altKey || e.shiftKey) return
      const mod = mac ? e.metaKey : e.ctrlKey
      if (!mod) return
      // Ctrl+K is "kill line" in a shell: leave it to the terminal when it has focus.
      if (!mac && (e.target as HTMLElement | null)?.closest?.('.xterm')) return
      e.preventDefault()
      setPaletteOpen(!open)
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [mac, open, setPaletteOpen])

  useEffect(() => {
    const d = ref.current
    if (!d) return
    if (open && !d.open) {
      setQuery('')
      setIndex(0)
      d.showModal?.()
      input.current?.focus()
    }
    if (!open && d.open) d.close?.()
  }, [open])

  const close = () => setPaletteOpen(false)

  const items = useMemo<Item[]>(() => {
    const go = (to: string, params?: Record<string, string>, search?: Record<string, string>) => () =>
      void navigate({ to, params, search } as Parameters<typeof navigate>[0])
    const out: Item[] = []
    const current = currentMissionId(s, ui.lastMissionId)
    const cm = current ? s.missions[current] : undefined
    const goal = query.trim()
    if (goal)
      out.push({
        id: 'start',
        group: 'Actions',
        label: `Start a mission: ${goal}`,
        keywords: 'new mission goal',
        run: go('/missions/new', undefined, { goal }),
      })
    out.push(
      { id: 'home', group: 'Go to', label: 'Home', keywords: 'start', run: go('/') },
      {
        id: 'new',
        group: 'Go to',
        label: 'New mission',
        keywords: 'launch create',
        run: go('/missions/new'),
      },
      { id: 'board', group: 'Go to', label: 'Board', keywords: 'tasks kanban', run: go('/board') },
      {
        id: 'agents',
        group: 'Go to',
        label: 'Connect agents',
        keywords: 'claude codex gemini setup',
        run: go('/setup/agents'),
      },
      { id: 'repos', group: 'Go to', label: 'Repositories', keywords: 'folder repo', run: go('/setup/repo') },
      {
        id: 'code',
        group: 'Go to',
        label: 'Open code editor',
        keywords: 'files source edit explorer',
        run: go('/code'),
      },
    )
    if (cm) {
      const p = { missionId: cm.id }
      out.push(
        { id: 'thread', group: 'Go to', label: `Thread · ${cm.title}`, run: go('/missions/$missionId', p) },
        { id: 'map', group: 'Go to', label: `Map · ${cm.title}`, run: go('/missions/$missionId/map', p) },
      )
    }
    for (const a of Object.values(s.approvals)) {
      if (a.state !== 'pending') continue
      const m = s.missions[a.missionId]
      out.push({
        id: `ap-${a.id}`,
        group: 'Needs you',
        label: a.prompt,
        hint: m ? missionNo(m) : undefined,
        keywords: 'approve deny approval',
        run: go('/missions/$missionId', { missionId: a.missionId }),
      })
    }
    for (const m of orderedMissions(s)) {
      out.push({
        id: `m-${m.id}`,
        group: 'Missions',
        label: m.title,
        hint: `${missionNo(m)} · ${missionStatus[m.status].label}`,
        keywords: m.goal,
        run: go(missionRoute(s, m), { missionId: m.id }),
      })
    }
    for (const r of Object.values(s.repositories)) {
      out.push({
        id: `r-${r.id}`,
        group: 'Repositories',
        label: `Switch to ${r.name}`,
        hint: r.id === ui.repoId ? 'CURRENT' : r.source.toUpperCase(),
        keywords: r.path ?? undefined,
        run: () => ui.setRepoId(r.id),
      })
    }
    if (host.terminal)
      out.push({
        id: 'terminal',
        group: 'Actions',
        label: ui.terminalOpen ? 'Hide terminal' : 'Open terminal',
        hint: 'CTRL `',
        keywords: 'shell console',
        run: () => ui.setTerminalOpen(!ui.terminalOpen),
      })
    if (cm && features.share)
      out.push({
        id: 'share',
        group: 'Actions',
        label: `Share ${missionNo(cm)}`,
        keywords: 'people invite members',
        run: () => ui.openShare(cm.id),
      })
    if (host.account && !host.account.person)
      out.push({
        id: 'signin',
        group: 'Actions',
        label: 'Sign in to sync',
        keywords: 'github account relay server team',
        run: () => void navigate({ to: '/signin', search: { provider: 'github' } }),
      })
    if (host.signOut) out.push({ id: 'signout', group: 'Actions', label: 'Sign out', run: host.signOut })
    return out
  }, [s, ui, host, query, navigate])

  const results = useMemo(() => {
    const terms = query.toLowerCase().split(/\s+/).filter(Boolean)
    const scored = items
      .map((item, i) => ({ item, i, sc: item.id === 'start' ? 3 : score(item, terms) }))
      .filter((x): x is { item: Item; i: number; sc: number } => x.sc !== null)
    if (terms.length > 0) scored.sort((a, b) => a.sc - b.sc || a.i - b.i)
    return scored.map((x) => x.item).slice(0, 50)
  }, [items, query])

  const active = Math.min(index, Math.max(results.length - 1, 0))
  const run = (item: Item | undefined) => {
    if (!item) return
    close()
    item.run()
  }
  const onKeyDown = (e: KeyboardEvent<HTMLInputElement>) => {
    if (e.key === 'ArrowDown') {
      e.preventDefault()
      setIndex((active + 1) % Math.max(results.length, 1))
    } else if (e.key === 'ArrowUp') {
      e.preventDefault()
      setIndex((active - 1 + results.length) % Math.max(results.length, 1))
    } else if (e.key === 'Enter') {
      e.preventDefault()
      run(results[active])
    }
  }

  useEffect(() => {
    document.getElementById(`${listId}-${active}`)?.scrollIntoView?.({ block: 'nearest' })
  }, [active, listId])

  let lastGroup = ''
  return (
    <dialog
      ref={ref}
      aria-label="Command palette"
      onClose={close}
      onClick={(e) => {
        if (e.target === ref.current) close()
      }}
      onKeyDown={(e) => {
        if (e.key === 'Escape') close()
      }}
      className="mx-auto mt-[12vh] mb-auto w-[640px] max-w-[calc(100vw-32px)] border-0 bg-transparent p-0 text-ink backdrop:bg-scrim"
    >
      {open ? (
        <div className="flex max-h-[min(560px,70vh)] flex-col rounded-sm border border-line-strong bg-surface-overlay shadow-lg">
          <input
            ref={input}
            role="combobox"
            aria-expanded="true"
            aria-controls={listId}
            aria-activedescendant={results.length ? `${listId}-${active}` : undefined}
            aria-label="Search missions, screens and actions"
            placeholder="Go to a mission, screen or action. Type a goal to start a mission."
            value={query}
            onChange={(e) => {
              setQuery(e.target.value)
              setIndex(0)
            }}
            onKeyDown={onKeyDown}
            className="h-12 shrink-0 border-0 border-b border-line bg-transparent px-4 text-body text-ink outline-none placeholder:text-ink-muted"
          />
          <div id={listId} role="listbox" aria-label="Results" className="min-h-0 overflow-y-auto py-1">
            {results.length === 0 ? (
              <p className="m-0 px-4 py-3 text-ink-muted">Nothing matches.</p>
            ) : (
              results.map((item, i) => {
                const header = item.group !== lastGroup ? item.group : null
                lastGroup = item.group
                return (
                  <div key={item.id}>
                    {header ? <Spec className="block px-4 pt-3 pb-1">{header}</Spec> : null}
                    <div
                      id={`${listId}-${i}`}
                      role="option"
                      aria-selected={i === active}
                      tabIndex={-1}
                      onMouseMove={() => i !== active && setIndex(i)}
                      onClick={() => run(item)}
                      onKeyDown={() => {}}
                      className={cx(
                        'mx-1 flex cursor-pointer items-center gap-3 rounded-sm px-3 py-2',
                        i === active ? 'bg-signal-soft text-ink' : 'text-ink',
                      )}
                    >
                      <span className="min-w-0 grow truncate">{item.label}</span>
                      {item.hint ? <Spec className="shrink-0">{item.hint}</Spec> : null}
                    </div>
                  </div>
                )
              })
            )}
          </div>
          <Spec className="flex shrink-0 gap-4 border-t border-line px-4 py-2">
            <span>↑↓ MOVE</span>
            <span>↵ OPEN</span>
            <span>ESC CLOSE</span>
          </Spec>
        </div>
      ) : null}
    </dialog>
  )
}
