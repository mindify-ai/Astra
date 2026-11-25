import { describe, expect, it } from 'vitest'
import conformance from '../../../fixtures/ops-conformance.json'
import seed from '../../../fixtures/seed.json'
import { formatHlc, HlcClock, ulid } from '../src/hlc'
import { applyOps, emptyState, insertEntities, newClocks } from '../src/reducer'
import { type SeedFile, stateFromSeed } from '../src/store'
import { collectionOf, type Entity, type EntityKind, type Op, type State } from '../src/types'

const lookup = (s: State, kind: EntityKind, id: string) =>
  (s[collectionOf[kind]] as Record<string, Entity>)[id]

describe('ops conformance (shared with Rust)', () => {
  for (const c of conformance.cases) {
    it(c.name, () => {
      const state = applyOps(insertEntities(emptyState(), c.initial as Entity[]), newClocks(), c.ops as Op[])
      for (const e of c.expect) {
        const got = lookup(state, e.kind as EntityKind, e.id) as unknown as
          | Record<string, unknown>
          | undefined
        if (e.fields === null) expect(got).toBeUndefined()
        else expect(got).toMatchObject(e.fields)
      }
    })
  }

  const permutations = <T>(xs: T[]): T[][] =>
    xs.length <= 1
      ? [xs]
      : xs.flatMap((x, i) => permutations([...xs.slice(0, i), ...xs.slice(i + 1)]).map((p) => [x, ...p]))

  /** Same filter as crates/astra-protocol/tests/fixtures.rs: a patch never precedes the put that created its entity. */
  const causal = (initial: Entity[], order: Op[]): boolean => {
    const known = new Set(initial.map((e) => `${e.kind}/${e.id}`))
    const putSomewhere = (key: string) =>
      order.some((o) => o.body.type === 'put' && `${o.body.entity.kind}/${o.body.entity.id}` === key)
    for (const op of order) {
      if (op.body.type === 'put') known.add(`${op.body.entity.kind}/${op.body.entity.id}`)
      else if (op.body.type === 'patch') {
        const key = `${op.body.entityKind}/${op.body.id}`
        if (putSomewhere(key) && !known.has(key)) return false
      }
    }
    return true
  }

  for (const c of conformance.cases) {
    it(`${c.name} (every causal arrival order)`, () => {
      for (const order of permutations(c.ops as Op[]).filter((o) => causal(c.initial as Entity[], o))) {
        const state = applyOps(insertEntities(emptyState(), c.initial as Entity[]), newClocks(), order)
        for (const e of c.expect) {
          const got = lookup(state, e.kind as EntityKind, e.id) as unknown as
            | Record<string, unknown>
            | undefined
          if (e.fields === null) expect(got).toBeUndefined()
          else expect(got).toMatchObject(e.fields)
        }
      }
    })
  }

  it('a patch newer than a concurrent delete revives the entity in any order', () => {
    const del: Op = {
      id: 'd',
      hlc: '0001732180000200:000000:n1',
      actor: 'n1',
      body: { type: 'delete', entityKind: 'person', id: 'p-r' },
    }
    const patch: Op = {
      id: 'p',
      hlc: '0001732180000300:000000:n2',
      actor: 'n2',
      body: { type: 'patch', entityKind: 'person', id: 'p-r', fields: { name: 'Back' } },
    }
    const seed: Entity[] = [{ kind: 'person', id: 'p-r', name: 'R', initials: 'R' }]
    for (const order of [
      [del, patch],
      [patch, del],
    ]) {
      const s = applyOps(insertEntities(emptyState(), seed), newClocks(), order)
      expect(s.people['p-r']).toMatchObject({ name: 'Back', initials: 'R' })
    }
    const older: Op = { ...patch, id: 'p2', hlc: '0001732180000100:000000:n2' }
    for (const order of [
      [del, older],
      [older, del],
    ]) {
      expect(applyOps(insertEntities(emptyState(), seed), newClocks(), order).people['p-r']).toBeUndefined()
    }
  })

  it('converges regardless of op order', () => {
    const all = conformance.cases.flatMap((c) => c.ops as Op[])
    const forward = applyOps(emptyState(), newClocks(), all)
    const backward = applyOps(emptyState(), newClocks(), [...all].reverse())
    expect(backward.people).toEqual(forward.people)
  })
})

describe('seed', () => {
  it('loads every entity into its collection', () => {
    const s = stateFromSeed(seed as SeedFile)
    expect(Object.keys(s.missions)).toEqual(['m-014', 'm-015', 'm-013'])
    expect(s.approvals['ap-0042']?.state).toBe('pending')
    expect(Object.keys(s.messages).length).toBe(8)
    expect(s.me).toBe('p-mark')
  })
})

describe('hlc', () => {
  it('orders lexicographically and is monotonic within a millisecond', () => {
    const c = new HlcClock('n1', () => 1000)
    const a = c.tick()
    const b = c.tick()
    expect(a < b).toBe(true)
    expect(a).toBe(formatHlc(1000, 0, 'n1'))
    c.observe(formatHlc(5000, 3, 'n2'))
    expect(c.tick()).toBe(formatHlc(5000, 4, 'n1'))
  })
  it('makes 26-char ULIDs', () => {
    expect(ulid()).toMatch(/^[0-9A-HJKMNP-TV-Z]{26}$/)
    // Monotonic within a millisecond: ids made together sort in the order they were made.
    const same = Array.from({ length: 50 }, () => ulid(1_763_716_320_000))
    expect([...same].sort()).toEqual(same)
    expect(new Set(same).size).toBe(50)
  })
})
