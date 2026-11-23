import { ulid } from './hlc'
import { type SeedFile, Store, stateFromSeed } from './store'
import type {
  Command,
  Entity,
  Id,
  LogLine,
  Message,
  MissionAgent,
  Op,
  OpBody,
  RaceResult,
  State,
  Task,
} from './types'

export interface MockOptions {
  seed: SeedFile
  /** Delay before the simulated node reacts, in ms. 0 makes tests synchronous-ish. */
  latency?: number
  nodeId?: string
}

const roleWord = {
  planner: 'Planner',
  builder: 'Builder',
  tester: 'Tester',
  reviewer: 'Reviewer',
  writer: 'Writer',
} as const

/**
 * In-browser client for the web demo, tests and design review. It plays the part of a runner
 * node: command ops get the same follow-up ops a real node would write (PROTOCOL.md "Who writes what").
 */
export class MockClient extends Store {
  readonly kind = 'mock' as const
  readonly ready: Promise<void>
  private readonly latency: number
  private timers = new Set<ReturnType<typeof setTimeout>>()

  constructor(opts: MockOptions) {
    super(opts.nodeId ?? 'n-browser')
    this.latency = opts.latency ?? 500
    this.load(stateFromSeed(opts.seed))
    this.ready = Promise.resolve()
  }

  /** Simulates going offline/online (for the offline badge and tests). */
  setOnline(online: boolean) {
    this.setConnectivity({ online, relay: online ? 'connected' : 'disconnected' })
  }

  dispose() {
    for (const t of this.timers) clearTimeout(t)
    this.timers.clear()
  }

  protected async send(ops: Op[]): Promise<void> {
    for (const op of ops) {
      if (op.body.type === 'command') this.handle(op, op.body.node, op.body.command)
    }
  }

  // ── simulated node ───────────────────────────────────────────────────────

  private later(fn: () => void, factor = 1) {
    if (this.latency === 0) return fn()
    const t = setTimeout(() => {
      this.timers.delete(t)
      fn()
    }, this.latency * factor)
    this.timers.add(t)
  }

  private node(node: Id, bodies: OpBody[]) {
    this.receive(this.stamp(bodies, node))
  }

  private ok(node: Id, op: Op, error: string | null = null) {
    this.node(node, [{ type: 'commandResult', commandOp: op.id, ok: error === null, error }])
  }

  private agentMessage(ma: MissionAgent, body: string, extra: Partial<Message> = {}): OpBody {
    return {
      type: 'put',
      entity: {
        kind: 'message',
        id: `msg-${ulid().toLowerCase()}`,
        missionId: ma.missionId,
        author: { type: 'agent', missionAgentId: ma.id },
        steering: null,
        to: null,
        body,
        plan: [],
        log: [],
        approvalId: null,
        at: new Date().toISOString(),
        ...extra,
      },
    }
  }

  private patch(kind: Entity['kind'], id: Id, fields: Record<string, unknown>): OpBody {
    return { type: 'patch', entityKind: kind, id, fields }
  }

  private agentsOf(s: State, missionId: Id) {
    return Object.values(s.missionAgents).filter((m) => m.missionId === missionId && m.enabled)
  }

  private canApprove(s: State, actor: Id, missionId: Id): boolean {
    const mission = s.missions[missionId]
    if (!mission) return false
    if (mission.ownerId === actor) return true
    const access = s.memberships[`${missionId}:${actor}`]?.access
    return mission.approvalPolicy === 'steerers' && access === 'steer'
  }

