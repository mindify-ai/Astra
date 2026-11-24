// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import { type ResizableOptions, ResizeHandle, useResizable } from '../src'

afterEach(cleanup)
beforeEach(() => window.localStorage.clear())

const KEY = 'astra.size.test'

function Panel(props: Partial<ResizableOptions>) {
  const [size, handle] = useResizable({
    storageKey: KEY,
    initial: 300,
    min: 200,
    max: 500,
    axis: 'x',
    ...props,
  })
  return (
    <aside data-testid="panel" className="relative" style={{ width: size }}>
      <ResizeHandle {...handle} label="Resize panel" />
    </aside>
  )
}

const handle = () => screen.getByRole('separator', { name: 'Resize panel' })
const width = () => screen.getByTestId('panel').style.width

describe('useResizable / ResizeHandle', () => {
  it('is a focusable, labelled separator', () => {
    render(<Panel />)
    const h = handle()
    expect(h.getAttribute('aria-orientation')).toBe('vertical')
    expect(h.getAttribute('aria-valuenow')).toBe('300')
    expect(h.getAttribute('aria-valuemin')).toBe('200')
    expect(h.getAttribute('aria-valuemax')).toBe('500')
    expect(h.tabIndex).toBe(0)
    expect(width()).toBe('300px')
  })

  it('steps 16px with arrows, 64px with Shift, and jumps with Home/End', () => {
    render(<Panel />)
    const h = handle()
    fireEvent.keyDown(h, { key: 'ArrowRight' })
    expect(h.getAttribute('aria-valuenow')).toBe('316')
    fireEvent.keyDown(h, { key: 'ArrowLeft', shiftKey: true })
    expect(h.getAttribute('aria-valuenow')).toBe('252')
    fireEvent.keyDown(h, { key: 'End' })
    expect(h.getAttribute('aria-valuenow')).toBe('500')
    fireEvent.keyDown(h, { key: 'Home' })
    expect(h.getAttribute('aria-valuenow')).toBe('200')
    expect(width()).toBe('200px')
    // The other axis's arrows do nothing.
    fireEvent.keyDown(h, { key: 'ArrowUp' })
    expect(h.getAttribute('aria-valuenow')).toBe('200')
  })

  it('clamps keyboard steps to min and max', () => {
    render(<Panel initial={490} />)
    const h = handle()
    fireEvent.keyDown(h, { key: 'ArrowRight', shiftKey: true })
    expect(h.getAttribute('aria-valuenow')).toBe('500')
    fireEvent.keyDown(h, { key: 'Home' })
    fireEvent.keyDown(h, { key: 'ArrowLeft' })
    expect(h.getAttribute('aria-valuenow')).toBe('200')
  })

  it('inverts the y axis for a bottom dock (up grows it)', () => {
    render(<Panel axis="y" invert />)
    const h = handle()
    expect(h.getAttribute('aria-orientation')).toBe('horizontal')
    fireEvent.keyDown(h, { key: 'ArrowUp' })
    expect(h.getAttribute('aria-valuenow')).toBe('316')
    fireEvent.keyDown(h, { key: 'ArrowDown', shiftKey: true })
    expect(h.getAttribute('aria-valuenow')).toBe('252')
  })

  it('persists per key and clamps what it reads back', () => {
    const { unmount } = render(<Panel />)
    fireEvent.keyDown(handle(), { key: 'ArrowRight' })
    expect(window.localStorage.getItem(KEY)).toBe('316')
    unmount()

    render(<Panel />)
    expect(handle().getAttribute('aria-valuenow')).toBe('316')
    cleanup()

    window.localStorage.setItem(KEY, '9000')
    render(<Panel />)
    expect(handle().getAttribute('aria-valuenow')).toBe('500')
    cleanup()

    window.localStorage.setItem(KEY, 'not a number')
    render(<Panel />)
    expect(handle().getAttribute('aria-valuenow')).toBe('300')
  })

  it('survives storage that throws', () => {
    const storage = Object.getOwnPropertyDescriptor(window, 'localStorage')!
    Object.defineProperty(window, 'localStorage', {
      configurable: true,
      get() {
        throw new Error('denied')
      },
    })
    try {
      render(<Panel />)
      fireEvent.keyDown(handle(), { key: 'ArrowRight' })
      expect(handle().getAttribute('aria-valuenow')).toBe('316')
    } finally {
      Object.defineProperty(window, 'localStorage', storage)
    }
  })

  it('resets to the initial size on double-click', () => {
    render(<Panel />)
    const h = handle()
    fireEvent.keyDown(h, { key: 'End' })
    expect(window.localStorage.getItem(KEY)).toBe('500')
    fireEvent.doubleClick(h)
    expect(h.getAttribute('aria-valuenow')).toBe('300')
    expect(window.localStorage.getItem(KEY)).toBeNull()
  })

  it('drags with the pointer, locking the cursor and selection until release', () => {
    render(<Panel axis="x" invert />)
    const h = handle()
    document.body.style.cursor = 'auto'
    fireEvent.pointerDown(h, { button: 0, pointerId: 1, clientX: 100 })
    expect(document.body.style.cursor).toBe('col-resize')
    expect(document.body.style.userSelect).toBe('none')
    expect(h.getAttribute('data-dragging')).toBe('true')
    // Inverted (handle on the left edge): moving left grows the panel.
    fireEvent.pointerMove(h, { pointerId: 1, clientX: 60 })
    expect(width()).toBe('340px')
    fireEvent.pointerMove(h, { pointerId: 1, clientX: -1000 })
    expect(width()).toBe('500px')
    fireEvent.pointerUp(h, { pointerId: 1, clientX: -1000 })
    expect(document.body.style.cursor).toBe('auto')
    expect(document.body.style.userSelect).toBe('')
    expect(h.getAttribute('data-dragging')).toBeNull()
    expect(window.localStorage.getItem(KEY)).toBe('500')
    // Moves after release do nothing.
    fireEvent.pointerMove(h, { pointerId: 1, clientX: 100 })
    expect(width()).toBe('500px')
  })
})
