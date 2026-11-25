import { HlcClock, ulid } from './hlc'
import { applyOps, type Clocks, emptyState, insertEntities, newClocks } from './reducer'
import type { Connectivity, Entity, Id, Op, OpBody, Presence, State } from './types'

export interface SeedFile {
  workspaceId: string
  me: string
  entities: Entity[]
  presence: Presence[]
}

export function stateFromSeed(seed: SeedFile): State {
  const s = insertEntities(emptyState(seed.workspaceId, seed.me), seed.entities)
  return { ...s, presence: seed.presence }
}

/** A runner node's answer to a command op. */
export interface CommandOutcome {
  ok: boolean
  error: string | null
}

/** What every UI talks to, whether data comes from the in-browser mock, a local Tauri node or a remote relay. */
export interface AstraClient {
  readonly kind: 'mock' | 'tauri' | 'remote'
  /** This client's node id: a runner node's id in the desktop app, a sync-only id on the web and phones. */
  readonly nodeId: string
  /** Resolves once the first state is loaded. */
  readonly ready: Promise<void>
  getState(): State
  getConnectivity(): Connectivity
  subscribe(listener: () => void): () => void
  /** Stamps the bodies as ops from the current user, applies them locally and sends them on. */
  submit(bodies: OpBody[]): Promise<Op[]>
  setPresence(presence: { missionId: string | null; activity: string } | null): void
  /** Resolves with the node's result for a command op, or `null` if none arrives within `timeoutMs`. */
  commandResult(commandOp: Id, timeoutMs?: number): Promise<CommandOutcome | null>
}

/**
 * Local replica: holds State, reduces ops with LWW rules and notifies subscribers.
 * Subclasses decide where ops go (mock node, Tauri core, WebSocket relay).
 */
export abstract class Store implements AstraClient {
  abstract readonly kind: AstraClient['kind']
  abstract readonly ready: Promise<void>
  protected state: State = emptyState()
  protected clocks: Clocks = newClocks()
  protected connectivity: Connectivity = { online: true, relay: 'disabled' }
  protected clock: HlcClock
  private listeners = new Set<() => void>()
  private outcomes = new Map<Id, CommandOutcome>()
  private waiting = new Map<Id, ((o: CommandOutcome) => void)[]>()

  nodeId: string

  constructor(nodeId: string) {
    this.nodeId = nodeId
    this.clock = new HlcClock(nodeId)
  }

  getState = (): State => this.state
  getConnectivity = (): Connectivity => this.connectivity

  subscribe = (listener: () => void): (() => void) => {
    this.listeners.add(listener)
    return () => this.listeners.delete(listener)
  }

  protected emit(): void {
    for (const l of this.listeners) l()
  }

  protected load(state: State, clocks: Clocks = newClocks()): void {
    this.state = state
    this.clocks = clocks
    this.emit()
  }

  /** Applies ops that arrived from anywhere (including our own echoes; duplicates are skipped). */
  protected receive(ops: Op[]): void {
    if (ops.length === 0) return
    const sorted = [...ops].sort((a, b) => (a.hlc < b.hlc ? -1 : a.hlc > b.hlc ? 1 : 0))
    for (const op of sorted) {
      this.clock.observe(op.hlc)
      if (op.body.type === 'commandResult') {
        const outcome = { ok: op.body.ok, error: op.body.error }
        this.outcomes.set(op.body.commandOp, outcome)
        for (const resolve of this.waiting.get(op.body.commandOp) ?? []) resolve(outcome)
        this.waiting.delete(op.body.commandOp)
      }
    }
    const next = applyOps(this.state, this.clocks, sorted)
    if (next !== this.state) {
      this.state = next
      this.emit()
    }
  }

  protected setConnectivity(c: Connectivity): void {
    if (c.online === this.connectivity.online && c.relay === this.connectivity.relay) return
    this.connectivity = c
    this.emit()
  }

  protected stamp(bodies: OpBody[], actor = this.state.me): Op[] {
    return bodies.map((body) => ({ id: ulid(), hlc: this.clock.tick(), actor, body }))
  }

  protected abstract send(ops: Op[]): Promise<void>

  async submit(bodies: OpBody[]): Promise<Op[]> {
    const ops = this.stamp(bodies)
    this.receive(ops)
    await this.send(ops)
    return ops
  }

  setPresence(presence: { missionId: string | null; activity: string } | null): void {
    const me = this.state.me
    const others = this.state.presence.filter((p) => p.personId !== me)
    const mine: Presence[] = presence ? [{ personId: me, ...presence, at: new Date().toISOString() }] : []
    this.state = { ...this.state, presence: [...others, ...mine] }
    this.emit()
    this.sendPresence(mine)
  }

  protected sendPresence(_presence: Presence[]): void {}

  commandResult(commandOp: Id, timeoutMs = 20_000): Promise<CommandOutcome | null> {
    const known = this.outcomes.get(commandOp)
    if (known) return Promise.resolve(known)
    return new Promise((resolve) => {
      const timer = setTimeout(() => {
        const rest = (this.waiting.get(commandOp) ?? []).filter((r) => r !== done)
        if (rest.length) this.waiting.set(commandOp, rest)
        else this.waiting.delete(commandOp)
        resolve(null)
      }, timeoutMs)
      const done = (o: CommandOutcome) => {
        clearTimeout(timer)
        resolve(o)
      }
      this.waiting.set(commandOp, [...(this.waiting.get(commandOp) ?? []), done])
    })
  }
}
