import { demoSeed, MockClient } from '@astra/client'
import { fireEvent, render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, describe, expect, it } from 'vitest'
import { AstraApp } from '../src'
import {
  clampToWorld,
  dropTarget,
  H,
  type Layout,
  MISSION,
  mergeLayout,
  NODE,
  swap,
  W,
} from '../src/screens/mapLayout'

const layout: Layout = {
  [MISSION]: { x: 320, y: 268 },
  a: { x: 340, y: 20 },
  b: { x: 616, y: 272 },
}

describe('map layout', () => {
  it('keeps boxes inside the world', () => {
    expect(clampToWorld('a', { x: -50, y: 9999 })).toEqual({ x: 0, y: H - NODE.h })
    expect(clampToWorld('a', { x: W, y: 10.4 })).toEqual({ x: W - NODE.w, y: 10 })
  })

  it('finds the agent a dragged box is dropped on, never the mission', () => {
    expect(dropTarget({ ...layout, a: { x: 620, y: 280 } }, 'a')).toBe('b')
    expect(dropTarget({ ...layout, a: { x: 330, y: 270 } }, 'a')).toBeNull()
    expect(dropTarget(layout, MISSION)).toBeNull()
  })

  it('swaps places: the dragged box takes the target spot, the target takes its origin', () => {
    const dragged = { ...layout, a: { x: 620, y: 280 } }
    const out = swap(dragged, 'a', 'b', layout.a as { x: number; y: number })
    expect(out.a).toEqual(layout.b)
    expect(out.b).toEqual(layout.a)
  })

  it('applies saved positions only for agents still on the mission', () => {
    const merged = mergeLayout(layout, { a: { x: 1, y: 2 }, gone: { x: 5, y: 5 } })
    expect(merged.a).toEqual({ x: 1, y: 2 })
    expect(merged.gone).toBeUndefined()
    expect(mergeLayout(layout, null)).toBe(layout)
  })
})

describe('map screen', () => {
  afterEach(() => localStorage.clear())

  it('moves an agent with the arrow keys, remembers it, and resets the layout', async () => {
    const client = new MockClient({ seed: demoSeed, latency: 0 })
    render(<AstraApp client={client} platform="mac" history={{ memory: '/missions/m-014/map' }} />)
    const user = userEvent.setup()
    const builder = await screen.findByRole('button', { name: /^Builder/ })
    const reset = screen.getByRole('button', { name: 'Reset layout' })
    expect((reset as HTMLButtonElement).disabled).toBe(true)
    const before = builder.style.left
    builder.focus()
    await user.keyboard('{ArrowRight}')
    const saved = JSON.parse(localStorage.getItem('astra.map.m-014') ?? '{}')
    expect(saved['ma-014-builder'].x).toBe(Number.parseFloat(before) + 16)
    expect((reset as HTMLButtonElement).disabled).toBe(false)
    await user.click(reset)
    expect(localStorage.getItem('astra.map.m-014')).toBeNull()
  })

  it('drops one agent on another to swap their places', async () => {
    const client = new MockClient({ seed: demoSeed, latency: 0 })
    render(<AstraApp client={client} platform="mac" history={{ memory: '/missions/m-014/map' }} />)
    const planner = await screen.findByRole('button', { name: /^Planner/ })
    const tester = screen.getByRole('button', { name: /^Tester/ })
    const p0 = { x: Number.parseFloat(planner.style.left), y: Number.parseFloat(planner.style.top) }
    const t0 = { x: Number.parseFloat(tester.style.left), y: Number.parseFloat(tester.style.top) }
    const at = (x: number, y: number) => ({ pointerId: 1, button: 0, clientX: x, clientY: y })
    fireEvent.pointerDown(planner, at(0, 0))
    fireEvent.pointerMove(planner, at(10, 10))
    fireEvent.pointerMove(planner, at(t0.x - p0.x, t0.y - p0.y))
    expect(screen.getByText('Swap')).toBeTruthy()
    fireEvent.pointerUp(planner, at(t0.x - p0.x, t0.y - p0.y))
    const saved = JSON.parse(localStorage.getItem('astra.map.m-014') ?? '{}')
    expect(saved['ma-014-planner']).toEqual(t0)
    expect(saved['ma-014-tester']).toEqual(p0)
    expect(await screen.findByText('Planner and Tester swapped places.')).toBeTruthy()
  })

  it('zooms with the toolbar', async () => {
    const client = new MockClient({ seed: demoSeed, latency: 0 })
    render(<AstraApp client={client} platform="mac" history={{ memory: '/missions/m-014/map' }} />)
    const user = userEvent.setup()
    await user.click(await screen.findByRole('button', { name: 'Zoom in' }))
    expect(screen.getByText('125%')).toBeTruthy()
  })
})
