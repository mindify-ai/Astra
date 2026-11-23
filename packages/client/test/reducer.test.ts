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
  })
})
