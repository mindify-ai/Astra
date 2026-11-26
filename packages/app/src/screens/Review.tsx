import type { RaceResult } from '@astra/client'
import {
  AgentGlyph,
  Avatar,
  Badge,
  Button,
  buttonClass,
  cx,
  Data,
  DiffView,
  Eyebrow,
  formatDiffStats,
  inlineLinkClass,
  Markdown,
  Spec,
  Status,
} from '@astra/ui'
import { Link } from '@tanstack/react-router'
import { useAstraState } from '../context'
import { agentOf, canApprove, hhmm, members, missionNo, roleLetter, roleName } from '../select'
import { MergeNotice, useResultMerge } from '../shared/git'
import { messagesOf } from '../shared/thread'
import { AppHeader, Page, ShareButton } from '../shell/AppHeader'
import { MissionNotFound, useMissionPresence } from './common'

export function resultsOf(s: ReturnType<typeof useAstraState>, missionId: string): RaceResult[] {
  return Object.values(s.results)
    .filter((r) => r.missionId === missionId)
    .sort((a, b) => a.label.localeCompare(b.label))
}

/** The reviewer agent's latest verdict on a race. */
export function useVerdict(missionId: string) {
  const s = useAstraState()
  const msgs = messagesOf(s, missionId).filter(
    (m) => m.author.type === 'agent' && s.missionAgents[m.author.missionAgentId]?.role === 'reviewer',
  )
  const m = msgs[msgs.length - 1]
  if (m?.author.type !== 'agent') return null
  const ma = s.missionAgents[m.author.missionAgentId]
  return { message: m, ma, agent: agentOf(s, ma) }
}

export const checksStatus = (r: Pick<RaceResult, 'checks'>) =>
  r.checks === 'passed' ? (
    <Status tone="success">Tests passed</Status>
  ) : r.checks === 'failed' ? (
    <Status tone="danger">Tests failed</Status>
  ) : r.checks === 'pending' ? (
    <Status tone="working">Tests running</Status>
  ) : (
    <Status tone="idle">No tests</Status>
  )

function ResultCard({ result, merged }: { result: RaceResult; merged: boolean }) {
  const s = useAstraState()
  const action = useResultMerge(result)
  const ma = s.missionAgents[result.missionAgentId]
  const allowed = canApprove(s, s.me, result.missionId, result.missionAgentId)
  return (
    <article
      className={cx(
        'flex min-w-0 flex-[1_1_480px] flex-col border',
        result.reviewerPick ? 'border-ink' : 'border-line',
      )}
    >
      <div className="flex flex-wrap items-center gap-x-4 gap-y-2 border-b border-line p-4">
        <h2 className="m-0 text-heading">Result {result.label}</h2>
        <span>
          {ma ? roleName[ma.role] : ''} <Spec>{agentOf(s, ma)?.name}</Spec>
        </span>
        {result.reviewerPick ? <Badge>Reviewer's pick</Badge> : null}
        <div className="grow" />
        <Data className="text-ink-muted">{formatDiffStats(result.stats)}</Data>
        {checksStatus(result)}
      </div>
      <MergeNotice action={action} />
      <DiffView files={result.diff} className="grow" />
      <div className="flex flex-wrap items-center gap-4 border-t border-line p-4">
        <Button
          variant={result.reviewerPick ? 'primary' : 'outline'}
          onClick={action.doMerge}
          disabled={!allowed || merged || action.busy}
          title={allowed ? undefined : 'Only the owner can merge'}
        >
          Merge {result.label}
        </Button>
        <Link
          to="/missions/$missionId/review/$resultId"
          params={{ missionId: result.missionId, resultId: result.id }}
          className={inlineLinkClass}
        >
          Review code →
        </Link>
        <Link to="/missions/$missionId" params={{ missionId: result.missionId }} className={inlineLinkClass}>
          Open thread →
        </Link>
      </div>
    </article>
  )
}

/** 7 · Review and merge: compare race results side by side. */
export function Review({ missionId }: { missionId: string }) {
  const s = useAstraState()
  const mission = s.missions[missionId]
  const verdict = useVerdict(missionId)
  useMissionPresence(missionId, 'Comparing results')
  if (!mission) return <MissionNotFound />
  const results = resultsOf(s, missionId)
  const people = members(s, missionId)
  const reading = s.presence.find(
    (p) => p.missionId === missionId && p.personId !== s.me && /result/i.test(p.activity),
  )
  const reader = reading ? s.people[reading.personId] : undefined
  const merged = mission.status === 'merged'

  return (
    <Page
      header={
        <AppHeader
          repoSelect
          nav={null}
          people={people.map((p) => p.person)}
          actions={<ShareButton missionId={missionId} />}
        />
      }
    >
      <main className="flex grow flex-col gap-6 px-8 pt-6 pb-8">
        <div className="flex flex-wrap items-end justify-between gap-4">
          <div className="flex flex-col gap-2">
            <Eyebrow>
              Mission {missionNo(mission)} · {mission.mode === 'race' ? 'Race' : 'Split'} · {results.length}{' '}
              results
            </Eyebrow>
            <h1 className="m-0 text-title">{mission.title}</h1>
          </div>
          {reader && reading ? (
            <span className="flex items-center gap-2 text-[13px] text-ink-muted">
              <Avatar initials={reader.initials} size={20} />
              {reader.name} is {reading.activity.charAt(0).toLowerCase() + reading.activity.slice(1)}
            </span>
          ) : null}
        </div>
        {merged ? (
          <p className="m-0 flex flex-wrap items-center gap-3">
            <Status tone="success">Merged</Status>
            <Link
              to="/missions/$missionId/merged"
              params={{ missionId }}
              className={buttonClass('text', 'sm', 'text-ink')}
            >
              See summary →
            </Link>
          </p>
        ) : null}
        {verdict ? (
          <section aria-label="Reviewer" className="flex gap-4 border border-line bg-surface-raised p-4">
            <AgentGlyph letter={verdict.ma ? roleLetter(verdict.ma.role) : 'R'} />
            <div className="flex flex-col gap-1">
              <div className="flex flex-wrap items-baseline gap-2">
                <b className="font-semibold">{verdict.ma ? roleName[verdict.ma.role] : 'Reviewer'}</b>
                <Spec>
                  {verdict.agent?.name} · {hhmm(verdict.message.at)}
                </Spec>
              </div>
              <Markdown className="max-w-[760px]">{verdict.message.body}</Markdown>
            </div>
          </section>
        ) : null}
        <div className="flex flex-wrap gap-6">
          {results.map((r) => (
            <ResultCard key={r.id} result={r} merged={merged} />
          ))}
          {results.length === 0 ? (
            <p className="m-0 text-ink-muted">
              The agents are still working. Results appear here when they finish.
            </p>
          ) : null}
        </div>
      </main>
    </Page>
  )
}
