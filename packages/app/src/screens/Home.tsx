import { createMission, type MissionMode } from '@astra/client'
import { Button, Eyebrow, inlineLinkClass, SectionHeading, Spec, SpecStatus } from '@astra/ui'
import { Link, useNavigate } from '@tanstack/react-router'
import { useState } from 'react'
import { useAstraState, useClient, useUi } from '../context'
import { agentOf, connectedAgents, missionNo, missionStatus, orderedMissions, roleName } from '../select'
import { AppHeader, Page, ShareButton } from '../shell/AppHeader'
import { defaultTeam } from '../team'

const rowLink = 'flex flex-col gap-0.5 border-b border-line-subtle py-3 text-ink no-underline'
const selectCls = 'h-8 rounded-sm border border-line-strong bg-surface-raised px-2 text-ink'

export function useNeedsYou() {
  const s = useAstraState()
  const approvals = Object.values(s.approvals)
    .filter((a) => a.state === 'pending')
    .map((a) => {
      const m = s.missions[a.missionId]
      const ma = s.missionAgents[a.missionAgentId]
      const agent = agentOf(s, ma)
      const task = Object.values(s.tasks).find((t) => t.approvalId === a.id)
      return {
        key: a.id,
        title: task?.title ?? a.prompt,
        meta: `${m ? missionNo(m) : ''} · ${ma ? roleName[ma.role].toUpperCase() : ''} · ${agent?.name.toUpperCase() ?? ''}`,
        missionId: a.missionId,
        kind: 'thread' as const,
      }
    })
  const picks = Object.values(s.missions)
    .filter((m) => m.status === 'needsYou' && Object.values(s.results).some((r) => r.missionId === m.id))
    .map((m) => {
      const results = Object.values(s.results).filter((r) => r.missionId === m.id)
      const names = results
        .map((r) => agentOf(s, s.missionAgents[r.missionAgentId])?.name.toUpperCase())
        .filter(Boolean)
      return {
        key: `pick-${m.id}`,
        title: `Pick one of ${results.length === 2 ? 'two' : results.length} results`,
        meta: `${missionNo(m)} · ${names.join(' AND ')} RACED`,
        missionId: m.id,
        kind: 'review' as const,
      }
    })
  return [...approvals, ...picks]
}

