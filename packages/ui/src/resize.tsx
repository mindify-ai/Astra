import {
  type KeyboardEvent as ReactKeyboardEvent,
  type PointerEvent as ReactPointerEvent,
  useCallback,
  useEffect,
  useRef,
  useState,
} from 'react'
import { cx } from './cx'

/** Which edge of the sized panel the handle sits on. */
export type ResizeEdge = 'left' | 'right' | 'top' | 'bottom'

export interface ResizableOptions {
  /** localStorage key the size is kept under, e.g. `astra.size.terminal`. */
  storageKey: string
  /** Size in px before the person resizes, and after a double-click reset. */
  initial: number
  min: number
  max: number
  /** `x` sizes a width (handle on a left/right edge), `y` a height (top/bottom edge). */
  axis: 'x' | 'y'
  /**
   * Grow when dragging towards the start (left/up) instead of the end. Use it when the handle sits on the
   * panel's left or top edge: a right-hand aside, or a bottom dock.
   */
  invert?: boolean
}

/** Everything `ResizeHandle` needs; spread it onto the handle. */
export interface ResizeHandleBinding {
  axis: 'x' | 'y'
  edge: ResizeEdge
  value: number
  min: number
  max: number
  dragging: boolean
  onPointerDown: (e: ReactPointerEvent<HTMLElement>) => void
  onPointerMove: (e: ReactPointerEvent<HTMLElement>) => void
  onPointerUp: (e: ReactPointerEvent<HTMLElement>) => void
  onPointerCancel: (e: ReactPointerEvent<HTMLElement>) => void
  onLostPointerCapture: (e: ReactPointerEvent<HTMLElement>) => void
  onDoubleClick: () => void
  onKeyDown: (e: ReactKeyboardEvent<HTMLElement>) => void
}

/** Keyboard step in px; Shift multiplies it by four. */
export const RESIZE_STEP = 16
export const RESIZE_STEP_LARGE = 64

const clamp = (n: number, min: number, max: number) => Math.min(Math.max(n, min), Math.max(min, max))

function readSize(key: string, initial: number, min: number, max: number): number {
  try {
    const raw = window.localStorage.getItem(key)
    if (raw === null) return clamp(initial, min, max)
    const n = Number(raw)
    return Number.isFinite(n) ? clamp(Math.round(n), min, max) : clamp(initial, min, max)
  } catch {
    return clamp(initial, min, max)
  }
}

function writeSize(key: string, size: number | null) {
  try {
    if (size === null) window.localStorage.removeItem(key)
    else window.localStorage.setItem(key, String(size))
  } catch {
    /* storage is unavailable (private mode, sandbox); the size just won't persist */
  }
}

/**
 * A panel size the person can change by dragging or with the keyboard, kept per `storageKey`.
 *
 * ```tsx
 * const [width, handle] = useResizable({ storageKey: 'astra.size.x', initial: 320, min: 240, max: 640, axis: 'x' })
 * <aside className="relative" style={{ width }}>
 *   …
 *   <ResizeHandle {...handle} label="Resize sidebar" />
 * </aside>
 * ```
 *
 * The returned size is always clamped to the current `min`/`max`, so a `max` derived from the window shrinks
 * the panel without forgetting the stored size.
 */