  private handle(op: Op, node: Id, cmd: Command) {
    this.later(() => {
      const s = this.state
      switch (cmd.type) {
        case 'approve':
        case 'deny': {
          const a = s.approvals[cmd.approvalId]
          if (a?.state !== 'pending') return this.ok(node, op, 'approval is not pending')
          if (!this.canApprove(s, op.actor, a.missionId)) return this.ok(node, op, 'not allowed to approve')
          const allow = cmd.type === 'approve'
          const ma = s.missionAgents[a.missionAgentId]
          const task = Object.values(s.tasks).find((t) => t.approvalId === a.id)
          const bodies: OpBody[] = [
            this.patch('approval', a.id, {
              state: allow ? 'approved' : 'denied',
              decidedBy: op.actor,
              decidedAt: new Date().toISOString(),
            }),
            this.patch('missionAgent', a.missionAgentId, { status: allow ? 'working' : 'idle' }),
          ]
          if (task) bodies.push(this.patch('task', task.id, { column: allow ? 'done' : 'queued' }))
          const integration = Object.values(s.tasks).find(
            (t) => t.missionAgentId === a.missionAgentId && t.column === 'queued' && t.id !== task?.id,
          )
          if (allow && integration) bodies.push(this.patch('task', integration.id, { column: 'working' }))
          if (allow && ma) {
            const log: LogLine[] = a.command ? [{ verb: 'run', target: a.command }] : []
            bodies.push(
              this.agentMessage(ma, 'Approved. Running it now, then the integration tests.', { log }),
            )
            bodies.push(this.patch('missionAgent', ma.id, { latest: log }))
          } else if (ma) {
            bodies.push(this.agentMessage(ma, 'Denied. I will skip that step and wait for directions.'))
          }
          this.node(node, bodies)
          this.refreshMissionStatus(node, a.missionId)
          return this.ok(node, op)
        }
        case 'sendMessage': {
          const target = cmd.to
            ? s.missionAgents[cmd.to]
            : this.agentsOf(s, cmd.missionId).find((m) => m.role === 'planner')
          const bodies: OpBody[] = []
          if (cmd.to && target) {
            if (target.steeredBy && target.steeredBy !== op.actor) {
              return this.ok(node, op, `steering held by ${target.steeredBy}`)
            }
            if (!target.steeredBy) bodies.push(this.patch('missionAgent', target.id, { steeredBy: op.actor }))
          }
          if (target) bodies.push(this.agentMessage(target, 'Got it. Adjusting the plan.'))
          this.node(node, bodies)
          return this.ok(node, op)
        }
        case 'takeSteering':
          this.node(node, [this.patch('missionAgent', cmd.missionAgentId, { steeredBy: op.actor })])
          return this.ok(node, op)
        case 'releaseSteering':
          this.node(node, [this.patch('missionAgent', cmd.missionAgentId, { steeredBy: null })])
          return this.ok(node, op)
        case 'pause':
          this.node(node, [
            this.patch('missionAgent', cmd.missionAgentId, { status: 'idle', statusDetail: 'paused' }),
          ])
          return this.ok(node, op)
        case 'resume':
          this.node(node, [
            this.patch('missionAgent', cmd.missionAgentId, { status: 'working', statusDetail: null }),
          ])
          return this.ok(node, op)
        case 'connectAgent':
          this.later(() => {
            this.node(node, [this.patch('agent', cmd.agentId, { status: 'connected' })])
            this.ok(node, op)
          })
          return
        case 'testAgent':
          return this.ok(node, op)
        case 'merge':
          return this.merge(op, node, cmd.resultId)
        case 'startMission':
          return this.start(op, node, cmd.missionId)
      }
    })
  }

  private refreshMissionStatus(node: Id, missionId: Id) {
    const s = this.state
    const mission = s.missions[missionId]
    if (!mission || mission.status === 'merged' || mission.status === 'done') return
    const agents = this.agentsOf(s, missionId)
    const status = agents.some((a) => a.status === 'needsYou')
      ? 'needsYou'
      : agents.some((a) => a.status === 'working')
        ? 'working'
        : mission.status
    if (status !== mission.status) this.node(node, [this.patch('mission', missionId, { status })])
  }

  private merge(op: Op, node: Id, resultId: Id) {
    const s = this.state
    const r = s.results[resultId]
    const mission = r ? s.missions[r.missionId] : undefined
    if (!r || !mission) return this.ok(node, op, 'unknown result')
    if (!this.canApprove(s, op.actor, mission.id)) return this.ok(node, op, 'not allowed to merge')
    const repo = s.repositories[mission.repoId]
    const others = Object.values(s.results).filter((x) => x.missionId === mission.id && x.id !== r.id)
    const pick = Object.values(s.tasks).find(
      (t) => t.missionId === mission.id && t.column === 'needsYou' && !t.approvalId,
    )
    const bodies: OpBody[] = [
      {
        type: 'put',
        entity: {
          kind: 'merge',
          id: mission.id,
          missionId: mission.id,
          resultId: r.id,
          into: `${repo?.name ?? mission.repoId} · ${mission.baseBranch}`,
          stats: r.stats,
          checks: r.checks,
          approvedBy: op.actor,
          note: r.reviewerPick ? "after Reviewer's pick" : null,
          keptBranches: others.map((o) => o.branch),
          at: new Date().toISOString(),
        },
      },
      this.patch('mission', mission.id, { status: 'merged', stats: r.stats }),
    ]
    if (pick) bodies.push(this.patch('task', pick.id, { column: 'done', mergedBy: op.actor }))
    this.node(node, bodies)
    this.ok(node, op)
  }

