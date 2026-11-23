import type { Entity, EntityKind, Op, State } from './types'
import { collectionOf } from './types'

/**
 * Per-field last-writer-wins reducer. Same rules as crates/astra-protocol (see
 * fixtures/ops-conformance.json): seeded entities carry clock "" (older than any op).
 */
export interface Clocks {
  /** `${kind}/${id}` → field → hlc */
  fields: Map<string, Record<string, string>>
  /** `${kind}/${id}` → hlc of the delete */
  tombstones: Map<string, string>
  applied: Set<string>
}

export const newClocks = (): Clocks => ({ fields: new Map(), tombstones: new Map(), applied: new Set() })

export function emptyState(workspaceId = '', me = ''): State {
  return {
    workspaceId,
    me,
    people: {},
    repositories: {},
    agents: {},
    missions: {},
    missionAgents: {},
    tasks: {},
    messages: {},
    approvals: {},
    results: {},
    merges: {},
    memberships: {},
    presence: [],
  }
}

type AnyCollection = Record<string, Entity>

export function insertEntities(state: State, entities: Entity[]): State {
  const next: State = { ...state }
  const copied = new Set<string>()
  for (const e of entities) {
    const key = collectionOf[e.kind]
    if (!copied.has(key)) {
      ;(next as unknown as Record<string, AnyCollection>)[key] = { ...(next[key] as AnyCollection) }
      copied.add(key)
    }
    ;((next as unknown as Record<string, AnyCollection>)[key] as AnyCollection)[e.id] = e
  }
  return next
}

/** Applies ops in the given order; copies each touched collection once. */
export function applyOps(state: State, clocks: Clocks, ops: Op[]): State {
  const next: State = { ...state }
  const copied = new Set<string>()
  const coll = (kind: EntityKind): AnyCollection => {
    const key = collectionOf[kind]
    if (!copied.has(key)) {
      ;(next as unknown as Record<string, AnyCollection>)[key] = { ...(next[key] as AnyCollection) }
      copied.add(key)
    }
    return (next as unknown as Record<string, AnyCollection>)[key] as AnyCollection
  }

  for (const op of ops) {
    if (clocks.applied.has(op.id)) continue
    clocks.applied.add(op.id)
    const body = op.body
    if (body.type === 'put') {
      const { kind, id } = body.entity
      const key = `${kind}/${id}`
      const tomb = clocks.tombstones.get(key)
      if (tomb !== undefined && tomb >= op.hlc) continue
      const c = coll(kind)
      const existing = c[id]
      if (!existing) {
        clocks.tombstones.delete(key)
        c[id] = body.entity
        clocks.fields.set(key, Object.fromEntries(Object.keys(body.entity).map((f) => [f, op.hlc])))
        continue
      }
      const fc = clocks.fields.get(key) ?? {}
      const merged: Record<string, unknown> = { ...existing }
      for (const [f, v] of Object.entries(body.entity)) {
        if (f === 'kind' || f === 'id') continue
        if ((fc[f] ?? '') < op.hlc) {
          merged[f] = v
          fc[f] = op.hlc
        }
      }
      clocks.fields.set(key, fc)
      c[id] = merged as unknown as Entity
    } else if (body.type === 'patch') {
      const c = coll(body.entityKind)
      const existing = c[body.id]
      if (!existing) continue
      const key = `${body.entityKind}/${body.id}`
      const fc = clocks.fields.get(key) ?? {}
      const merged: Record<string, unknown> = { ...existing }
      let changed = false
      for (const [f, v] of Object.entries(body.fields)) {
        if (f === 'kind' || f === 'id') continue
        if ((fc[f] ?? '') < op.hlc) {
          merged[f] = v
          fc[f] = op.hlc
          changed = true
        }
      }
      clocks.fields.set(key, fc)
      if (changed) c[body.id] = merged as unknown as Entity
    } else if (body.type === 'delete') {
      const key = `${body.entityKind}/${body.id}`
      const c = coll(body.entityKind)
      const fc = clocks.fields.get(key) ?? {}
      // biome-ignore lint/complexity/useMathMinMax: HLCs are strings compared lexicographically
      const newest = Object.values(fc).reduce((a, b) => (a > b ? a : b), '')
      if (c[body.id] && newest >= op.hlc) continue
      delete c[body.id]
      clocks.fields.delete(key)
      const prev = clocks.tombstones.get(key)
      if (prev === undefined || prev < op.hlc) clocks.tombstones.set(key, op.hlc)
    }
    // command / commandResult do not change entity state.
  }
  return next
}
