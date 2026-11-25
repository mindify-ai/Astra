import { buttonClass, cx, ResizeHandle, Spec, useResizable } from '@astra/ui'
import { type ReactNode, useEffect, useRef, useState } from 'react'
import { defaultLayout, type Side, useHost, useUi, type WorkbenchLayout } from '../context'
import { BrowserPane } from '../shared/browser'
import { MissionChatPane } from '../shared/missionChat'

type PaneId = 'chat' | 'browser'

/**
 * Lays the code editor out with the chat and the browser beside it, each on the side the person chose. Two
 * panes on one side stack, with a draggable split between them. Column widths and splits are remembered.
 */
export function Workbench({ children }: { children: ReactNode }) {
  const { layout, setLayout } = useUi()
  const hide = (id: PaneId) => setLayout((l) => ({ ...l, [id]: { ...l[id], open: false } }))
  const panesOn = (side: Side): PaneId[] =>
    (['chat', 'browser'] as const).filter((id) => layout[id].open && layout[id].side === side)
  const pane = (id: PaneId) =>
    id === 'chat' ? (
      <MissionChatPane onClose={() => hide('chat')} className="min-h-0 grow" />
    ) : (
      <BrowserPane onClose={() => hide('browser')} className="min-h-0 grow" />
    )
  const left = panesOn('left')
  const right = panesOn('right')
  return (
    <div className="flex min-h-0 min-w-0 grow">
      {left.length > 0 ? <Column side="left" panes={left} render={pane} /> : null}
      <div className="flex min-h-0 min-w-[320px] grow">{children}</div>
      {right.length > 0 ? <Column side="right" panes={right} render={pane} /> : null}
    </div>
  )
}

function Column({ side, panes, render }: { side: Side; panes: PaneId[]; render: (id: PaneId) => ReactNode }) {
  const [width, handle] = useResizable({
    storageKey: `astra.size.workbench.${side}`,
    initial: side === 'left' ? 380 : 480,
    min: 260,
    max: 960,
    axis: 'x',
    invert: side === 'right',
  })
  const [first, second] = panes
  return (
    <div
      style={{ width }}
      className={cx(
        'relative box-border flex min-h-0 shrink-0 flex-col',
        side === 'left' ? 'border-r border-line' : 'border-l border-line',
      )}
    >
      <ResizeHandle {...handle} label={`Resize ${side} panes`} />
      {first && second ? (
        <Stack side={side} top={render(first)} bottom={render(second)} />
      ) : first ? (
        render(first)
      ) : null}
    </div>
  )
}

function Stack({ side, top, bottom }: { side: Side; top: ReactNode; bottom: ReactNode }) {
  const [height, handle] = useResizable({
    storageKey: `astra.size.workbench.${side}.split`,
    initial: 360,
    min: 160,
    max: 1200,
    axis: 'y',
  })
  return (
    <>
      <div style={{ height }} className="relative flex min-h-0 shrink-0 flex-col border-b border-line">
        {top}
        <ResizeHandle {...handle} label={`Resize ${side} split`} />
      </div>
      <div className="flex min-h-0 grow flex-col">{bottom}</div>
    </>
  )
}

interface Preset {
  label: string
  layout: (l: WorkbenchLayout) => WorkbenchLayout
}

const presets: Preset[] = [
  {
    label: 'Code only',
    layout: (l) => ({ ...l, chat: { ...l.chat, open: false }, browser: { ...l.browser, open: false } }),
  },
  {
    label: 'Chat · code',
    layout: (l) => ({ ...l, chat: { open: true, side: 'left' }, browser: { ...l.browser, open: false } }),
  },
  {
    label: 'Code · chat',
    layout: (l) => ({ ...l, chat: { open: true, side: 'right' }, browser: { ...l.browser, open: false } }),
  },
  {
    label: 'Code · browser',
    layout: (l) => ({ ...l, chat: { ...l.chat, open: false }, browser: { open: true, side: 'right' } }),
  },
  {
    label: 'Chat · code · browser',
    layout: (l) => ({ ...l, chat: { open: true, side: 'left' }, browser: { open: true, side: 'right' } }),
  },
  {
    label: 'Code · chat over browser',
    layout: (l) => ({ ...l, chat: { open: true, side: 'right' }, browser: { open: true, side: 'right' } }),
  },
]

