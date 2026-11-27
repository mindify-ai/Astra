import { CloseIcon, cx, DockIcon, Eyebrow, GlobeIcon, RESIZE_STEP } from '@astra/ui'
import { useNavigate } from '@tanstack/react-router'
import {
  type KeyboardEvent as ReactKeyboardEvent,
  type PointerEvent as ReactPointerEvent,
  useCallback,
  useEffect,
  useRef,
  useState,
} from 'react'
import { useAstraState, usePlatform, useUi } from '../context'
import { BrowserView, browserIconButton } from '../shared/browser'

/**
 * The mini browser toggle for the header: an icon in the palette button's frame, since the header has no room
 * for another word.
 */
export function BrowserButton() {
  const ui = useUi()
  const open = ui.miniBrowserOpen
  return (
    <button
      type="button"
      aria-label="Mini browser"
      aria-pressed={open}
      title="Mini browser: a small window that stays over every screen"
      onClick={() => ui.setMiniBrowserOpen(!open)}
      className={cx(
        'flex size-8 shrink-0 items-center justify-center rounded-sm border bg-transparent p-0 hover:border-line-strong hover:text-ink',
        open ? 'border-line-strong text-ink' : 'border-line text-ink-muted',
      )}
    >
      <GlobeIcon size={14} />
    </button>
  )
}

interface Rect {
  x: number
  y: number
  w: number
  h: number
}
interface Viewport {
  w: number
  h: number
}
/** A window edge or corner being dragged, by compass point. */
type Grip = 'n' | 's' | 'e' | 'w' | 'nw' | 'ne' | 'sw' | 'se'

const STORAGE = 'astra.browser.mini'
const MIN = { w: 320, h: 240 }
const INITIAL = { w: 440, h: 340 }
/** The gap kept between the window and the viewport's edges. */
const MARGIN = 8
/** Where the window first opens: this far in from the bottom-right corner. */
const INSET = 24

const clamp = (n: number, min: number, max: number) => Math.min(Math.max(n, min), Math.max(min, max))
const viewport = (): Viewport =>
  typeof window === 'undefined' ? { w: 1440, h: 960 } : { w: window.innerWidth, h: window.innerHeight }

/** Keeps the window at least its minimum size and wholly inside the viewport. */
export function fitRect(r: Rect, vp: Viewport): Rect {
  const w = clamp(Math.round(r.w), MIN.w, vp.w - MARGIN * 2)
  const h = clamp(Math.round(r.h), MIN.h, vp.h - MARGIN * 2)
  return {
    w,
    h,
    x: clamp(Math.round(r.x), MARGIN, vp.w - w - MARGIN),
    y: clamp(Math.round(r.y), MARGIN, vp.h - h - MARGIN),
  }
}

const initialRect = (vp: Viewport): Rect =>
  fitRect({ ...INITIAL, x: vp.w - INITIAL.w - INSET, y: vp.h - INITIAL.h - INSET }, vp)

/** Drags one edge or corner by (dx, dy); the opposite edges stay where they are. */
export function resizeRect(from: Rect, grip: Grip, dx: number, dy: number, vp: Viewport): Rect {
  const right = from.x + from.w
  const bottom = from.y + from.h
  let { x, y, w, h } = from
  if (grip.includes('e')) w = clamp(from.w + dx, MIN.w, vp.w - MARGIN - from.x)
  if (grip.includes('w')) {
    w = clamp(from.w - dx, MIN.w, right - MARGIN)
    x = right - w
  }
  if (grip.includes('s')) h = clamp(from.h + dy, MIN.h, vp.h - MARGIN - from.y)
  if (grip.includes('n')) {
    h = clamp(from.h - dy, MIN.h, bottom - MARGIN)
    y = bottom - h
  }
  return { x: Math.round(x), y: Math.round(y), w: Math.round(w), h: Math.round(h) }
}