  private start(op: Op, node: Id, missionId: Id) {
    const s = this.state
    const mission = s.missions[missionId]
    if (!mission) return this.ok(node, op, 'unknown mission')
    const agents = this.agentsOf(s, missionId)
    const now = new Date().toISOString()
    const bodies: OpBody[] = [this.patch('mission', missionId, { status: 'working' })]
    const task = (ma: MissionAgent, title: string, column: Task['column'], order: number): OpBody => ({
      type: 'put',
      entity: {
        kind: 'task',
        id: `${ma.id}-t`,
        missionId,
        title,
        role: ma.role,
        missionAgentId: ma.id,
        column,
        note: null,
        mergedBy: null,
        approvalId: null,
        order,
      },
    })

    if (mission.mode === 'race') {
      const builders = agents.filter((a) => a.role === 'builder')
      const racers = builders.length > 0 ? builders : agents.filter((a) => a.role !== 'reviewer')
      racers.forEach((ma, i) => {
        const branch = `${mission.branch}-${String.fromCharCode(97 + i)}`
        bodies.push(
          this.patch('missionAgent', ma.id, {
            status: 'working',
            startedAt: now,
            branch,
            task: mission.title,
          }),
        )
        bodies.push(task(ma, mission.title, 'working', i + 1))
      })
      this.node(node, bodies)
      this.later(() => this.finishRace(node, missionId, racers), 3)
      return this.ok(node, op)
    }

    const planner = agents.find((a) => a.role === 'planner')
    const workers = agents.filter((a) => a !== planner)
    if (planner) {
      bodies.push(
        this.patch('missionAgent', planner.id, {
          status: 'idle',
          statusDetail: 'plan done',
          startedAt: now,
          branch: mission.branch,
          task: `Plan: ${mission.title}`,
        }),
      )
      bodies.push(task(planner, `Plan: ${mission.title}`, 'done', 1))
      bodies.push(
        this.agentMessage(planner, `Plan ready. ${workers.length} agents take it from here.`, {
          plan: [
            { text: 'Read the relevant code', state: 'done' },
            { text: 'Split the goal into tasks', state: 'done' },
            {
              text: `Hand the tasks to ${workers.map((w) => roleWord[w.role]).join(', ') || 'the team'}`,
              state: 'active',
            },
          ],
        }),
      )
    }
    workers.forEach((ma, i) => {
      const first = i === 0
      bodies.push(
        this.patch('missionAgent', ma.id, {
          status: first ? 'working' : 'queued',
          startedAt: first ? now : null,
          branch: mission.branch,
          task: `${roleWord[ma.role]}: ${mission.title}`,
        }),
      )
      bodies.push(task(ma, `${roleWord[ma.role]}: ${mission.title}`, first ? 'working' : 'queued', i + 1))
    })
    this.node(node, bodies)
    return this.ok(node, op)
  }

  private finishRace(node: Id, missionId: Id, racers: MissionAgent[]) {
    const s = this.state
    const mission = s.missions[missionId]
    if (!mission) return
    const bodies: OpBody[] = []
    racers.forEach((ma, i) => {
      const label = String.fromCharCode(65 + i)
      const result: RaceResult = {
        kind: 'result',
        id: `${missionId}-res-${label.toLowerCase()}`,
        missionId,
        label,
        missionAgentId: ma.id,
        branch: `${mission.branch}-${label.toLowerCase()}`,
        stats: { files: 1 + i, additions: 12 + i * 20, deletions: 2 + i },
        checks: 'passed',
        reviewerPick: i === 0,
        diff: [
          {
            path: 'src/index.ts',
            lines: [
              { kind: 'ctx', text: '// …' },
              { kind: 'add', text: `// ${mission.title} (${label})` },
            ],
          },
        ],
      }
      bodies.push({ type: 'put', entity: result })
      bodies.push(this.patch('missionAgent', ma.id, { status: 'done' }))
      bodies.push(this.patch('task', `${ma.id}-t`, { column: 'done' }))
    })
    const reviewer = this.agentsOf(s, missionId).find((a) => a.role === 'reviewer')
    if (reviewer) {
      bodies.push(this.patch('missionAgent', reviewer.id, { status: 'done' }))
      bodies.push(
        this.agentMessage(
          reviewer,
          'All results pass the tests. I would merge A: it is the smallest change.',
        ),
      )
    }
    bodies.push({
      type: 'put',
      entity: {
        kind: 'task',
        id: `${missionId}-pick`,
        missionId,
        title: `Pick one of ${racers.length} results`,
        role: 'reviewer',
        missionAgentId: null,
        column: 'needsYou',
        note: 'Results are ready to compare.',
        mergedBy: null,
        approvalId: null,
        order: 1,
      },
    })
    bodies.push(this.patch('mission', missionId, { status: 'needsYou' }))
    this.node(node, bodies)
  }
}
