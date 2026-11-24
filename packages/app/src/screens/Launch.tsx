import { type Access, createMission, type MissionMode, slugify, titleFromGoal } from '@astra/client'
import {
  Avatar,
  Button,
  buttonClass,
  CloseIcon,
  Eyebrow,
  Field,
  inlineLinkClass,
  RadioCard,
  Select,
  Spec,
  TextArea,
  TextInput,
} from '@astra/ui'
import { Link, useNavigate } from '@tanstack/react-router'
import { useMemo, useState } from 'react'
import { useAstraState, useClient, useUi } from '../context'
import { connectedAgents, missionAgentName, orderedMissions } from '../select'
import { type AgentChoice, firstChoice, HarnessSelect, TypeSelect } from '../shared/agentPicker'
import { AppHeader, Page } from '../shell/AppHeader'
import { defaultTeam } from '../team'

interface TeamRow extends AgentChoice {
  key: string
  enabled: boolean
}

let rows = 0
const rowKey = () => `row-${++rows}`

const teamRows = (s: ReturnType<typeof useAstraState>, mode: MissionMode): TeamRow[] =>
  defaultTeam(s, mode).map((t) => ({
    key: rowKey(),
    agentId: t.agent.id,
    role: t.role,
    agentTypeId: null,
    enabled: true,
  }))