function readRect(): Rect | null {
  try {
    const raw = JSON.parse(window.localStorage.getItem(STORAGE) ?? 'null') as Partial<Rect> | null
    if (!raw || typeof raw !== 'object') return null
    const { x, y, w, h } = raw
    return [x, y, w, h].every((n) => typeof n === 'number' && Number.isFinite(n)) ? (raw as Rect) : null
  } catch {
    return null
  }
}
function writeRect(r: Rect | null) {
  try {
    if (r) window.localStorage.setItem(STORAGE, JSON.stringify(r))
    else window.localStorage.removeItem(STORAGE)
  } catch {
    /* storage is unavailable; the window just opens in its corner next time */
  }
}

const grips: { grip: Grip; cursor: string; className: string }[] = [
  { grip: 'n', cursor: 'ns-resize', className: 'inset-x-2 -top-1 h-2 cursor-ns-resize' },
  { grip: 's', cursor: 'ns-resize', className: 'inset-x-2 -bottom-1 h-2 cursor-ns-resize' },
  { grip: 'w', cursor: 'ew-resize', className: 'inset-y-2 -left-1 w-2 cursor-ew-resize' },
  { grip: 'e', cursor: 'ew-resize', className: 'inset-y-2 -right-1 w-2 cursor-ew-resize' },
  { grip: 'nw', cursor: 'nwse-resize', className: '-top-1 -left-1 size-3 cursor-nwse-resize' },
  { grip: 'se', cursor: 'nwse-resize', className: '-right-1 -bottom-1 size-3 cursor-nwse-resize' },
  { grip: 'ne', cursor: 'nesw-resize', className: '-top-1 -right-1 size-3 cursor-nesw-resize' },
  { grip: 'sw', cursor: 'nesw-resize', className: '-bottom-1 -left-1 size-3 cursor-nesw-resize' },
]

/**
 * The browser as a small window that floats over whatever screen is open, so a preview or a page of docs
 * stays in view while you move between the thread, the map and review. Drag its title to move it and its
 * edges to resize it; where it sits is remembered. Phones have no room for it.
 */
export function MiniBrowser() {
  const ui = useUi()
  const { layout } = usePlatform()
  if (!ui.miniBrowserOpen || layout === 'mobile') return null
  return <MiniWindow />
}

