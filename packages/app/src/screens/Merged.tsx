import { buttonClass, Data, Dot, formatDiffStats, KVList, Spec } from '@astra/ui'
import { Link } from '@tanstack/react-router'
import { useAstraState } from '../context'
import { agentOf, members, missionNo, personName, roleName } from '../select'
import { AppHeader, Page } from '../shell/AppHeader'
import { MissionNotFound } from './common'
import { useNeedsYou } from './Home'
import { checksStatus } from './Review'

/** 8 · Merged. */
export function Merged({ missionId }: { missionId: string }) {
  const s = useAstraState()
  const mission = s.missions[missionId]
  const record = s.merges[missionId]
  const needs = useNeedsYou().filter((n) => n.missionId !== missionId)
  if (!mission) return <MissionNotFound />
  const result = record?.resultId ? s.results[record.resultId] : undefined
  const ma = result ? s.missionAgents[result.missionAgentId] : undefined
  const others = Object.values(s.results).filter((r) => r.missionId === missionId && r.id !== result?.id)
  const lead = record
    ? `${mission.title} is on ${mission.baseBranch}.${others.length ? ` Result ${others.map((o) => o.label).join(' and ')} ${others.length > 1 ? 'are' : 'is'} kept on ${others.length > 1 ? 'their own branches' : 'its own branch'}.` : ''}`
    : 'This mission has not been merged yet.'

  return (
    <Page header={<AppHeader repoSelect nav={null} people={members(s, missionId).map((m) => m.person)} />}>
      <main className="mx-auto box-border flex w-full max-w-[720px] flex-col gap-8 px-6 pt-24 pb-12">
        <div className="flex flex-col gap-3">
          <span className="flex items-center gap-2 text-kicker uppercase text-success">
            <Dot tone="success" />
            Mission {missionNo(mission)} · {record ? 'Merged' : mission.status}
          </span>
          <h1 className="m-0 text-display-md uppercase">
            {record ? 'Mission complete.' : 'Not merged yet.'}
          </h1>
          <p className="m-0 text-lead text-ink-muted">{lead}</p>
        </div>
        {record ? (
          <KVList
            items={[
              {
                k: 'Merged',
                v: result ? (
                  <>
                    Result {result.label}, by {ma ? roleName[ma.role] : 'an agent'}{' '}
                    <Spec>{agentOf(s, ma)?.name}</Spec>
                  </>
                ) : (
                  <Data>{mission.branch}</Data>
                ),
              },
              { k: 'Into', v: <Data>{record.into}</Data> },
              { k: 'Changes', v: <Data>{formatDiffStats(record.stats)}</Data> },
              { k: 'Checks', v: checksStatus(record) },
              {
                k: 'Approved by',
                v: `${personName(s, record.approvedBy)}${record.note ? `, ${record.note}` : ''}`,
              },
              ...(record.keptBranches.length
                ? [{ k: 'Kept for later', v: <Data>{record.keptBranches.join(', ')}</Data> }]
                : []),
            ]}
          />
        ) : null}
        <div className="flex flex-wrap items-center gap-3">
          <Link to="/" className={buttonClass('primary', 'md', 'px-6')}>
            Start next mission →
          </Link>
          <Link to="/board" className={buttonClass('outline', 'md')}>
            Open board
          </Link>
          {needs.length > 0 ? (
            <span className="text-ink-muted">
              {needs.length === 1 ? 'One mission still needs you.' : `${needs.length} things still need you.`}
            </span>
          ) : null}
        </div>
      </main>
    </Page>
  )
}
