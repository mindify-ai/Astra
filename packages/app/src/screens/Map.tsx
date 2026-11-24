import { agentCommand, type MissionAgent } from '@astra/client'
import {
  Button,
  buttonClass,
  cx,
  Data,
  Dot,
  Eyebrow,
  KVList,
  LogWell,
  Spec,
  Status,
  toneTextClass,
} from '@astra/ui'
import { Link } from '@tanstack/react-router'
import {
  type KeyboardEvent as ReactKeyboardEvent,
  type PointerEvent as ReactPointerEvent,
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from 'react'
import { useAstraState, useClient, useUi } from '../context'
import {
  agentOf,
  agentStatus,
  canSteer,
  hhmm,
  members,
  missionAgents,
  missionNo,
  personName,
  roleName,
} from '../select'
import { AppHeader, NewMissionButton, Page, ShareButton } from '../shell/AppHeader'
import { MissionNotFound, useMissionPresence } from './common'
import {
  CENTER,
  clampToWorld,
  defaultLayout,
  dropTarget,
  exitPoint,
  H,
  type Layout,
  loadLayout,
  MISSION,
  mergeLayout,
  NODE,
  type Point,
  saveLayout,
  sizeOf,
  swap,
  W,
} from './mapLayout'

export { placeAgents } from './mapLayout'

const ZOOM = { min: 0.5, max: 1.75, step: 0.25 }
const TWEEN_MS = 320
const KEY_STEP = 16

const edgeStyle = (ma: MissionAgent) => {
  if (ma.status === 'working') return { color: 'var(--signal)', mode: 'flow', label: 'working now' } as const
  if (ma.status === 'needsYou')
    return { color: 'var(--warning)', mode: 'pulse', label: 'waiting for your approval' } as const
  if (ma.status === 'queued') return { color: 'var(--line-strong)', mode: 'dashed', label: 'queued' } as const
  if (ma.status === 'offline')
    return { color: 'var(--line-strong)', mode: 'dashed', label: 'offline' } as const
  return { color: 'var(--line-strong)', mode: 'solid', label: ma.statusDetail ?? 'finished' } as const
}

function nodeLine(s: ReturnType<typeof useAstraState>, ma: MissionAgent): string {
  const st = agentStatus[ma.status]
  if (ma.status === 'working' && ma.steeredBy) return `${st.label} · ${personName(s, ma.steeredBy)} steering`
  if (ma.statusDetail) return `${st.label} · ${ma.statusDetail}`
  return st.label
}

function useReducedMotion(): boolean {
  const [reduced, setReduced] = useState(() => {
    try {
      return window.matchMedia('(prefers-reduced-motion: reduce)').matches
    } catch {
      return false
    }
  })
  useEffect(() => {
    let mq: MediaQueryList
    try {
      mq = window.matchMedia('(prefers-reduced-motion: reduce)')
    } catch {
      return
    }
    const on = () => setReduced(mq.matches)
    mq.addEventListener?.('change', on)
    return () => mq.removeEventListener?.('change', on)
  }, [])
  return reduced
}

/** Eases what's drawn towards the target layout; the box under the pointer follows it exactly. */
function useTweened(target: Layout, follow: string | null, reduced: boolean): Layout {
  const [shown, setShown] = useState(target)
  const shownRef = useRef(shown)
  shownRef.current = shown
  useEffect(() => {
    if (reduced || typeof requestAnimationFrame !== 'function') {
      setShown(target)
      return
    }
    const from = shownRef.current
    const start = performance.now()
    let raf = 0
    const step = (now: number) => {
      const t = Math.min(1, (now - start) / TWEEN_MS)
      const e = 1 - (1 - t) ** 3
      const next: Layout = {}
      for (const [id, p] of Object.entries(target)) {
        const f = from[id] ?? p
        next[id] = id === follow ? p : { x: f.x + (p.x - f.x) * e, y: f.y + (p.y - f.y) * e }
      }
      setShown(next)
      if (t < 1) raf = requestAnimationFrame(step)
    }
    raf = requestAnimationFrame(step)
    return () => cancelAnimationFrame(raf)
  }, [target, follow, reduced])
  return shown
}

interface Drag {
  id: string
  pointerId: number
  start: Point
  origin: Point
  moved: boolean
}
interface Pan {
  pointerId: number
  start: Point
  origin: Point
}

/** Map: the mission at the centre, its agents in orbit, and the selected agent's detail. */
export function MapView({ missionId }: { missionId: string }) {
  const s = useAstraState()
  const client = useClient()
  const ui = useUi()
  const mission = s.missions[missionId]
  const agents = missionAgents(s, missionId)
  const [selectedId, setSelectedId] = useState<string | null>(null)
  useMissionPresence(missionId, 'Looking at the map')
  useEffect(() => {
    if (mission) ui.setLastMissionId(missionId)
  }, [missionId, mission, ui])

  const reduced = useReducedMotion()
  const agentKey = agents.map((a) => a.id).join(',')
  // biome-ignore lint/correctness/useExhaustiveDependencies: the default layout depends only on which agents are on the mission
  const base = useMemo(() => defaultLayout(agents), [agentKey])
  const [custom, setCustom] = useState<Layout | null>(() => loadLayout(missionId))
  useEffect(() => setCustom(loadLayout(missionId)), [missionId])
  const layout = useMemo(() => mergeLayout(base, custom), [base, custom])

  const [dragId, setDragId] = useState<string | null>(null)
  const [overId, setOverId] = useState<string | null>(null)
  const [zoom, setZoom] = useState(1)
  const [pan, setPan] = useState<Point>({ x: 0, y: 0 })
  const [announce, setAnnounce] = useState('')
  const [isPanning, setIsPanning] = useState(false)
  const drag = useRef<Drag | null>(null)
  const panning = useRef<Pan | null>(null)
  const suppressClick = useRef(false)
  const viewport = useRef<HTMLDivElement>(null)
  const shown = useTweened(layout, dragId, reduced)

  const commit = useCallback(
    (next: Layout) => {
      setCustom(next)
      saveLayout(missionId, next)
    },
    [missionId],
  )

  // ⌘/Ctrl + wheel (and trackpad pinch) zooms; a plain wheel scrolls the page as usual.
  useEffect(() => {
    const el = viewport.current
    if (!el) return
    const onWheel = (e: WheelEvent) => {
      if (!e.ctrlKey && !e.metaKey) return
      e.preventDefault()
      setZoom((z) => Math.min(ZOOM.max, Math.max(ZOOM.min, z * Math.exp(-e.deltaY / 300))))
    }
    el.addEventListener('wheel', onWheel, { passive: false })
    return () => el.removeEventListener('wheel', onWheel)
  }, [])

  if (!mission) return <MissionNotFound />

  const selected =
    (selectedId && s.missionAgents[selectedId]) || agents.find((a) => a.status === 'working') || agents[0]
  const counts = {
    working: agents.filter((a) => a.status === 'working').length,
    needs: agents.filter((a) => a.status === 'needsYou').length,
  }
  const label = (id: string) =>
    id === MISSION ? `Mission ${missionNo(mission)}` : roleName[s.missionAgents[id]?.role ?? 'builder']

  const startDrag = (id: string, e: ReactPointerEvent<HTMLElement>) => {
    if (e.button !== 0) return
    e.stopPropagation()
    e.currentTarget.setPointerCapture?.(e.pointerId)
    drag.current = {
      id,
      pointerId: e.pointerId,
      start: { x: e.clientX, y: e.clientY },
      origin: layout[id] ?? { x: 0, y: 0 },
      moved: false,
    }
  }
  const moveDrag = (e: ReactPointerEvent<HTMLElement>) => {
    const d = drag.current
    if (!d || d.pointerId !== e.pointerId) return
    const dx = (e.clientX - d.start.x) / zoom
    const dy = (e.clientY - d.start.y) / zoom
    if (!d.moved && Math.hypot(dx, dy) < 4) return
    if (!d.moved) {
      d.moved = true
      setDragId(d.id)
    }
    const next = { ...layout, [d.id]: clampToWorld(d.id, { x: d.origin.x + dx, y: d.origin.y + dy }) }
    setCustom(next)
    setOverId(dropTarget(next, d.id))
  }
  const endDrag = (e: ReactPointerEvent<HTMLElement>, cancel = false) => {
    const d = drag.current
    if (!d || d.pointerId !== e.pointerId) return
    drag.current = null
    setDragId(null)
    setOverId(null)
    if (!d.moved) return
    suppressClick.current = true
    if (cancel) {
      commit({ ...layout, [d.id]: d.origin })
      return
    }
    const target = dropTarget(layout, d.id)
    if (target) {
      commit(swap(layout, d.id, target, d.origin))
      setAnnounce(`${label(d.id)} and ${label(target)} swapped places.`)
    } else {
      commit(layout)
      setAnnounce(`${label(d.id)} moved.`)
    }
  }
  const nudge = (id: string, e: ReactKeyboardEvent<HTMLElement>) => {
    const step = e.shiftKey ? KEY_STEP * 4 : KEY_STEP
    const delta: Record<string, Point> = {
      ArrowLeft: { x: -step, y: 0 },
      ArrowRight: { x: step, y: 0 },
      ArrowUp: { x: 0, y: -step },
      ArrowDown: { x: 0, y: step },
    }
    const dlt = delta[e.key]
    if (!dlt) return
    e.preventDefault()
    const p = layout[id] ?? { x: 0, y: 0 }
    commit({ ...layout, [id]: clampToWorld(id, { x: p.x + dlt.x, y: p.y + dlt.y }) })
  }
  const nodeHandlers = (id: string) => ({
    onPointerDown: (e: ReactPointerEvent<HTMLElement>) => startDrag(id, e),
    onPointerMove: moveDrag,
    onPointerUp: (e: ReactPointerEvent<HTMLElement>) => endDrag(e),
    onPointerCancel: (e: ReactPointerEvent<HTMLElement>) => endDrag(e, true),
    onKeyDown: (e: ReactKeyboardEvent<HTMLElement>) => nudge(id, e),
  })

  const startPan = (e: ReactPointerEvent<HTMLDivElement>) => {
    if (e.button !== 0) return
    e.currentTarget.setPointerCapture?.(e.pointerId)
    panning.current = { pointerId: e.pointerId, start: { x: e.clientX, y: e.clientY }, origin: pan }
    setIsPanning(true)
  }
  const movePan = (e: ReactPointerEvent<HTMLDivElement>) => {
    const p = panning.current
    if (!p || p.pointerId !== e.pointerId) return
    setPan({ x: p.origin.x + e.clientX - p.start.x, y: p.origin.y + e.clientY - p.start.y })
  }
  const endPan = () => {
    panning.current = null
    setIsPanning(false)
  }

  const zoomBy = (dz: number) =>
    setZoom((z) => Math.min(ZOOM.max, Math.max(ZOOM.min, Math.round((z + dz) * 100) / 100)))
  const fit = () => {
    setPan({ x: 0, y: 0 })
    const w = viewport.current?.clientWidth ?? W
    setZoom(Math.min(1, Math.max(ZOOM.min, (w - 32) / W)))
  }
  const resetLayout = () => {
    setCustom(null)
    saveLayout(missionId, null)
    setAnnounce('Layout reset.')
  }

  const m = shown[MISSION] ?? { x: W / 2 - CENTER.w / 2, y: H / 2 - CENTER.h / 2 }
  const mcx = m.x + CENTER.w / 2
  const mcy = m.y + CENTER.h / 2
  const edges = agents.map((ma) => {
    const p = shown[ma.id] ?? { x: 0, y: 0 }
    const ncx = p.x + NODE.w / 2
    const ncy = p.y + NODE.h / 2
    const a = exitPoint(mcx, mcy, CENTER.w, CENTER.h, ncx, ncy)
    const b = exitPoint(ncx, ncy, NODE.w + 16, NODE.h + 16, mcx, mcy)
    return { ma, d: `M${a.x.toFixed(1)} ${a.y.toFixed(1)} L${b.x.toFixed(1)} ${b.y.toFixed(1)}` }
  })
  const moved = custom !== null

  return (
    <Page
      header={
        <AppHeader
          repoSelect
          nav="map"
          people={members(s, missionId).map((p) => p.person)}
          actions={
            <>
              <ShareButton missionId={missionId} />
              <NewMissionButton />
            </>
          }
        />
      }
    >
      <div className="flex grow flex-wrap">
        <main className="flex min-w-0 flex-[999_1_560px] flex-col">
          <div className="flex flex-wrap items-end justify-between gap-x-8 gap-y-4 px-8 py-6">
            <div className="flex flex-col gap-2">
              <Eyebrow>Mission {missionNo(mission)}</Eyebrow>
              <h1 className="m-0 text-title">{mission.title}</h1>
            </div>
            <Spec>
              {agents.length} agents · {counts.working} working · {counts.needs} needs you
            </Spec>
          </div>
          <div className="px-8 pb-8">
            <div
              ref={viewport}
              role="application"
              aria-label="Mission map"
              aria-describedby="astra-map-hint"
              className={cx(
                'relative touch-none overflow-hidden border border-line-subtle bg-surface select-none',
                isPanning ? 'cursor-grabbing' : 'cursor-grab',
              )}
              style={{ height: H }}
              onPointerDown={startPan}
              onPointerMove={movePan}
              onPointerUp={endPan}
              onPointerCancel={endPan}
            >
              <div
                className="absolute"
                style={{
                  left: '50%',
                  top: '50%',
                  width: W,
                  height: H,
                  transform: `translate(-50%, -50%) translate(${pan.x}px, ${pan.y}px) scale(${zoom})`,
                  transition: reduced || isPanning ? undefined : 'transform 200ms ease-out',
                }}
              >
                <div
                  aria-hidden="true"
                  className="absolute box-border rounded-[50%] border border-line-subtle"
                  style={{ left: 64, top: 64, width: 752, height: 512 }}
                />
                <div
                  aria-hidden="true"
                  className="absolute box-border rounded-[50%] border border-line"
                  style={{ left: 164, top: 112, width: 552, height: 416 }}
                />
                <svg
                  className="pointer-events-none absolute inset-0"
                  width={W}
                  height={H}
                  role="img"
                  aria-label="Mission and agents"
                >
                  <defs>
                    <marker
                      id="astra-arrow"
                      viewBox="0 0 4 4"
                      refX="3.5"
                      refY="2"
                      markerWidth="8"
                      markerHeight="8"
                      orient="auto"
                    >
                      <path d="M0 0 L4 2 L0 4 Z" fill="context-stroke" />
                    </marker>
                  </defs>
                  {edges.map(({ ma, d }) => {
                    const e = edgeStyle(ma)
                    const isSel = selected?.id === ma.id
                    return (
                      <g key={ma.id}>
                        <path
                          d={d}
                          fill="none"
                          stroke={e.color}
                          strokeWidth={isSel ? 2.25 : 1.5}
                          strokeLinecap="round"
                          strokeDasharray={e.mode === 'solid' ? undefined : '6 6'}
                          markerEnd="url(#astra-arrow)"
                          className={cx(
                            e.mode === 'flow' && 'astra-flow',
                            e.mode === 'pulse' && 'astra-pulse',
                          )}
                        >
                          <title>{`${roleName[ma.role]}: ${e.label}`}</title>
                        </path>
                        {e.mode === 'flow' && !reduced ? (
                          <circle r="3" fill={e.color}>
                            <animateMotion dur="1.8s" repeatCount="indefinite" path={d} />
                          </circle>
                        ) : null}
                      </g>
                    )
                  })}
                </svg>
                <button
                  type="button"
                  aria-label={`Mission ${missionNo(mission)}: ${mission.title}. Drag or use the arrow keys to move.`}
                  {...nodeHandlers(MISSION)}
                  onClick={(e) => e.preventDefault()}
                  className={cx(
                    'astra-pop absolute box-border flex touch-none flex-col items-start border bg-surface px-4 py-3 text-left text-ink',
                    dragId === MISSION ? 'z-20 cursor-grabbing border-signal' : 'z-10 cursor-grab border-ink',
                  )}
                  style={{ left: m.x, top: m.y, width: CENTER.w, height: CENTER.h }}
                >
                  <Spec>Mission {missionNo(mission)}</Spec>
                  <b className="font-semibold">{mission.title}</b>
                </button>
                {agents.map((ma, i) => {
                  const p = shown[ma.id] ?? { x: 0, y: 0 }
                  const st = agentStatus[ma.status]
                  const active = ma.status === 'working'
                  const isSel = selected?.id === ma.id
                  const isDrag = dragId === ma.id
                  const isOver = overId === ma.id
                  const size = sizeOf(ma.id)
                  return (
                    <button
                      key={ma.id}
                      type="button"
                      aria-pressed={isSel}
                      aria-describedby="astra-map-hint"
                      {...nodeHandlers(ma.id)}
                      onClick={() => {
                        if (suppressClick.current) {
                          suppressClick.current = false
                          return
                        }
                        setSelectedId(ma.id)
                      }}
                      className={cx(
                        'astra-pop absolute box-border flex touch-none flex-col items-start border px-4 py-3 text-left text-ink transition-[border-color,background-color,scale] duration-150',
                        active ? 'border-signal bg-signal-soft' : 'border-line bg-surface-raised',
                        isSel && !active && 'border-ink',
                        isDrag ? 'z-30 scale-[1.04] cursor-grabbing' : 'z-10 cursor-grab',
                        isOver && 'border-dashed border-signal',
                      )}
                      style={{
                        left: p.x,
                        top: p.y,
                        width: size.w,
                        height: size.h,
                        animationDelay: reduced ? undefined : `${80 + i * 70}ms`,
                      }}
                    >
                      <b className="font-semibold">{roleName[ma.role]}</b>
                      <Spec>{agentOf(s, ma)?.name}</Spec>
                      <span className={cx('text-[13px]', toneTextClass(st.tone))}>
                        <Dot tone={st.tone} className={cx('mr-2 align-middle', active && 'astra-pulse')} />
                        {nodeLine(s, ma)}
                      </span>
                      {isOver ? (
                        <span className="absolute right-2 bottom-1 font-mono text-spec text-signal uppercase">
                          Swap
                        </span>
                      ) : null}
                    </button>
                  )
                })}
              </div>

              <div
                className="absolute top-3 right-3 z-40 flex items-center gap-1 border border-line bg-surface-raised p-1"
                onPointerDown={(e) => e.stopPropagation()}
              >
                <Button
                  variant="text"
                  size="sm"
                  aria-label="Zoom out"
                  disabled={zoom <= ZOOM.min}
                  onClick={() => zoomBy(-ZOOM.step)}
                >
                  −
                </Button>
                <Data className="w-12 text-center text-ink-muted" aria-live="polite">
                  {Math.round(zoom * 100)}%
                </Data>
                <Button
                  variant="text"
                  size="sm"
                  aria-label="Zoom in"
                  disabled={zoom >= ZOOM.max}
                  onClick={() => zoomBy(ZOOM.step)}
                >
                  +
                </Button>
                <Button variant="text" size="sm" onClick={fit}>
                  Fit
                </Button>
                <Button variant="text" size="sm" disabled={!moved} onClick={resetLayout}>
                  Reset layout
                </Button>
              </div>
              <p
                id="astra-map-hint"
                className="pointer-events-none absolute bottom-3 left-3 z-40 m-0 font-mono text-spec text-ink-muted uppercase"
              >
                Drag to arrange · Drop on an agent to swap · Arrow keys nudge · ⌘ + scroll to zoom
              </p>
              <p className="sr-only" aria-live="polite">
                {announce}
              </p>
            </div>
          </div>
        </main>

        <aside className="box-border flex flex-[1_1_320px] flex-col gap-6 border-l border-line bg-surface-alt p-6">
          {selected ? (
            <>
              <div className="flex flex-col gap-2">
                <Eyebrow>Selected agent</Eyebrow>
                <h2 className="m-0 text-title">{roleName[selected.role]}</h2>
                <Spec>
                  {[
                    agentOf(s, selected)?.name,
                    agentOf(s, selected)?.vendor,
                    agentOf(s, selected)?.runsOn === 'server'
                      ? 'Your server'
                      : agentOf(s, selected)?.transport,
                  ]
                    .filter(Boolean)
                    .join(' · ')}
                </Spec>
                <Status tone={agentStatus[selected.status].tone}>
                  {agentStatus[selected.status].label}
                  {selected.steeredBy ? ` · steered by ${personName(s, selected.steeredBy)}` : ''}
                </Status>
              </div>
              <KVList
                dense
                items={[
                  { k: 'Task', v: selected.task ?? '—' },
                  { k: 'Branch', v: <Data>{selected.branch ?? mission.branch}</Data> },
                  { k: 'Started', v: <Data>{hhmm(selected.startedAt) || '—'}</Data> },
                ]}
              />
              {selected.latest.length > 0 ? (
                <div className="flex flex-col gap-2">
                  <Eyebrow>Latest</Eyebrow>
                  <LogWell lines={selected.latest} />
                </div>
              ) : null}
              <div className="flex flex-wrap gap-3">
                <Link
                  to="/missions/$missionId"
                  params={{ missionId }}
                  className={buttonClass('primary', 'md')}
                >
                  Open thread
                </Link>
                {canSteer(s, s.me, missionId) &&
                (selected.status === 'working' || selected.statusDetail === 'paused') ? (
                  <Button
                    variant="outline"
                    onClick={() =>
                      agentCommand(client, selected.id, selected.status === 'working' ? 'pause' : 'resume')
                    }
                  >
                    {selected.status === 'working' ? 'Pause' : 'Resume'}
                  </Button>
                ) : null}
              </div>
            </>
          ) : (
            <p className="m-0 text-ink-muted">This mission has no agents.</p>
          )}
        </aside>
      </div>
    </Page>
  )
}
