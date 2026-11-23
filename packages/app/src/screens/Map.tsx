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
import { useEffect, useState } from 'react'
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

const W = 880
const H = 640
const NODE = { w: 200, h: 96 }
const CENTER = { w: 240, h: 104 }
const ORBIT = { rx: 276, ry: 208 }

interface Placed {
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

/** Point where the segment from a box's centre towards (tx, ty) leaves the box. */
function exitPoint(cx: number, cy: number, w: number, h: number, tx: number, ty: number) {
  const dx = tx - cx
  const dy = ty - cy
  const sx = dx === 0 ? Number.POSITIVE_INFINITY : w / 2 / Math.abs(dx)
  const sy = dy === 0 ? Number.POSITIVE_INFINITY : h / 2 / Math.abs(dy)
  const s = Math.min(sx, sy)
  return { x: cx + dx * s, y: cy + dy * s }
}

const edgeStyle = (ma: MissionAgent) => {
  if (ma.status === 'working') return { color: 'var(--signal)', dashed: false, label: 'working now' }
  if (ma.status === 'needsYou')
    return { color: 'var(--warning)', dashed: true, label: 'waiting for your approval' }
  if (ma.status === 'queued') return { color: 'var(--line-strong)', dashed: true, label: 'queued' }
  if (ma.status === 'offline') return { color: 'var(--line-strong)', dashed: true, label: 'offline' }
  return { color: 'var(--line-strong)', dashed: false, label: ma.statusDetail ?? 'finished' }
}

function nodeLine(s: ReturnType<typeof useAstraState>, ma: MissionAgent): string {
  const st = agentStatus[ma.status]
  if (ma.status === 'working' && ma.steeredBy) return `${st.label} · ${personName(s, ma.steeredBy)} steering`
  if (ma.statusDetail) return `${st.label} · ${ma.statusDetail}`
  return st.label
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

  if (!mission) return <MissionNotFound />
  const placed = placeAgents(agents)
  const selected =
    (selectedId && s.missionAgents[selectedId]) || agents.find((a) => a.status === 'working') || agents[0]
  const counts = {
    working: agents.filter((a) => a.status === 'working').length,
    needs: agents.filter((a) => a.status === 'needsYou').length,
  }
  const cxm = W / 2
  const cym = H / 2

  return (
    <Page
      header={
        <AppHeader
          repoSelect
          nav="map"
          people={members(s, missionId).map((m) => m.person)}
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
          <div className="overflow-x-auto px-8 pb-8">
            <div className="relative mx-auto" style={{ width: W, height: H }}>
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
                className="absolute inset-0"
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
                {placed.map((p) => {
                  const ncx = p.x + NODE.w / 2
                  const ncy = p.y + NODE.h / 2
                  const a = exitPoint(cxm, cym, CENTER.w, CENTER.h, ncx, ncy)
                  const b = exitPoint(ncx, ncy, NODE.w + 16, NODE.h + 16, cxm, cym)
                  const e = edgeStyle(p.ma)
                  return (
                    <line
                      key={p.ma.id}
                      x1={a.x}
                      y1={a.y}
                      x2={b.x}
                      y2={b.y}
                      stroke={e.color}
                      strokeWidth="1.5"
                      strokeLinecap="round"
                      strokeDasharray={e.dashed ? '6 6' : undefined}
                      markerEnd="url(#astra-arrow)"
                    >
                      <title>{`${roleName[p.ma.role]}: ${e.label}`}</title>
                    </line>
                  )
                })}
              </svg>
              <div
                className="absolute box-border flex flex-col border border-ink bg-surface px-4 py-3"
                style={{
                  left: cxm - CENTER.w / 2,
                  top: cym - CENTER.h / 2,
                  width: CENTER.w,
                  height: CENTER.h,
                }}
              >
                <Spec>Mission {missionNo(mission)}</Spec>
                <b className="font-semibold">{mission.title}</b>
              </div>
              {placed.map((p) => {
                const st = agentStatus[p.ma.status]
                const active = p.ma.status === 'working'
                const isSel = selected?.id === p.ma.id
                return (
                  <button
                    key={p.ma.id}
                    type="button"
                    aria-pressed={isSel}
                    onClick={() => setSelectedId(p.ma.id)}
                    className={cx(
                      'absolute box-border flex flex-col items-start border px-4 py-3 text-left text-ink',
                      active ? 'border-signal bg-signal-soft' : 'border-line bg-surface-raised',
                      isSel && !active && 'border-ink',
                    )}
                    style={{ left: p.x, top: p.y, width: NODE.w, height: NODE.h }}
                  >
                    <b className="font-semibold">{roleName[p.ma.role]}</b>
                    <Spec>{agentOf(s, p.ma)?.name}</Spec>
                    <span className={cx('text-[13px]', toneTextClass(st.tone))}>
                      <Dot tone={st.tone} className="mr-2 align-middle" />
                      {nodeLine(s, p.ma)}
                    </span>
                  </button>
                )
              })}
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
