import { createMission, type MissionMode, sendMessage } from '@astra/client'
import {
  ArrowUpIcon,
  Avatar,
  Badge,
  Button,
  buttonClass,
  cx,
  Data,
  DiffView,
  Eyebrow,
  formatDiffStats,
  Markdown,
  SectionHeading,
  Segmented,
  Spec,
  Status,
  UnderlineTabs,
} from '@astra/ui'
import { useRouter } from '@tanstack/react-router'
import { useEffect, useState } from 'react'
import { useAstraState, useClient, useHost, usePlatform, useUi } from '../context'
import { useMissionPresence } from '../screens/common'
import { GitContent } from '../screens/Git'
import { useNeedsYou } from '../screens/Home'
import { checksStatus, resultsOf, useVerdict } from '../screens/Review'
import {
  agentOf,
  canApprove,
  canSteer,
  connectedAgents,
  hhmm,
  members,
  missionNo,
  missionStatus,
  orderedMissions,
  roleName,
} from '../select'
import { MergeNotice, useResultMerge } from '../shared/git'
import { latestPerAgent, MessageItem, messagesOf } from '../shared/thread'
import { defaultTeam } from '../team'
import { BrandBar, MissionTopBar, MobileFrame, MobileTabs, useLinkRenderer } from './MobileShell'

const rowLink =
  'flex min-h-14 flex-col justify-center gap-0.5 border-b border-line-subtle py-1 text-ink no-underline'

export function MobileHome() {
  const s = useAstraState()
  const client = useClient()
  const ui = useUi()
  const router = useRouter()
  const render = useLinkRenderer()
  const [goal, setGoal] = useState('')
  const [mode, setMode] = useState<MissionMode>('split')
  const needs = useNeedsYou()
  const agents = connectedAgents(s)
  const me = s.people[s.me]

  const start = async () => {
    const repo = s.repositories[ui.repoId]
    if (!goal.trim() || !repo) {
      document.querySelector<HTMLTextAreaElement>('textarea[aria-label="Goal"]')?.focus()
      return
    }
    const id = await createMission(client, {
      goal: goal.trim(),
      mode,
      repoId: repo.id,
      baseBranch: repo.defaultBranch,
      agents: defaultTeam(s, mode).map((t) => ({ agentId: t.agent.id, role: t.role, enabled: true })),
    })
    ui.setLastMissionId(id)
    router.history.push(`/missions/${id}`)
  }

  return (
    <MobileFrame
      top={<BrandBar />}
      bottom={<MobileTabs active="home" />}
      className="flex flex-col gap-5 px-4 pt-2 pb-6"
    >
      <select
        aria-label="Repository"
        value={ui.repoId}
        onChange={(e) => ui.setRepoId(e.target.value)}
        className="box-border h-12 w-full rounded-sm border border-line-strong bg-surface-raised px-3 font-mono text-data text-ink"
      >
        {Object.values(s.repositories).map((r) => (
          <option key={r.id} value={r.id}>
            {r.name}
          </option>
        ))}
      </select>
      <div className="flex flex-col gap-3">
        <Eyebrow>Welcome back, {me?.name}</Eyebrow>
        <h1 className="m-0 text-display-sm uppercase">Start a mission.</h1>
      </div>
      <form
        className="flex flex-col rounded-sm border border-line-strong bg-surface-raised"
        onSubmit={(e) => {
          e.preventDefault()
          void start()
        }}
      >
        <textarea
          aria-label="Goal"
          value={goal}
          onChange={(e) => setGoal(e.target.value)}
          placeholder="Describe the goal for your agents."
          className="box-border min-h-24 resize-none border-0 bg-transparent p-3 text-[16px] leading-6 text-ink -outline-offset-2"
        />
        <div className="flex items-center gap-2 border-t border-line p-2">
          <select
            aria-label="How the agents work"
            value={mode}
            onChange={(e) => setMode(e.target.value as MissionMode)}
            className="h-11 rounded-sm border border-line-strong bg-surface-raised px-2 text-ink"
          >
            <option value="split">Split the work</option>
            <option value="race">Race</option>
          </select>
          <div className="grow" />
          <Button type="submit" size="touchSm">
            Start →
          </Button>
        </div>
      </form>
      <section className="flex flex-col gap-3">
        <SectionHeading tone={needs.length ? 'needsYou' : 'idle'}>Needs you · {needs.length}</SectionHeading>
        <div className="border-t border-line">
          {needs.map((n) =>
            render({
              to: n.kind === 'review' ? `/missions/${n.missionId}/review` : `/missions/${n.missionId}`,
              className: rowLink,
              children: (
                <>
                  <span>{n.title}</span>
                  <Spec>{n.meta}</Spec>
                </>
              ),
            }),
          )}
        </div>
      </section>
      <section className="flex flex-col gap-3">
        <Eyebrow as="h2">Agents · {agents.length} connected</Eyebrow>
        <Spec as="p" className="m-0">
          {agents.map((a) => a.name).join(' · ')}
        </Spec>
      </section>
    </MobileFrame>
  )
}

