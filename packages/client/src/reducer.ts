import type { Entity, EntityKind, Op, State } from './types'
import { collectionOf } from './types'

/**
 * Per-field last-writer-wins reducer. Rule for rule the same as crates/astra-protocol/src/reducer.rs
 * (fixtures/ops-conformance.json runs against both):
 * - seeded entities carry clock "" (older than any op);
 * - put/patch set each field whose clock is older than the op; patches to never-seen entities are ignored;
 * - a delete records a tombstone; an entity is visible iff it has no tombstone or its newest field clock is
 *   newer than the tombstone. A deleted entity's data stays hidden, so ops that arrive later still update it
 *   and a concurrent write newer than the delete revives it, whatever the arrival order.
 */
export interface EntityClock {
  fields: Record<string, string>
  tombstone?: string
  /** Last known data of a deleted (invisible) entity. */
  hidden?: Record<string, unknown>
}

export interface Clocks {
  /** `${kind}/${id}` → clock record */
  entities: Map<string, EntityClock>
  applied: Set<string>
}

export const newClocks = (): Clocks => ({ entities: new Map(), applied: new Set() })

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
    agentTypes: {},
    nodes: {},
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

const newest = (c: EntityClock): string => {
  let n = ''
  for (const v of Object.values(c.fields)) if (v > n) n = v
  return n
}
const visible = (c: EntityClock): boolean => c.tombstone === undefined || newest(c) > c.tombstone

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

  const write = (
    kind: EntityKind,
    id: string,
    hlc: string,
    incoming: Record<string, unknown>,
    isPut: boolean,
  ) => {
    const key = `${kind}/${id}`
    const prev = clocks.entities.get(key)
    const c = coll(kind)
    const current = (c[id] as unknown as Record<string, unknown> | undefined) ?? prev?.hidden
    if (!current && !isPut) return // patch to an entity never seen
    const clock: EntityClock = {
      fields: { ...(prev?.fields ?? {}) },
      tombstone: prev?.tombstone,
      hidden: prev?.hidden,
    }
    const data: Record<string, unknown> = { ...(current ?? {}) }
    let changed = false
    for (const [f, v] of Object.entries(incoming)) {
      if (f === 'kind' || f === 'id') continue
      if ((clock.fields[f] ?? '') < hlc) {
        clock.fields[f] = hlc
        data[f] = v
        changed = true
      }
    }
    if (!changed) return
    data.kind = kind
    data.id = id
    if (visible(clock)) {
      clock.hidden = undefined
      c[id] = data as unknown as Entity
    } else {
      delete c[id]
      clock.hidden = data
    }
    clocks.entities.set(key, clock)
  }

  for (const op of ops) {
    if (clocks.applied.has(op.id)) continue
    clocks.applied.add(op.id)
    const body = op.body
    if (body.type === 'put') {
      // Messages put before side threads existed carry no `side`; they belong to the main thread.
      const entity =
        body.entity.kind === 'message' && body.entity.side === undefined
          ? { ...body.entity, side: null }
          : body.entity
      write(entity.kind, entity.id, op.hlc, entity as unknown as Record<string, unknown>, true)
    } else if (body.type === 'patch') {
      write(body.entityKind, body.id, op.hlc, body.fields, false)
    } else if (body.type === 'delete') {
      const key = `${body.entityKind}/${body.id}`
      const prev = clocks.entities.get(key)
      if (prev?.tombstone !== undefined && prev.tombstone >= op.hlc) continue
      const clock: EntityClock = {
        fields: { ...(prev?.fields ?? {}) },
        tombstone: op.hlc,
        hidden: prev?.hidden,
      }
      const c = coll(body.entityKind)
      const existing = c[body.id]
      if (!visible(clock) && existing) {
        clock.hidden = existing as unknown as Record<string, unknown>
        delete c[body.id]
      }
      clocks.entities.set(key, clock)
    }
    // command / commandResult do not change entity state.
  }
  return next
}