/** 5 · New mission. */
export function Launch({ initialGoal = '' }: { initialGoal?: string }) {
  const s = useAstraState()
  const client = useClient()
  const ui = useUi()
  const navigate = useNavigate()
  const [goal, setGoal] = useState(initialGoal)
  const [mode, setMode] = useState<MissionMode>('split')
  const [branch, setBranch] = useState('')
  const [team, setTeam] = useState<TeamRow[]>(() => teamRows(s, 'split'))
  const [managing, setManaging] = useState(false)
  const repo = s.repositories[ui.repoId]

  // People default to the access they had on the most recent mission; everyone else can watch.
  const [people, setPeople] = useState<Record<string, Access>>(() => {
    const last = orderedMissions(s)[0]
    const out: Record<string, Access> = {}
    for (const p of Object.values(s.people)) {
      if (p.id === s.me) continue
      out[p.id] = (
        last && s.memberships[`${last.id}:${p.id}`]?.access === 'steer' ? 'steer' : 'watch'
      ) as Access
    }
    return out
  })

  const pickMode = (m: MissionMode) => {
    setMode(m)
    setTeam(teamRows(s, m))
  }

  const branchPlaceholder = `astra/${slugify(titleFromGoal(goal || 'new mission'))}`
  const steerers = useMemo(
    () =>
      Object.entries(people)
        .filter(([, a]) => a === 'steer')
        .map(([id]) => s.people[id]?.name ?? id),
    [people, s.people],
  )
  const watchers = useMemo(
    () =>
      Object.entries(people)
        .filter(([, a]) => a === 'watch')
        .map(([id]) => s.people[id]?.name ?? id),
    [people, s.people],
  )
  const summary = [
    steerers.length ? `You and ${steerers.join(' and ')} can steer.` : 'Only you can steer.',
    watchers.length ? `${watchers.join(' and ')} can watch.` : '',
  ]
    .filter(Boolean)
    .join(' ')

  const start = async () => {
    if (!goal.trim() || !repo) return
    const id = await createMission(client, {
      goal: goal.trim(),
      mode,
      repoId: repo.id,
      baseBranch: repo.defaultBranch,
      branch: branch || undefined,
      agents: team.map(({ agentId, role, enabled, agentTypeId }) => ({
        agentId,
        role,
        enabled,
        agentTypeId,
      })),
      people: Object.entries(people).map(([personId, access]) => ({ personId, access })),
    })
    ui.setLastMissionId(id)
    void navigate({ to: '/missions/$missionId', params: { missionId: id } })
  }

  const cancelTo = ui.lastMissionId && s.missions[ui.lastMissionId] ? ui.lastMissionId : null

  return (
    <Page
      header={
        <AppHeader
          repoSelect
          nav={null}
          actions={
            cancelTo ? (
              <Link
                to="/missions/$missionId"
                params={{ missionId: cancelTo }}
                className={buttonClass('text', 'sm')}
              >
                CANCEL
              </Link>
            ) : (
              <Link to="/" className={buttonClass('text', 'sm')}>
                CANCEL
              </Link>
            )
          }
        />
      }
    >
      <main className="mx-auto box-border flex w-full max-w-[720px] flex-col gap-8 px-6 py-12">
        <div className="flex flex-col gap-2">
          <Eyebrow>New mission</Eyebrow>
          <h1 className="m-0 text-title">What should the agents do?</h1>
        </div>
        <Field label="Goal">
          {(id) => (
            <TextArea
              id={id}
              rows={3}
              value={goal}
              onChange={(e) => setGoal(e.target.value)}
              placeholder="Add token-bucket rate limiting to the gateway. Keep the public API unchanged."
            />
          )}
        </Field>

        <fieldset className="m-0 border-0 p-0">
          <legend className="mb-3 p-0 text-kicker uppercase text-ink-muted">How</legend>
          <div className="flex flex-wrap gap-3">
            <RadioCard
              name="mode"
              checked={mode === 'split'}
              onChange={() => pickMode('split')}
              title="Split the work"
              description="One agent plans. The others build, test and review."
            />
            <RadioCard
              name="mode"
              checked={mode === 'race'}
              onChange={() => pickMode('race')}
              title="Race"
              description="Every agent attempts the same task. You compare and merge one."
            />
          </div>
        </fieldset>

        <section className="flex flex-col gap-3">
          <div className="flex items-center justify-between">
            <Eyebrow as="h2">Agents</Eyebrow>
            <Link to="/setup/agents" className={inlineLinkClass}>
              + Connect another
            </Link>
          </div>
          <div className="border-t border-line">
            {team.map((t) => {
              const agent = s.agents[t.agentId]
              const name = missionAgentName(s, t)
              const update = (next: Partial<TeamRow>) =>
                setTeam(team.map((x) => (x.key === t.key ? { ...x, ...next } : x)))
              return (
                <div
                  key={t.key}
                  className="flex min-h-12 flex-wrap items-center gap-x-3 gap-y-1 border-b border-line-subtle px-3 py-1"
                >
                  <label className="flex min-h-10 flex-[1_1_200px] cursor-pointer items-center gap-3">
                    <input
                      type="checkbox"
                      className="size-4"
                      checked={t.enabled}
                      onChange={() => update({ enabled: !t.enabled })}
                    />
                    <span className="font-medium">{name}</span>
                    <Spec>{agent?.name}</Spec>
                  </label>
                  <TypeSelect
                    s={s}
                    label={`Type for ${name} on ${agent?.name ?? 'agent'}`}
                    value={t}
                    onChange={update}
                  />
                  <HarnessSelect
                    s={s}
                    label={`Agent for ${name}`}
                    value={t.agentId}
                    onChange={(agentId) => update({ agentId })}
                  />
                  <Button
                    variant="text"
                    size="sm"
                    aria-label={`Remove ${name} on ${agent?.name ?? 'agent'}`}
                    onClick={() => setTeam(team.filter((x) => x.key !== t.key))}
                  >
                    <CloseIcon size={14} />
                  </Button>
                </div>
              )
            })}
            {team.length === 0 ? (
              <p className="m-0 border-b border-line-subtle px-3 py-3 text-ink-muted">
                {connectedAgents(s).length === 0
                  ? 'No agents are connected yet.'
                  : 'No agents on this mission yet.'}
              </p>
            ) : null}
          </div>
          {firstChoice(s) ? (
            <div>
              <Button
                variant="text"
                size="sm"
                onClick={() => {
                  const c = firstChoice(s)
                  if (c) setTeam([...team, { ...c, key: rowKey(), enabled: true }])
                }}
              >
                + Add agent
              </Button>
            </div>
          ) : null}
        </section>

        <section className="flex flex-col gap-3">
          <Eyebrow as="h2">Workspace</Eyebrow>
          <div className="flex flex-wrap gap-3">
            <Field label="Repository" className="flex-[1_1_280px]">
              {(id) => (
                <Select id={id} mono value={ui.repoId} onChange={(e) => ui.setRepoId(e.target.value)}>
                  {Object.values(s.repositories).map((r) => (
                    <option key={r.id} value={r.id}>
                      {r.name}
                    </option>
                  ))}
                </Select>
              )}
            </Field>
            <Field label="Branch" className="flex-[1_1_280px]">
              {(id) => (
                <TextInput
                  id={id}
                  mono
                  value={branch}
                  placeholder={branchPlaceholder}
                  onChange={(e) => setBranch(e.target.value)}
                />
              )}
            </Field>
          </div>
        </section>

        <section className="flex flex-col gap-3">
          <Eyebrow as="h2">People</Eyebrow>
          <div className="flex flex-wrap items-center gap-3">
            <div className="flex items-center gap-1">
              {[s.people[s.me], ...Object.keys(people).map((id) => s.people[id])].map((p) =>
                p ? <Avatar key={p.id} initials={p.initials} name={p.name} /> : null,
              )}
            </div>
            <span className="grow text-ink-muted">{summary}</span>
            <button
              type="button"
              className={`${inlineLinkClass} border-0 bg-transparent p-0`}
              aria-expanded={managing}
              onClick={() => setManaging((m) => !m)}
            >
              Manage →
            </button>
          </div>
          {managing ? (
            <ul className="m-0 list-none border-t border-line p-0">
              {Object.entries(people).map(([id, access]) => (
                <li key={id} className="flex min-h-12 items-center gap-3 border-b border-line-subtle px-3">
                  <span className="grow">{s.people[id]?.name}</span>
                  <Select
                    size="sm"
                    aria-label={`Access for ${s.people[id]?.name}`}
                    value={access}
                    onChange={(e) => setPeople({ ...people, [id]: e.target.value as Access })}
                  >
                    <option value="steer">Can steer</option>
                    <option value="watch">Can watch</option>
                  </Select>
                </li>
              ))}
            </ul>
          ) : null}
        </section>

        <div className="flex flex-wrap items-center gap-x-4 gap-y-3 border-t border-line pt-6">
          <Button className="px-6" onClick={start} disabled={!goal.trim() || !team.some((t) => t.enabled)}>
            Start mission →
          </Button>
          <span className="flex-[1_1_280px] text-ink-muted">
            Agents work on the branch and ask before they run a command outside it.
          </span>
        </div>
      </main>
    </Page>
  )
}