export function MobileThread({ missionId }: { missionId: string }) {
  const s = useAstraState()
  const client = useClient()
  const ui = useUi()
  const { mobileVariant } = usePlatform()
  const [draft, setDraft] = useState('')
  const mission = s.missions[missionId]
  useMissionPresence(missionId, 'Watching the thread')
  useEffect(() => {
    if (mission) ui.setLastMissionId(missionId)
  }, [missionId, mission, ui])
  if (!mission) return <MobileMissing />
  const messages = messagesOf(s, missionId)
  const latest = latestPerAgent(messages)
  const people = members(s, missionId).map((m) => m.person)
  const st = missionStatus[mission.status]
  const mayWrite = canSteer(s, s.me, missionId)
  const ios = mobileVariant === 'ios'

  return (
    <MobileFrame
      top={<MissionTopBar title={`Mission ${missionNo(mission)}`} people={people} />}
      bottom={
        <form
          className={cx(
            'flex gap-2 border-t border-line px-4 pt-3',
            ios ? 'pb-[max(env(safe-area-inset-bottom),12px)]' : 'pb-[max(env(safe-area-inset-bottom),12px)]',
          )}
          onSubmit={(e) => {
            e.preventDefault()
            void sendMessage(client, missionId, null, draft).then(() => setDraft(''))
          }}
        >
          <input
            aria-label="Message"
            value={draft}
            onChange={(e) => setDraft(e.target.value)}
            disabled={!mayWrite}
            placeholder={mayWrite ? 'Message the agents' : 'You can watch this mission'}
            className="box-border h-12 min-w-0 grow rounded-sm border border-line-strong bg-surface-raised px-3 text-[16px] text-ink"
          />
          <button
            type="submit"
            aria-label="Send"
            disabled={!mayWrite}
            className="flex size-12 shrink-0 items-center justify-center border-0 bg-invert text-on-invert disabled:opacity-50"
          >
            <ArrowUpIcon />
          </button>
        </form>
      }
    >
      <div className="flex flex-col gap-2 border-b border-line px-4 pt-2 pb-4">
        <h1 className="m-0 text-title">{mission.title}</h1>
        <div className="flex flex-wrap items-center gap-x-4 gap-y-1">
          <Status tone={st.tone}>{st.label}</Status>
          <Data className="text-ink-muted">{mission.branch}</Data>
        </div>
      </div>
      <ol aria-label="Thread" className="m-0 list-none px-4 py-0">
        {messages.map((m, i) => (
          <MessageItem
            key={m.id}
            message={m}
            showStatus={latest.has(m.id)}
            variant={mobileVariant}
            last={i === messages.length - 1}
          />
        ))}
      </ol>
    </MobileFrame>
  )
}