/** The LAYOUT menu: presets, and each pane's visibility and side, plus where the terminal docks. */
export function LayoutMenu() {
  const host = useHost()
  const ui = useUi()
  const { layout, setLayout } = ui
  const [open, setOpen] = useState(false)
  const ref = useRef<HTMLDivElement>(null)

  useEffect(() => {
    if (!open) return
    const close = (e: MouseEvent) => {
      if (!ref.current?.contains(e.target as Node)) setOpen(false)
    }
    const esc = (e: KeyboardEvent) => e.key === 'Escape' && setOpen(false)
    window.addEventListener('mousedown', close)
    window.addEventListener('keydown', esc)
    return () => {
      window.removeEventListener('mousedown', close)
      window.removeEventListener('keydown', esc)
    }
  }, [open])

  const item =
    'flex h-8 w-full items-center gap-3 border-0 bg-transparent px-4 text-left text-body-sm text-ink hover:bg-surface-raised'
  const check = (on: boolean) => (
    <span
      aria-hidden="true"
      className={cx(
        'inline-block size-2 shrink-0 rounded-full',
        on ? 'bg-signal' : 'border border-line-strong',
      )}
    />
  )
  const sideRow = (id: PaneId, label: string) => (
    <div className="flex items-center gap-2 px-4 py-1">
      <button
        type="button"
        role="menuitemcheckbox"
        aria-checked={layout[id].open}
        className="flex h-8 grow items-center gap-3 border-0 bg-transparent p-0 text-left text-body-sm text-ink"
        onClick={() => setLayout((l) => ({ ...l, [id]: { ...l[id], open: !l[id].open } }))}
      >
        {check(layout[id].open)}
        {label}
      </button>
      <fieldset aria-label={`${label} side`} className="m-0 flex border-0 p-0">
        {(['left', 'right'] as const).map((side) => (
          <button
            key={side}
            type="button"
            role="menuitemradio"
            aria-checked={layout[id].side === side}
            onClick={() => setLayout((l) => ({ ...l, [id]: { open: true, side } }))}
            className={cx(
              'h-7 border border-line px-2 font-mono text-spec uppercase',
              side === 'right' && '-ml-px',
              layout[id].side === side ? 'bg-surface-raised text-ink' : 'bg-transparent text-ink-muted',
            )}
          >
            {side}
          </button>
        ))}
      </fieldset>
    </div>
  )

  return (
    <div ref={ref} className="relative">
      <button
        type="button"
        aria-haspopup="menu"
        aria-expanded={open}
        title="Arrange the code, chat, browser and terminal"
        className={buttonClass('text', 'sm')}
        onClick={() => setOpen((o) => !o)}
      >
        LAYOUT
      </button>
      {open ? (
        <div
          role="menu"
          aria-label="Layout"
          className="absolute top-10 right-0 z-50 flex w-72 flex-col border border-line bg-surface-overlay py-2 shadow-lg"
        >
          <Spec className="px-4 py-1">PRESETS</Spec>
          {presets.map((p) => (
            <button
              key={p.label}
              type="button"
              role="menuitem"
              className={item}
              onClick={() => {
                setLayout(p.layout)
                setOpen(false)
              }}
            >
              {p.label}
            </button>
          ))}
          <div className="my-2 border-t border-line" />
          <Spec className="px-4 py-1">PANES</Spec>
          {sideRow('chat', 'Chat')}
          {sideRow('browser', 'Browser')}
          {host.terminal ? (
            <div className="flex items-center gap-2 px-4 py-1">
              <button
                type="button"
                role="menuitemcheckbox"
                aria-checked={ui.terminalOpen}
                className="flex h-8 grow items-center gap-3 border-0 bg-transparent p-0 text-left text-body-sm text-ink"
                onClick={() => ui.setTerminalOpen(!ui.terminalOpen)}
              >
                {check(ui.terminalOpen)}
                Terminal
              </button>
              <fieldset aria-label="Terminal edge" className="m-0 flex border-0 p-0">
                {(['bottom', 'right'] as const).map((edge) => (
                  <button
                    key={edge}
                    type="button"
                    role="menuitemradio"
                    aria-checked={layout.terminal === edge}
                    onClick={() => {
                      setLayout((l) => ({ ...l, terminal: edge }))
                      ui.setTerminalOpen(true)
                    }}
                    className={cx(
                      'h-7 border border-line px-2 font-mono text-spec uppercase',
                      edge === 'right' && '-ml-px',
                      layout.terminal === edge
                        ? 'bg-surface-raised text-ink'
                        : 'bg-transparent text-ink-muted',
                    )}
                  >
                    {edge}
                  </button>
                ))}
              </fieldset>
            </div>
          ) : null}
          <div className="my-2 border-t border-line" />
          <button
            type="button"
            role="menuitem"
            className={cx(item, 'text-ink-muted')}
            onClick={() => {
              setLayout(() => defaultLayout)
              setOpen(false)
            }}
          >
            Reset layout
          </button>
        </div>
      ) : null}
    </div>
  )
}