/** 4 · Home. */
export function Home() {
  const s = useAstraState()
  const client = useClient()
  const ui = useUi()
  const navigate = useNavigate()
  const [goal, setGoal] = useState('')
  const [mode, setMode] = useState<MissionMode>('split')
  const [who, setWho] = useState('all')
  const [branch, setBranch] = useState('')
  const repo = s.repositories[ui.repoId]
  const me = s.people[s.me]
  const needs = useNeedsYou()
  const missions = orderedMissions(s, ui.repoId).slice(0, 3)
  const agents = connectedAgents(s)
  const team = defaultTeam(s, mode)

  const start = async () => {
    if (!goal.trim() || !repo) {
      document.querySelector<HTMLTextAreaElement>('textarea[aria-label="Goal"]')?.focus()
      return
    }
    const chosen = who === 'all' ? team : team.filter((t) => t.role === who)
    const id = await createMission(client, {
      goal: goal.trim(),
      mode,
      repoId: repo.id,
      baseBranch: branch || repo.defaultBranch,
      agents: chosen.map((t) => ({ agentId: t.agent.id, role: t.role, enabled: true })),
    })
    ui.setLastMissionId(id)
    void navigate({ to: '/missions/$missionId', params: { missionId: id } })
  }

  return (
    <Page
      header={
        <AppHeader
          repoSelect
          nav={null}
          people={Object.values(s.people)}
          actions={<ShareButton missionId={ui.lastMissionId} />}
        />
      }
    >
      <main className="mx-auto box-border flex w-full max-w-[1056px] flex-col px-6 pt-24 pb-12 sm:px-12">
        <Eyebrow>Welcome back, {me?.name ?? ''}</Eyebrow>
        <h1 className="m-0 mt-6 text-display-md uppercase">Start a mission.</h1>
        <p className="m-0 mt-6 max-w-[640px] text-lead text-ink-muted">
          Give one goal to every agent you have connected, from any vendor or harness.
        </p>

        <form
          className="mt-8 flex flex-col rounded-sm border border-line-strong bg-surface-raised"
          onSubmit={(e) => {
            e.preventDefault()
            void start()
          }}
        >
          <div className="flex flex-wrap items-center gap-x-3 gap-y-2 border-b border-line px-4 py-3">
            <label htmlFor="home-repo" className="text-kicker uppercase text-ink-muted">
              Repository
            </label>
            <select
              id="home-repo"
              value={ui.repoId}
              onChange={(e) => ui.setRepoId(e.target.value)}
              className={`${selectCls} font-mono text-data`}
            >
              {Object.values(s.repositories).map((r) => (
                <option key={r.id} value={r.id}>
                  {r.name}
                </option>
              ))}
            </select>
            <label htmlFor="home-branch" className="ml-3 text-kicker uppercase text-ink-muted">
              Branch
            </label>
            <select
              id="home-branch"
              value={branch || repo?.defaultBranch}
              onChange={(e) => setBranch(e.target.value)}
              className={`${selectCls} font-mono text-data`}
            >
              {[...new Set([repo?.defaultBranch ?? 'main', 'develop'])].map((b) => (
                <option key={b}>{b}</option>
              ))}
            </select>
            <div className="grow" />
            <Link to="/setup/repo" className={inlineLinkClass}>
              + Add a repository
            </Link>
          </div>
          <textarea
            aria-label="Goal"
            rows={3}
            value={goal}
            onChange={(e) => setGoal(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) void start()
            }}
            placeholder="Describe the goal. For example: add rate limiting to the gateway and keep the public API unchanged."
            className="box-border min-h-26 resize-y border-0 bg-transparent p-4 text-body text-ink -outline-offset-2"
          />
          <div className="flex flex-wrap items-center gap-x-4 gap-y-3 border-t border-line px-4 py-3">
            <select
              aria-label="How the agents work"
              value={mode}
              onChange={(e) => setMode(e.target.value as MissionMode)}
              className={selectCls}
            >
              <option value="split">Split the work</option>
              <option value="race">Race</option>
            </select>
            <select
              aria-label="Which agents"
              value={who}
              onChange={(e) => setWho(e.target.value)}
              className={selectCls}
            >
              <option value="all">All {team.length} agents</option>
              {[...new Set(team.map((t) => t.role))].map((r) => (
                <option key={r} value={r}>
                  {roleName[r]}
                </option>
              ))}
            </select>
            <Link to="/missions/new" search={{ goal: goal || undefined }} className={inlineLinkClass}>
              More options
            </Link>
            <div className="grow" />
            <Button type="submit" className="px-6">
              Start mission →
            </Button>
          </div>
        </form>

        <div className="mt-16 flex flex-wrap gap-x-12 gap-y-8">
          <section className="flex min-w-0 flex-[1_1_260px] flex-col gap-3">
            <SectionHeading tone={needs.length > 0 ? 'needsYou' : 'idle'}>
              Needs you · {needs.length}
            </SectionHeading>
            <div className="flex flex-col border-t border-line">
              {needs.map((n) =>
                n.kind === 'review' ? (
                  <Link
                    key={n.key}
                    to="/missions/$missionId/review"
                    params={{ missionId: n.missionId }}
                    className={rowLink}
                  >
                    <span>{n.title}</span>
                    <Spec>{n.meta}</Spec>
                  </Link>
                ) : (
                  <Link
                    key={n.key}
                    to="/missions/$missionId"
                    params={{ missionId: n.missionId }}
                    className={rowLink}
                  >
                    <span>{n.title}</span>
                    <Spec>{n.meta}</Spec>
                  </Link>
                ),
              )}
              {needs.length === 0 ? (
                <p className="m-0 py-3 text-ink-muted">Nothing is waiting on you.</p>
              ) : null}
            </div>
          </section>

          <section className="flex min-w-0 flex-[1_1_260px] flex-col gap-3">
            <SectionHeading>Missions</SectionHeading>
            <div className="flex flex-col border-t border-line">
              {missions.map((m) => {
                const st = missionStatus[m.status]
                const to =
                  m.status === 'needsYou'
                    ? '/missions/$missionId/review'
                    : m.status === 'merged'
                      ? '/missions/$missionId/merged'
                      : '/missions/$missionId'
                return (
                  <Link key={m.id} to={to} params={{ missionId: m.id }} className={rowLink}>
                    <span>{m.title}</span>
                    <SpecStatus tone={st.tone}>
                      {missionNo(m)} · {st.label}
                    </SpecStatus>
                  </Link>
                )
              })}
            </div>
            <Link to="/board" className={inlineLinkClass}>
              All missions →
            </Link>
          </section>

          <section className="flex min-w-0 flex-[1_1_260px] flex-col gap-3">
            <SectionHeading>Agents · {agents.length} connected</SectionHeading>
            <ul className="m-0 list-none border-t border-line p-0">
              {agents.map((a) => (
                <li
                  key={a.id}
                  className="flex min-h-10 items-center justify-between gap-3 border-b border-line-subtle"
                >
                  <span>{a.name}</span>
                  <Spec>{a.vendor}</Spec>
                </li>
              ))}
            </ul>
            <Link to="/setup/agents" className={inlineLinkClass}>
              + Connect an agent
            </Link>
          </section>
        </div>
      </main>
    </Page>
  )
}
