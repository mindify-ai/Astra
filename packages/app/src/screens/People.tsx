import { type Access, invite, setApprovalPolicy, setMembership } from '@astra/client'
import { Avatar, Button, CloseIcon, Dot, Eyebrow, Radio, Select, Spec, TextInput } from '@astra/ui'
import { useEffect, useRef, useState } from 'react'
import { useAstraState, useClient, useHost, useUi } from '../context'
import { members, missionNo, roleName } from '../select'

function activityOf(
  s: ReturnType<typeof useAstraState>,
  personId: string,
  missionId: string,
): { tone: 'working' | 'needsYou' | 'idle'; text: string } {
  const pending = Object.values(s.approvals).find((a) => a.missionId === missionId && a.state === 'pending')
  const owner = s.missions[missionId]?.ownerId
  if (pending && personId === owner) {
    const ma = s.missionAgents[pending.missionAgentId]
    return { tone: 'needsYou', text: `${ma ? roleName[ma.role] : 'An agent'} is waiting on you` }
  }
  const steering = Object.values(s.missionAgents).find(
    (m) => m.missionId === missionId && m.steeredBy === personId,
  )
  if (steering) return { tone: 'working', text: `Steering ${roleName[steering.role]}` }
  const p = s.presence.find((x) => x.personId === personId && x.missionId === missionId)
  return { tone: 'idle', text: p?.activity ?? 'Not here right now' }
}

/** People in this mission: invite, change access, and who can approve agents. */
export function PeoplePanel({ missionId, onClose }: { missionId: string; onClose: () => void }) {
  const s = useAstraState()
  const client = useClient()
  const host = useHost()
  const [email, setEmail] = useState('')
  const [access, setAccess] = useState<Access>('steer')
  const [copied, setCopied] = useState(false)
  const mission = s.missions[missionId]
  const list = members(s, missionId)
  if (!mission) return null
  const owner = mission.ownerId === s.me

  return (
    <div className="box-border flex w-[480px] max-w-full flex-col gap-5 border border-line bg-surface-overlay p-6 shadow-lg">
      <div className="flex items-start justify-between gap-4">
        <div className="flex flex-col gap-1">
          <h1 id="people-title" className="m-0 text-heading">
            People in this mission
          </h1>
          <Spec>
            {missionNo(mission)} · {mission.shortName}
          </Spec>
        </div>
        <button
          type="button"
          aria-label="Close"
          onClick={onClose}
          className="flex size-8 items-center justify-center border-0 bg-transparent p-0 text-ink-muted hover:text-ink"
        >
          <CloseIcon />
        </button>
      </div>
      {owner ? (
        <form
          className="flex flex-wrap gap-2"
          onSubmit={(e) => {
            e.preventDefault()
            if (email.includes('@')) void invite(client, missionId, email, access).then(() => setEmail(''))
          }}
        >
          <TextInput
            type="email"
            aria-label="Email"
            placeholder="name@company.com"
            value={email}
            onChange={(e) => setEmail(e.target.value)}
            className="flex-[1_1_160px]"
          />
          <Select
            aria-label="Access"
            value={access}
            onChange={(e) => setAccess(e.target.value as Access)}
            className="px-2"
          >
            <option value="steer">Can steer</option>
            <option value="watch">Can watch</option>
          </Select>
          <Button type="submit" className="px-4">
            Invite
          </Button>
        </form>
      ) : null}
      <ul className="m-0 list-none border-t border-line p-0">
        {list.map(({ person, access: a }) => {
          const act = activityOf(s, person.id, missionId)
          return (
            <li key={person.id} className="flex min-h-14 items-center gap-3 border-b border-line">
              <Avatar initials={person.initials} />
              <span className="flex grow flex-col">
                <span className="font-medium">
                  {person.id === s.me ? `${person.name}, you` : person.name}
                </span>
                <span
                  className={`flex items-center gap-2 text-[13px] ${act.tone === 'working' ? 'text-signal' : act.tone === 'needsYou' ? 'text-warning' : 'text-ink-muted'}`}
                >
                  <Dot tone={act.tone} />
                  {act.text}
                </span>
              </span>
              {a === 'owner' ? (
                <span className="text-[13px] text-ink-muted">Owner</span>
              ) : (
                <Select
                  size="sm"
                  aria-label={`Access for ${person.name}`}
                  value={a}
                  disabled={!owner}
                  onChange={(e) =>
                    void setMembership(
                      client,
                      missionId,
                      person.id,
                      e.target.value === 'remove' ? null : (e.target.value as Access),
                    )
                  }
                  className="text-[13px]"
                >
                  <option value="steer">Can steer</option>
                  <option value="watch">Can watch</option>
                  <option value="remove">Remove</option>
                </Select>
              )}
            </li>
          )
        })}
      </ul>
      <fieldset className="m-0 border-0 p-0">
        <legend className="mb-2 p-0">
          <Eyebrow>Who can approve an agent</Eyebrow>
        </legend>
        <Radio
          name="approve-policy"
          checked={mission.approvalPolicy === 'owner'}
          onChange={() => owner && void setApprovalPolicy(client, missionId, 'owner')}
        >
          Owner only
        </Radio>
        <Radio
          name="approve-policy"
          checked={mission.approvalPolicy === 'steerers'}
          onChange={() => owner && void setApprovalPolicy(client, missionId, 'steerers')}
        >
          Anyone who can steer
        </Radio>
      </fieldset>
      <div className="mt-auto flex flex-col gap-4">
        <Spec as="p" className="m-0">
          One person steers an agent at a time. Everyone else can watch or take over.
        </Spec>
        <Button
          variant="outline"
          className="self-start"
          onClick={async () => {
            const link = `astra://join?workspace=${encodeURIComponent(s.workspaceId)}&mission=${encodeURIComponent(missionId)}`
            if (host.copy) await host.copy(link)
            else await navigator.clipboard?.writeText(link)
            setCopied(true)
          }}
        >
          {copied ? 'Link copied' : 'Copy invite link'}
        </Button>
      </div>
    </div>
  )
}

/** Modal wrapper: opened from any SHARE button. */
export function PeopleDialog() {
  const ui = useUi()
  const s = useAstraState()
  const ref = useRef<HTMLDialogElement>(null)
  const missionId = ui.shareMissionId ?? Object.keys(s.missions)[0] ?? null
  const open = ui.shareMissionId !== null
  useEffect(() => {
    const d = ref.current
    if (!d) return
    if (open && !d.open) d.showModal?.()
    if (!open && d.open) d.close?.()
  }, [open])
  return (
    <dialog
      ref={ref}
      aria-labelledby="people-title"
      onClose={() => ui.openShare(null)}
      onClick={(e) => {
        if (e.target === ref.current) ui.openShare(null)
      }}
      onKeyDown={(e) => {
        if (e.key === 'Escape') ui.openShare(null)
      }}
      className="m-auto max-w-[calc(100vw-32px)] border-0 bg-transparent p-0 text-ink backdrop:bg-scrim"
    >
      {open && missionId ? <PeoplePanel missionId={missionId} onClose={() => ui.openShare(null)} /> : null}
    </dialog>
  )
}
