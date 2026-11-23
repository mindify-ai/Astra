import { expect, it } from 'vitest'
import type * as G from '../src/generated/protocol'
import type * as H from '../src/types'

/** True only when A and B are mutually assignable. A mismatch is a type error in `checks` below. */
type Same<A, B> = [A] extends [B] ? ([B] extends [A] ? true : false) : false

// The hand-written client types must stay identical to the ones exported from Rust.
const checks: {
  person: Same<G.Person, H.Person>
  repository: Same<G.Repository, H.Repository>
  agent: Same<G.Agent, H.AgentConnection>
  mission: Same<G.Mission, H.Mission>
  missionAgent: Same<G.MissionAgent, H.MissionAgent>
  task: Same<G.Task, H.Task>
  message: Same<G.Message, H.Message>
  approval: Same<G.Approval, H.Approval>
  result: Same<G.MissionResult, H.RaceResult>
  merge: Same<G.Merge, H.MergeRecord>
  membership: Same<G.Membership, H.Membership>
  presence: Same<G.Presence, H.Presence>
  command: Same<G.Command, H.Command>
  op: Same<G.Op, H.Op>
  connectivity: Same<G.Connectivity, H.Connectivity>
} = {
  person: true,
  repository: true,
  agent: true,
  mission: true,
  missionAgent: true,
  task: true,
  message: true,
  approval: true,
  result: true,
  merge: true,
  membership: true,
  presence: true,
  command: true,
  op: true,
  connectivity: true,
}

it('client protocol types match the Rust-generated ones', () => {
  expect(Object.values(checks).every(Boolean)).toBe(true)
})