export function useResizable({
  storageKey,
  initial,
  min,
  max,
  axis,
  invert = false,
}: ResizableOptions): [number, ResizeHandleBinding] {
  const [stored, setStored] = useState(() => readSize(storageKey, initial, min, max))
  const [dragging, setDragging] = useState(false)
  const size = clamp(stored, min, max)

  const latest = useRef({ size, min, max })
  latest.current = { size, min, max }
  const drag = useRef<{ start: number; from: number; cursor: string; userSelect: string } | null>(null)

  const set = useCallback(
    (n: number, persist: boolean) => {
      const next = clamp(Math.round(n), latest.current.min, latest.current.max)
      latest.current.size = next
      setStored(next)
      if (persist) writeSize(storageKey, next)
    },
    [storageKey],
  )

  const coord = (e: { clientX: number; clientY: number }) => (axis === 'x' ? e.clientX : e.clientY)
  const sign = invert ? -1 : 1

  const endDrag = useCallback(() => {
    const d = drag.current
    if (!d) return
    drag.current = null
    document.body.style.cursor = d.cursor
    document.body.style.userSelect = d.userSelect
    setDragging(false)
    writeSize(storageKey, latest.current.size)
  }, [storageKey])

  // Leaving mid-drag (route change) must not strand the page with a resize cursor and no text selection.
  useEffect(() => endDrag, [endDrag])

  const binding: ResizeHandleBinding = {
    axis,
    edge: axis === 'x' ? (invert ? 'left' : 'right') : invert ? 'top' : 'bottom',
    value: size,
    min,
    max,
    dragging,
    onPointerDown: (e) => {
      if (e.button !== 0) return
      e.preventDefault()
      try {
        e.currentTarget.setPointerCapture(e.pointerId)
      } catch {
        /* no capture (old engine or synthetic event); moves still arrive while over the handle */
      }
      drag.current = {
        start: coord(e),
        from: latest.current.size,
        cursor: document.body.style.cursor,
        userSelect: document.body.style.userSelect,
      }
      document.body.style.cursor = axis === 'x' ? 'col-resize' : 'row-resize'
      document.body.style.userSelect = 'none'
      setDragging(true)
    },
    onPointerMove: (e) => {
      const d = drag.current
      if (!d) return
      set(d.from + (coord(e) - d.start) * sign, false)
    },
    onPointerUp: endDrag,
    onPointerCancel: endDrag,
    onLostPointerCapture: endDrag,
    onDoubleClick: () => {
      setStored(initial)
      latest.current.size = clamp(initial, min, max)
      writeSize(storageKey, null)
    },
    onKeyDown: (e) => {
      const step = e.shiftKey ? RESIZE_STEP_LARGE : RESIZE_STEP
      const [back, forward] = axis === 'x' ? ['ArrowLeft', 'ArrowRight'] : ['ArrowUp', 'ArrowDown']
      let next: number | null = null
      if (e.key === forward) next = latest.current.size + step * sign
      else if (e.key === back) next = latest.current.size - step * sign
      else if (e.key === 'Home') next = min
      else if (e.key === 'End') next = max
      if (next === null) return
      e.preventDefault()
      set(next, true)
    },
  }
  return [size, binding]
}

const edgeClass: Record<ResizeEdge, { hit: string; line: string }> = {
  // The 8px hit area straddles the panel edge; the hairline lands on the panel's own 1px border.
  right: { hit: 'inset-y-0 -right-1 w-2 cursor-col-resize', line: 'inset-y-0 right-1 w-px' },
  left: { hit: 'inset-y-0 -left-1 w-2 cursor-col-resize', line: 'inset-y-0 left-1 w-px' },
  top: { hit: 'inset-x-0 -top-1 h-2 cursor-row-resize', line: 'inset-x-0 top-1 h-px' },
  bottom: { hit: 'inset-x-0 -bottom-1 h-2 cursor-row-resize', line: 'inset-x-0 bottom-1 h-px' },
}

export interface ResizeHandleProps extends ResizeHandleBinding {
  /** Accessible name, e.g. "Resize terminal". */
  label: string
  className?: string
}

/**
 * The draggable edge of a resizable panel: a hairline that lights up in `signal` on hover, focus and drag,
 * with an 8px hit area. Place it inside a `relative` panel; it positions itself on `edge`.
 */
export function ResizeHandle({
  axis,
  edge,
  value,
  min,
  max,
  dragging,
  label,
  className,
  ...handlers
}: ResizeHandleProps) {
  const pos = edgeClass[edge]
  return (
    // biome-ignore lint/a11y/useSemanticElements: a focusable splitter is an ARIA widget; <hr> can't take focus or children
    <div
      role="separator"
      aria-orientation={axis === 'x' ? 'vertical' : 'horizontal'}
      aria-label={label}
      aria-valuenow={value}
      aria-valuemin={min}
      aria-valuemax={max}
      tabIndex={0}
      title="Drag to resize · double-click to reset"
      data-dragging={dragging || undefined}
      className={cx('group absolute z-10 touch-none select-none', pos.hit, className)}
      {...handlers}
    >
      <span
        aria-hidden="true"
        className={cx(
          'pointer-events-none absolute transition-colors duration-150 group-hover:bg-signal group-focus-visible:bg-signal',
          pos.line,
          dragging ? 'bg-signal' : 'bg-line',
        )}
      />
    </div>
  )
}