export function MobileReview({ missionId }: { missionId: string }) {
  const s = useAstraState()
  const render = useLinkRenderer()
  const { mobileVariant } = usePlatform()
  const mission = s.missions[missionId]
  const results = resultsOf(s, missionId)
  const verdict = useVerdict(missionId)
  const [pick, setPick] = useState<string | null>(null)
  useMissionPresence(missionId, 'Comparing results')
  const current = results.find((r) => r.id === pick) ?? results.find((r) => r.reviewerPick) ?? results[0]
  const mergeAction = useResultMerge(current)
  if (!mission) return <MobileMissing />
  const ma = current ? s.missionAgents[current.missionAgentId] : undefined
  const ios = mobileVariant === 'ios'
  const options = results.map((r) => ({ value: r.id, label: `Result ${r.label}` }))
  const allowed = canApprove(s, s.me, missionId, current?.missionAgentId) && mission.status !== 'merged'

  return (
    <MobileFrame
      top={
        <MissionTopBar
          title={`Mission ${missionNo(mission)}`}
          people={members(s, missionId).map((m) => m.person)}
        />
      }
      bottom={
        current ? (
          <div
            className={cx(
              'flex items-center gap-4 border-t border-line px-4 pt-3 pb-[max(env(safe-area-inset-bottom),12px)]',
              !ios && 'justify-end',
            )}
          >
            {!ios
              ? render({
                  to: `/missions/${missionId}`,
                  className: buttonClass('text', 'touch', 'px-0'),
                  children: 'Open thread',
                })
              : null}
            <Button
              size="touch"
              className={ios ? 'w-full' : 'px-6'}
              onClick={mergeAction.doMerge}
              disabled={!allowed || mergeAction.busy}
            >
              {mission.status === 'merged' ? 'Merged' : `Merge ${current.label}`}
            </Button>
          </div>
        ) : null
      }
      className="flex flex-col gap-4 px-4 pt-2 pb-4"
    >
      <div className="flex flex-col gap-2">
        <Eyebrow>
          {mission.mode === 'race' ? 'Race' : 'Split'} · {results.length} results
        </Eyebrow>
        <h1 className="m-0 text-title">{mission.title}</h1>
      </div>
      <MergeNotice action={mergeAction} />
      {verdict ? (
        <section
          aria-label="Reviewer"
          className="flex flex-col gap-1 border border-line bg-surface-raised p-3"
        >
          <div className="flex flex-wrap items-baseline gap-2">
            <b className="font-semibold">Reviewer</b>
            <Spec>
              {verdict.agent?.name} · {hhmm(verdict.message.at)}
            </Spec>
          </div>
          <Markdown>{verdict.message.body}</Markdown>
        </section>
      ) : null}
      {current ? (
        <>
          {ios ? (
            <Segmented options={options} value={current.id} onChange={setPick} />
          ) : (
            <UnderlineTabs options={options} value={current.id} onChange={setPick} />
          )}
          <div className="flex flex-col gap-3">
            <div className="flex flex-wrap items-center gap-2">
              <span>{ma ? roleName[ma.role] : ''}</span>
              <Spec>{agentOf(s, ma)?.name}</Spec>
              {current.reviewerPick ? <Badge>Reviewer's pick</Badge> : null}
            </div>
            <div className="flex flex-wrap items-center gap-x-4 gap-y-1">
              <Data className="text-ink-muted">{formatDiffStats(current.stats)}</Data>
              {checksStatus(current)}
            </div>
            <DiffView files={current.diff} compact />
          </div>
        </>
      ) : (
        <p className="m-0 text-ink-muted">Results appear here when the agents finish.</p>
      )}
    </MobileFrame>
  )
}

export function MobileAgents() {
  const s = useAstraState()
  return (
    <MobileFrame
      top={<BrandBar />}
      bottom={<MobileTabs active="agents" />}
      className="flex flex-col gap-4 px-4 pt-2 pb-6"
    >
      <h1 className="m-0 text-display-sm uppercase">Agents</h1>
      <ul className="m-0 list-none border-t border-line p-0">
        {Object.values(s.agents).map((a) => (
          <li key={a.id} className="flex min-h-14 items-center gap-3 border-b border-line-subtle">
            <span className="flex grow flex-col">
              <span>{a.name}</span>
              <Spec>{a.vendor}</Spec>
            </span>
            {a.status === 'connected' ? (
              <Status tone="success">Connected</Status>
            ) : a.status === 'settingUp' ? (
              <Status tone="working">Setting up</Status>
            ) : (
              <Status tone="idle">Not connected</Status>
            )}
          </li>
        ))}
      </ul>
      <p className="m-0 text-ink-muted">Connect new agents from the desktop app, where they run.</p>
    </MobileFrame>
  )
}

export function MobilePeople() {
  const s = useAstraState()
  const host = useHost()
  const missions = orderedMissions(s).slice(0, 5)
  return (
    <MobileFrame
      top={<BrandBar />}
      bottom={<MobileTabs active="people" />}
      className="flex flex-col gap-6 px-4 pt-2 pb-6"
    >
      <h1 className="m-0 text-display-sm uppercase">People</h1>
      {missions.map((m) => (
        <section key={m.id} className="flex flex-col gap-2">
          <Eyebrow as="h2">
            {missionNo(m)} · {m.shortName}
          </Eyebrow>
          <ul className="m-0 list-none border-t border-line p-0">
            {members(s, m.id).map(({ person, access }) => (
              <li key={person.id} className="flex min-h-14 items-center gap-3 border-b border-line-subtle">
                <Avatar initials={person.initials} />
                <span className="grow">{person.id === s.me ? `${person.name}, you` : person.name}</span>
                <span className="text-[13px] text-ink-muted">
                  {access === 'owner' ? 'Owner' : access === 'steer' ? 'Can steer' : 'Can watch'}
                </span>
              </li>
            ))}
          </ul>
        </section>
      ))}
      {host.signOut ? (
        <Button variant="outline" size="touch" className="self-start" onClick={host.signOut}>
          Sign out
        </Button>
      ) : null}
    </MobileFrame>
  )
}

function MobileMissing() {
  return (
    <MobileFrame top={<BrandBar />} bottom={<MobileTabs active="missions" />} className="px-4 py-8">
      <p className="m-0 text-ink-muted">This mission is not here. It may still be syncing.</p>
    </MobileFrame>
  )
}

export function MobileGit() {
  return (
    <MobileFrame top={<BrandBar />} bottom={<MobileTabs active="git" />} className="px-4 pt-2 pb-6">
      <GitContent mobile />
    </MobileFrame>
  )
}
