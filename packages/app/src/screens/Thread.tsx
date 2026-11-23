import { sendMessage } from '@astra/client'
import {
  Button,
  cx,
  Data,
  Eyebrow,
  formatDiffStats,
  inlineLinkClass,
  Select,
  Spec,
  SpecStatus,
  Status,
} from '@astra/ui'
import { Link } from '@tanstack/react-router'
import { useEffect, useRef, useState } from 'react'
import { useAstraState, useClient, useUi } from '../context'
import {
  agentOf,
  agentStatus,
  canSteer,
  members,
  missionAgents,
  missionNo,
  missionRoute,
  missionStatus,
  orderedMissions,
  roleName,
} from '../select'
import { latestPerAgent, MessageItem, messagesOf } from '../shared/thread'
import { AppHeader, NewMissionButton, Page, ShareButton } from '../shell/AppHeader'
import { MissionNotFound, useMissionPresence } from './common'

/** 6 · Thread: the mission conversation between people and agents. */
export function Thread({ missionId }: { missionId: string }) {
  const s = useAstraState()
  const client = useClient()
  const ui = useUi()
  const mission = s.missions[missionId]
  const [to, setTo] = useState<string>('all')
  const [draft, setDraft] = useState('')
  const listEnd = useRef<HTMLDivElement>(null)
  const messages = messagesOf(s, missionId)
  useMissionPresence(missionId, 'Watching the thread')

  const known = Boolean(mission)
  useEffect(() => {
    if (known) ui.setLastMissionId(missionId)
  }, [known, missionId, ui])

  // biome-ignore lint/correctness/useExhaustiveDependencies: scroll to the newest message whenever one arrives
  useEffect(() => {
    listEnd.current?.scrollIntoView?.({ block: 'end' })
  }, [messages.length])

  if (!mission) return <MissionNotFound />
  const agents = missionAgents(s, missionId)
  const people = members(s, missionId)
  const latest = latestPerAgent(messages)
  const mayWrite = canSteer(s, s.me, missionId)

  const send = async () => {
    if (!draft.trim()) return
    await sendMessage(client, missionId, to === 'all' ? null : to, draft)
    setDraft('')
  }

  return (
    <Page
      header={
        <AppHeader
          repoSelect
          nav="thread"
          people={people.map((p) => p.person)}
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
        <aside className="box-border flex flex-[1_1_240px] flex-col gap-8 border-r border-line bg-surface-alt px-3 py-6">
          <section className="flex flex-col gap-2">
            <Eyebrow as="h2" className="px-3">
              Missions
            </Eyebrow>
            {orderedMissions(s).map((m) => {
              const st = missionStatus[m.status]
              const current = m.id === missionId
              return (
                <Link
                  key={m.id}
                  to={current ? '/missions/$missionId' : missionRoute(s, m)}
                  params={{ missionId: m.id }}
                  aria-current={current ? 'true' : undefined}
                  className={cx(
                    'flex flex-col gap-0.5 px-3 py-2.5 text-ink no-underline',
                    current && 'bg-signal-soft',
                  )}
                >
                  <span>{m.title}</span>
                  <SpecStatus tone={st.tone}>
                    {missionNo(m)} · {st.label}
                  </SpecStatus>
                </Link>
              )
            })}
          </section>
          <section className="flex flex-col gap-2">
            <Eyebrow as="h2" className="px-3">
              Agents
            </Eyebrow>
            <ul className="m-0 list-none p-0">
              {agents.map((ma) => {
                const st = agentStatus[ma.status]
                return (
                  <li key={ma.id} className="flex items-center gap-3 px-3 py-2">
                    <span className="flex grow flex-col">
                      <span>{roleName[ma.role]}</span>
                      <Spec>{agentOf(s, ma)?.name}</Spec>
                    </span>
                    <Status tone={st.tone}>{st.label}</Status>
                  </li>
                )
              })}
            </ul>
            <Link to="/setup/agents" className={cx(inlineLinkClass, 'px-3 py-2')}>
              + Connect an agent
            </Link>
          </section>
        </aside>

        <main className="flex min-w-0 flex-[999_1_560px] flex-col">
          <div className="flex flex-wrap items-end justify-between gap-x-8 gap-y-4 border-b border-line px-8 py-6">
            <div className="flex flex-col gap-2">
              <Eyebrow>Mission {missionNo(mission)}</Eyebrow>
              <h1 className="m-0 text-title">{mission.title}</h1>
              <Data className="text-ink-muted">
                {mission.branch} · {agents.length} agents · {people.length} people
              </Data>
            </div>
            {mission.stats ? (
              <Data className="text-ink-muted">{formatDiffStats(mission.stats, 'files changed')}</Data>
            ) : null}
          </div>

          <ol aria-label="Thread" className="m-0 max-w-[880px] grow list-none px-8 py-0">
            {messages.map((m, i) => (
              <MessageItem
                key={m.id}
                message={m}
                showStatus={latest.has(m.id)}
                last={i === messages.length - 1}
              />
            ))}
            <div ref={listEnd} />
          </ol>

          <form
            className="flex flex-wrap items-center gap-3 border-t border-line px-8 pt-4 pb-6"
            onSubmit={(e) => {
              e.preventDefault()
              void send()
            }}
          >
            <label htmlFor="thread-to" className="text-kicker uppercase text-ink-muted">
              To
            </label>
            <Select id="thread-to" value={to} onChange={(e) => setTo(e.target.value)} disabled={!mayWrite}>
              <option value="all">All agents</option>
              {agents.map((ma) => (
                <option key={ma.id} value={ma.id}>
                  {roleName[ma.role]}
                </option>
              ))}
            </Select>
            <input
              type="text"
              aria-label="Message"
              value={draft}
              onChange={(e) => setDraft(e.target.value)}
              disabled={!mayWrite}
              placeholder={mayWrite ? 'Message the agents, or steer one' : 'You can watch this mission'}
              className="box-border h-10 min-w-0 flex-[1_1_320px] rounded-sm border border-line-strong bg-surface-raised px-3 text-ink"
            />
            <Button type="submit" disabled={!mayWrite}>
              Send
            </Button>
          </form>
        </main>
      </div>
    </Page>
  )
}
