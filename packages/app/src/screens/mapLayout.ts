import type { MissionAgent } from '@astra/client'

/** The map's world: a fixed canvas the viewport pans and zooms over. */
export const W = 880
export const H = 640
export const NODE = { w: 200, h: 96 }
export const CENTER = { w: 240, h: 104 }
const ORBIT = { rx: 276, ry: 208 }
/** The mission box's id in a layout. */
export const MISSION = 'mission'

export interface Point {
  x: number
  y: number
}
/** Top-left corners keyed by mission-agent id (and `MISSION`). */
export type Layout = Record<string, Point>

export interface Placed {
  ma: MissionAgent
  x: number
  y: number
}

/** Agents sit clockwise on the inner orbit, starting at the top, in role order. */
export function placeAgents(agents: MissionAgent[]): Placed[] {
  return agents.map((ma, i) => {
    const angle = -Math.PI / 2 + (i * 2 * Math.PI) / Math.max(agents.length, 1)
    const cx = W / 2 + ORBIT.rx * Math.cos(angle)
    const cy = H / 2 + ORBIT.ry * Math.sin(angle)
    return { ma, x: Math.round(cx - NODE.w / 2), y: Math.round(cy - NODE.h / 2) }
  })
}

/** The orbit layout, with the mission in the middle. */
export function defaultLayout(agents: MissionAgent[]): Layout {
  const out: Layout = { [MISSION]: { x: W / 2 - CENTER.w / 2, y: H / 2 - CENTER.h / 2 } }
  for (const p of placeAgents(agents)) out[p.ma.id] = { x: p.x, y: p.y }
  return out
}

export const sizeOf = (id: string) => (id === MISSION ? CENTER : NODE)

/** Keeps a box inside the world. */
export function clampToWorld(id: string, p: Point): Point {
  const s = sizeOf(id)
  return {
    x: Math.round(Math.min(Math.max(p.x, 0), W - s.w)),
    y: Math.round(Math.min(Math.max(p.y, 0), H - s.h)),
  }
}

/** Saved positions win over the default layout; positions for agents no longer on the mission are dropped. */
export function mergeLayout(base: Layout, saved: Layout | null): Layout {
  if (!saved) return base
  const out: Layout = { ...base }
  for (const id of Object.keys(base)) {
    const p = saved[id]
    if (p && Number.isFinite(p.x) && Number.isFinite(p.y)) out[id] = clampToWorld(id, p)
  }
  return out
}

/** The agent box whose area contains the dragged box's centre, if any (the drop target). */
export function dropTarget(layout: Layout, dragId: string): string | null {
  const d = layout[dragId]
  if (!d || dragId === MISSION) return null
  const s = sizeOf(dragId)
  const cx = d.x + s.w / 2
  const cy = d.y + s.h / 2
  for (const [id, p] of Object.entries(layout)) {
    if (id === dragId || id === MISSION) continue
    if (cx >= p.x && cx <= p.x + NODE.w && cy >= p.y && cy <= p.y + NODE.h) return id
  }
  return null
}

/** Dropping one agent on another trades their places: the dragged one takes the target's spot. */
export function swap(layout: Layout, dragId: string, targetId: string, dragOrigin: Point): Layout {
  const target = layout[targetId]
  if (!target) return layout
  return { ...layout, [dragId]: { ...target }, [targetId]: { ...dragOrigin } }
}

/** Point where the segment from a box's centre towards (tx, ty) leaves the box. */
export function exitPoint(cx: number, cy: number, w: number, h: number, tx: number, ty: number): Point {
  const dx = tx - cx
  const dy = ty - cy
  const sx = dx === 0 ? Number.POSITIVE_INFINITY : w / 2 / Math.abs(dx)
  const sy = dy === 0 ? Number.POSITIVE_INFINITY : h / 2 / Math.abs(dy)
  const s = Math.min(sx, sy)
  if (!Number.isFinite(s)) return { x: cx, y: cy }
  return { x: cx + dx * s, y: cy + dy * s }
}

const key = (missionId: string) => `astra.map.${missionId}`

export function loadLayout(missionId: string): Layout | null {
  try {
    const raw = localStorage.getItem(key(missionId))
    return raw ? (JSON.parse(raw) as Layout) : null
  } catch {
    return null
  }
}

export function saveLayout(missionId: string, layout: Layout | null) {
  try {
    if (layout) localStorage.setItem(key(missionId), JSON.stringify(layout))
    else localStorage.removeItem(key(missionId))
  } catch {
    // Private windows and blocked storage: the layout just isn't remembered.
  }
}