function MiniWindow() {
  const ui = useUi()
  const navigate = useNavigate()
  const hasRepo = Object.keys(useAstraState().repositories).length > 0
  const [vp, setVp] = useState(viewport)
  const [stored, setStored] = useState<Rect | null>(readRect)
  const [dragging, setDragging] = useState(false)
  // Clamped on every render, so shrinking the app's window moves this one with it without forgetting its place.
  const rect = stored ? fitRect(stored, vp) : initialRect(vp)

  const latest = useRef({ rect, vp })
  latest.current = { rect, vp }
  const drag = useRef<{
    mode: 'move' | Grip
    x: number
    y: number
    from: Rect
    cursor: string
    userSelect: string
  } | null>(null)

  useEffect(() => {
    const onResize = () => setVp(viewport())
    window.addEventListener('resize', onResize)
    return () => window.removeEventListener('resize', onResize)
  }, [])

  const place = (next: Rect, persist: boolean) => {
    latest.current.rect = next
    setStored(next)
    if (persist) writeRect(next)
  }

  const endDrag = useCallback(() => {
    const d = drag.current
    if (!d) return
    drag.current = null
    document.body.style.cursor = d.cursor
    document.body.style.userSelect = d.userSelect
    setDragging(false)
    writeRect(latest.current.rect)
  }, [])
  // Closing mid-drag must not strand the page with a drag cursor and no text selection.
  useEffect(() => endDrag, [endDrag])

  const dragHandlers = (mode: 'move' | Grip, cursor: string) => ({
    onPointerDown: (e: ReactPointerEvent<HTMLElement>) => {
      if (e.button !== 0) return
      e.preventDefault()
      try {
        e.currentTarget.setPointerCapture(e.pointerId)
      } catch {
        /* no capture (old engine or synthetic event); moves still arrive while over the handle */
      }
      drag.current = {
        mode,
        x: e.clientX,
        y: e.clientY,
        from: latest.current.rect,
        cursor: document.body.style.cursor,
        userSelect: document.body.style.userSelect,
      }
      document.body.style.cursor = cursor
      document.body.style.userSelect = 'none'
      setDragging(true)
    },
    onPointerMove: (e: ReactPointerEvent<HTMLElement>) => {
      const d = drag.current
      if (!d) return
      const dx = e.clientX - d.x
      const dy = e.clientY - d.y
      const { vp } = latest.current
      place(
        d.mode === 'move'
          ? fitRect({ ...d.from, x: d.from.x + dx, y: d.from.y + dy }, vp)
          : resizeRect(d.from, d.mode, dx, dy, vp),
        false,
      )
    },
    onPointerUp: endDrag,
    onPointerCancel: endDrag,
    onLostPointerCapture: endDrag,
  })

  /** Arrow keys move the window; with Shift they resize it. */
  const onKeyDown = (e: ReactKeyboardEvent<HTMLElement>) => {
    const dx = e.key === 'ArrowRight' ? RESIZE_STEP : e.key === 'ArrowLeft' ? -RESIZE_STEP : 0
    const dy = e.key === 'ArrowDown' ? RESIZE_STEP : e.key === 'ArrowUp' ? -RESIZE_STEP : 0
    if (dx === 0 && dy === 0) return
    e.preventDefault()
    const { rect: r, vp } = latest.current
    place(
      fitRect(e.shiftKey ? { ...r, w: r.w + dx, h: r.h + dy } : { ...r, x: r.x + dx, y: r.y + dy }, vp),
      true,
    )
  }

  const close = () => ui.setMiniBrowserOpen(false)
  const dock = () => {
    ui.setLayout((l) => ({ ...l, browser: { ...l.browser, open: true } }))
    close()
    void navigate({ to: '/code' })
  }

  return (
    <section
      aria-label="Mini browser"
      style={{ left: rect.x, top: rect.y, width: rect.w, height: rect.h }}
      className="fixed z-40 box-border flex flex-col border border-line-strong bg-surface shadow-lg"
    >
      <div className="flex h-8 shrink-0 items-center border-b border-line bg-surface-overlay pr-1">
        <button
          type="button"
          aria-label="Move mini browser"
          title="Drag to move · arrow keys move · Shift+arrows resize · double-click to reset"
          className={cx(
            'flex h-full min-w-0 grow touch-none items-center border-0 bg-transparent px-3 text-left',
            dragging ? 'cursor-grabbing' : 'cursor-grab',
          )}
          {...dragHandlers('move', 'grabbing')}
          onKeyDown={onKeyDown}
          onDoubleClick={() => {
            setStored(null)
            writeRect(null)
          }}
        >
          <Eyebrow>Browser</Eyebrow>
        </button>
        {hasRepo ? (
          <button
            type="button"
            aria-label="Dock beside the code"
            title="Dock beside the code"
            className={browserIconButton}
            onClick={dock}
          >
            <DockIcon size={14} />
          </button>
        ) : null}
        <button type="button" aria-label="Close mini browser" className={browserIconButton} onClick={close}>
          <CloseIcon size={14} />
        </button>
      </div>
      <div className="relative flex min-h-0 grow flex-col">
        <BrowserView />
        {/* A page under the pointer would swallow the drag; cover it until the drag ends. */}
        {dragging ? <div aria-hidden="true" className="absolute inset-0" /> : null}
      </div>
      {grips.map(({ grip, cursor, className }) => (
        <div
          key={grip}
          aria-hidden="true"
          data-grip={grip}
          className={cx('absolute z-10 touch-none select-none', className)}
          {...dragHandlers(grip, cursor)}
        />
      ))}
    </section>
  )
}
