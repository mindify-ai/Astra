import type { Task, TaskColumn } from '@astra/client'
import { Avatar, Card, cx, Markdown, SectionHeading, Spec } from '@astra/ui'
import { Link } from '@tanstack/react-router'
import { useAstraState, useUi } from '../context'
import {
  agentOf,
  archivedMissions,
  columnStatus,
  connectedAgents,
  missionNo,
  orderedMissions,
  personName,
  roleName,
} from '../select'
import { MissionActions } from '../shared/missionActions'
import { ApprovalActions } from '../shared/thread'
import { AppHeader, NewMissionButton, Page, ShareButton } from '../shell/AppHeader'

const columns: TaskColumn[] = ['queued', 'working', 'needsYou', 'done']

function TaskCard({ task }: { task: Task }) {
  const s = useAstraState()
  const mission = s.missions[task.missionId]
  const ma = task.missionAgentId ? s.missionAgents[task.missionAgentId] : undefined
  const agent = agentOf(s, ma)
  const approval = task.approvalId ? s.approvals[task.approvalId] : undefined
  const steering = task.column === 'working' && ma?.steeredBy ? s.people[ma.steeredBy] : undefined
  const comparable =
    task.column === 'needsYou' &&
    !approval &&
    Object.values(s.results).some((r) => r.missionId === task.missionId)

  return (
    <Card emphasis={task.column === 'done' ? 'quiet' : 'default'}>
      {mission ? (
        <Spec>
          {missionNo(mission)} · {mission.shortName}
        </Spec>
      ) : null}
      <h3 className="m-0 text-body-sm font-medium">{task.title}</h3>
      {ma ? (
        <span className="text-ink-muted">
          {roleName[ma.role]}
          <Spec className="ml-1 text-inherit">{agent?.name}</Spec>
          {task.mergedBy ? ` · merged by ${personName(s, task.mergedBy)}` : ''}
        </span>
      ) : task.note ? (
        <Markdown className="text-ink-muted">{task.note}</Markdown>
      ) : null}
      {steering ? (
        <div className="flex items-center gap-2 border-t border-line pt-2 text-[13px] text-ink-muted">
          <Avatar initials={steering.initials} size={20} />
          {steering.name} is steering
        </div>
      ) : null}
      {approval && approval.state === 'pending' ? (
        <div className="border-t border-line pt-0">
          <ApprovalActions approval={approval} variant="desktop" />
        </div>
      ) : null}
      {comparable ? (
        <Link
          to="/missions/$missionId/review"
          params={{ missionId: task.missionId }}
          className="pt-2 text-label uppercase text-ink no-underline"
        >
          Compare →
        </Link>
      ) : null}
    </Card>
  )
}

/** Board: every task across the repository's missions, by state. */
export function Board() {
  const s = useAstraState()
  const ui = useUi()
  const missions = orderedMissions(s, ui.repoId)
  const archived = archivedMissions(s, ui.repoId)
  const ids = new Set(missions.map((m) => m.id))
  const tasks = Object.values(s.tasks)
    .filter((t) => ids.has(t.missionId))
    .sort(
      (a, b) =>
        a.order - b.order || (s.missions[b.missionId]?.number ?? 0) - (s.missions[a.missionId]?.number ?? 0),
    )

  return (
    <Page
      header={
        <AppHeader
          repoSelect
          nav="board"
          people={Object.values(s.people)}
          actions={
            <>
              <ShareButton missionId={ui.lastMissionId} />
              <NewMissionButton />
            </>
          }
        />
      }
    >
      <div className="flex flex-wrap items-end justify-between gap-x-8 gap-y-4 border-b border-line px-8 py-6">
        <div className="flex flex-col gap-2">
          <span className="text-kicker uppercase text-ink-muted">Board</span>
          <h1 className="m-0 text-title">All missions</h1>
        </div>
        <Spec>
          {missions.length} missions · {tasks.length} tasks · {connectedAgents(s).length} agents
        </Spec>
      </div>
      <main className="flex grow flex-wrap">
        {columns.map((col, i) => {
          const list = tasks.filter((t) => t.column === col)
          const st = columnStatus[col]
          return (
            <section
              key={col}
              aria-label={st.label}
              className={cx(
                'flex flex-[1_1_260px] flex-col gap-3 p-6',
                i < columns.length - 1 && 'border-r border-line',
              )}
            >
              <SectionHeading tone={st.tone} className="mb-1">
                {st.label} · {list.length}
              </SectionHeading>
              {list.map((t) => (
                <TaskCard key={t.id} task={t} />
              ))}
            </section>
          )
        })}
      </main>
      {archived.length > 0 ? (
        <section
          aria-label="Archived missions"
          className="flex flex-col gap-2 border-t border-line px-8 py-6"
        >
          <SectionHeading tone="idle" className="mb-1">
            Archived · {archived.length}
          </SectionHeading>
          <ul className="m-0 flex list-none flex-col border-t border-line p-0">
            {archived.map((m) => (
              <li
                key={m.id}
                className="flex min-h-11 flex-wrap items-center gap-x-4 gap-y-1 border-b border-line-subtle py-1"
              >
                <Link
                  to="/missions/$missionId"
                  params={{ missionId: m.id }}
                  className="flex-[1_1_280px] text-ink no-underline hover:underline"
                >
                  {m.title}
                </Link>
                <Spec>{missionNo(m)}</Spec>
                <MissionActions missionId={m.id} compact />
              </li>
            ))}
          </ul>
        </section>
      ) : null}
    </Page>
  )
}
