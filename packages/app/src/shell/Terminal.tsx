import type { TerminalHost, TerminalSession } from '@astra/client'
import { buttonClass, CloseIcon, cx, Eyebrow, ResizeHandle, useResizable } from '@astra/ui'
import '@xterm/xterm/css/xterm.css'
import { type ReactNode, useEffect, useRef, useState } from 'react'
import { useAstraState, useHost, usePlatform, useUi } from '../context'

/** The terminal toggle for the header (desktop app only). */
export function TerminalButton() {
  const host = useHost()
  const ui = useUi()
  if (!host.terminal) return null
  return (
    <button
      type="button"
      className={buttonClass('text', 'sm')}
      aria-pressed={ui.terminalOpen}
      title="Terminal (Ctrl+`)"
      onClick={() => ui.setTerminalOpen(!ui.terminalOpen)}
    >
      TERMINAL
    </button>
  )
}

/**
 * Lays the page out above a terminal dock when the host can run shells (the desktop app). The dock stays
 * mounted while hidden, so shells keep running between toggles.
 */
export function WithTerminal({ children }: { children: ReactNode }) {
  const host = useHost()
  const ui = useUi()
  const { layout } = usePlatform()
  const { terminalOpen, setTerminalOpen } = ui

  useEffect(() => {
    if (!host.terminal) return
    const onKey = (e: KeyboardEvent) => {
      if (e.ctrlKey && e.key === '`') {
        e.preventDefault()
        setTerminalOpen(!terminalOpen)
      }
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [host.terminal, terminalOpen, setTerminalOpen])

  if (!host.terminal || layout === 'mobile') return <>{children}</>
  const edge = ui.layout.terminal
  return (
    <div className={cx('flex h-dvh bg-surface', edge === 'right' ? 'flex-row' : 'flex-col')}>
      <div className="min-h-0 min-w-0 grow overflow-auto">{children}</div>
      <TerminalDock
        host={host.terminal}
        open={terminalOpen}
        edge={edge}
        onEdge={(e) => ui.setLayout((l) => ({ ...l, terminal: e }))}
        onHide={() => setTerminalOpen(false)}
      />
    </div>
  )
}

interface Tab {
  key: number
  cwd: string | null
}

/**
 * The dock may grow to 80% of the window (height at the bottom, width on the right); tracked so shrinking the
 * window shrinks the dock with it.
 */
const maxDock = () => ({
  height: Math.round((typeof window === 'undefined' ? 900 : window.innerHeight) * 0.8),
  width: Math.round((typeof window === 'undefined' ? 1440 : window.innerWidth) * 0.8),
})

function useMaxDock() {
  const [max, setMax] = useState(maxDock)
  useEffect(() => {
    const onResize = () => setMax(maxDock())
    window.addEventListener('resize', onResize)
    return () => window.removeEventListener('resize', onResize)
  }, [])
  return max
}

function TerminalDock({
  host,
  open,
  edge,
  onEdge,
  onHide,
}: {
  host: TerminalHost
  open: boolean
  edge: 'bottom' | 'right'
  onEdge: (edge: 'bottom' | 'right') => void
  onHide: () => void
}) {
  const s = useAstraState()
  const ui = useUi()
  const repo = s.repositories[ui.repoId]
  const cwd = repo?.path ?? null
  const [tabs, setTabs] = useState<Tab[]>([])
  const [active, setActive] = useState<number | null>(null)
  const next = useRef(1)
  const max = useMaxDock()
  // The xterm views refit themselves through their ResizeObserver as the size changes.
  const [height, resizeHeight] = useResizable({
    storageKey: 'astra.size.terminal',
    initial: 360,
    min: 120,
    max: Math.max(120, max.height),
    axis: 'y',
    invert: true,
  })
  const [width, resizeWidth] = useResizable({
    storageKey: 'astra.size.terminal.right',
    initial: 520,
    min: 280,
    max: Math.max(280, max.width),
    axis: 'x',
    invert: true,
  })
  const right = edge === 'right'

  const add = () => {
    const tab = { key: next.current++, cwd }
    setTabs((t) => [...t, tab])
    setActive(tab.key)
  }
  const remove = (key: number) => {
    setTabs((t) => {
      const rest = t.filter((x) => x.key !== key)
      if (active === key) setActive(rest[rest.length - 1]?.key ?? null)
      if (rest.length === 0) onHide()
      return rest
    })
  }

  // Opening the dock with no shell starts one.
  // biome-ignore lint/correctness/useExhaustiveDependencies: only when the dock opens
  useEffect(() => {
    if (open && tabs.length === 0) add()
  }, [open])

  return (
    <section
      aria-label="Terminal"
      hidden={!open}
      style={right ? { width } : { height }}
      className={cx(
        'relative flex shrink-0 flex-col bg-surface',
        right ? 'max-w-[80dvw] border-l border-line-strong' : 'max-h-[80dvh] border-t border-line-strong',
      )}
    >
      <ResizeHandle {...(right ? resizeWidth : resizeHeight)} label="Resize terminal" />
      <div className="flex h-9 shrink-0 items-center gap-1 border-b border-line px-3">
        <Eyebrow as="h2" className="mr-3">
          Terminal
        </Eyebrow>
        <div role="tablist" aria-label="Shells" className="flex min-w-0 items-center gap-1 overflow-x-auto">
          {tabs.map((t, i) => (
            <span
              key={t.key}
              className={cx(
                'flex items-center gap-1 rounded-sm pr-1 pl-2',
                t.key === active ? 'bg-surface-raised text-ink' : 'text-ink-muted',
              )}
            >
              <button
                type="button"
                role="tab"
                aria-selected={t.key === active}
                className="h-7 border-0 bg-transparent p-0 font-mono text-spec text-inherit"
                onClick={() => setActive(t.key)}
              >
                {i + 1} · {t.cwd ? t.cwd.split('/').pop() : '~'}
              </button>
              <button
                type="button"
                aria-label={`Close shell ${i + 1}`}
                className="flex size-5 items-center justify-center border-0 bg-transparent p-0 text-ink-muted hover:text-ink"
                onClick={() => remove(t.key)}
              >
                <CloseIcon size={12} />
              </button>
            </span>
          ))}
          <button
            type="button"
            aria-label="New shell"
            title={cwd ? `New shell in ${cwd}` : 'New shell'}
            className="h-7 border-0 bg-transparent px-2 text-ink-muted hover:text-ink"
            onClick={add}
          >
            +
          </button>
        </div>
        <div className="grow" />
        <button
          type="button"
          className={buttonClass('text', 'sm')}
          title={right ? 'Dock the terminal at the bottom' : 'Dock the terminal on the right'}
          onClick={() => onEdge(right ? 'bottom' : 'right')}
        >
          {right ? 'DOCK BOTTOM' : 'DOCK RIGHT'}
        </button>
        <button type="button" className={buttonClass('text', 'sm')} onClick={onHide}>
          HIDE
        </button>
      </div>
      <div className="relative min-h-0 grow">
        {tabs.map((t) => (
          <Shell
            key={t.key}
            host={host}
            cwd={t.cwd}
            visible={open && t.key === active}
            onExit={() => remove(t.key)}
          />
        ))}
      </div>
    </section>
  )
}

const cssVar = (name: string, fallback: string) =>
  getComputedStyle(document.documentElement).getPropertyValue(name).trim() || fallback

/** One xterm.js view bound to a shell session. */
function Shell({
  host,
  cwd,
  visible,
  onExit,
}: {
  host: TerminalHost
  cwd: string | null
  visible: boolean
  onExit: () => void
}) {
  const el = useRef<HTMLDivElement>(null)
  const view = useRef<{ fit: () => void; focus: () => void } | null>(null)
  const exit = useRef(onExit)
  exit.current = onExit

  // biome-ignore lint/correctness/useExhaustiveDependencies: one shell per mount; cwd is fixed at creation
  useEffect(() => {
    let disposed = false
    let session: TerminalSession | null = null
    let observer: ResizeObserver | null = null
    let dispose = () => {}
    void (async () => {
      const [{ Terminal }, { FitAddon }] = await Promise.all([
        import('@xterm/xterm'),
        import('@xterm/addon-fit'),
      ])
      if (disposed || !el.current) return
      const term = new Terminal({
        fontFamily: cssVar('--font-mono', 'ui-monospace, Menlo, monospace'),
        fontSize: 13,
        cursorBlink: true,
        scrollback: 5000,
        theme: {
          background: cssVar('--surface', '#000000'),
          foreground: cssVar('--ink', '#ffffff'),
          cursor: cssVar('--signal', '#8cd44a'),
          selectionBackground: cssVar('--selection', '#1e4d80'),
          selectionInactiveBackground: cssVar('--selection-inactive', '#2b3440'),
        },
      })
      const fit = new FitAddon()
      term.loadAddon(fit)
      term.open(el.current)
      const refit = () => {
        // A hidden dock has no size; fitting it would shrink the shell to nothing.
        if (el.current?.offsetParent) fit.fit()
      }
      refit()
      view.current = { fit: refit, focus: () => term.focus() }
      dispose = () => term.dispose()
      try {
        session = await host.open({
          cwd,
          cols: term.cols,
          rows: term.rows,
          onData: (d) => term.write(d),
          onExit: () => exit.current(),
        })
      } catch (err) {
        term.write(`Could not start a shell: ${String(err)}\r\n`)
        return
      }
      if (disposed) return session.close()
      const s = session
      term.onData((d) => s.write(d))
      term.onResize(({ cols, rows }) => s.resize(cols, rows))
      observer = new ResizeObserver(refit)
      observer.observe(el.current)
      term.focus()
    })().catch(() => {
      /* the view could not load (no canvas/fonts); the dock stays usable for other shells */
    })
    return () => {
      disposed = true
      observer?.disconnect()
      session?.close()
      dispose()
      view.current = null
    }
  }, [])

  useEffect(() => {
    if (!visible) return
    view.current?.fit()
    view.current?.focus()
  }, [visible])

  return <div ref={el} hidden={!visible} className="absolute inset-0 box-border px-3 pt-2" />
}
