import { type Approval, approve, type Message, type State } from '@astra/client'
import {
  AgentGlyph,
  ArrowRightIcon,
  Avatar,
  Button,
  CheckIcon,
  cx,
  LogWell,
  Markdown,
  Spec,
  Status,
} from '@astra/ui'
import { useAstraState, useClient } from '../context'
import { agentOf, agentStatus, canApprove, hhmm, personName, roleLetter, roleName } from '../select'

export function messagesOf(s: State, missionId: string): Message[] {
  return Object.values(s.messages)
    .filter((m) => m.missionId === missionId)
    .sort((a, b) => (a.at < b.at ? -1 : a.at > b.at ? 1 : a.id.localeCompare(b.id)))
}

/** Ids of the latest message per agent: only those carry the agent's live status. */
export function latestPerAgent(messages: Message[]): Set<string> {
  const latest = new Map<string, string>()
  for (const m of messages) if (m.author.type === 'agent') latest.set(m.author.missionAgentId, m.id)
  return new Set(latest.values())
}

export type ActionsVariant = 'desktop' | 'ios' | 'android'

/** Approve / Deny. iOS: equal width, approve first. Android: right-aligned, deny first. */
export function ApprovalActions({ approval, variant }: { approval: Approval; variant: ActionsVariant }) {
  const s = useAstraState()
  const client = useClient()
  if (approval.state !== 'pending') {
    const who = personName(s, approval.decidedBy)
    return approval.state === 'approved' ? (
      <Status tone="success" className="mt-2">
        Approved by {who}
      </Status>
    ) : (
      <Status tone="danger" className="mt-2">
        Denied by {who}
      </Status>
    )
  }
  if (!canApprove(s, s.me, approval.missionId, approval.missionAgentId)) {
    const owner = s.missions[approval.missionId]?.ownerId ?? null
    return (
      <p className="m-0 mt-2 text-[13px] text-ink-muted">Waiting for {personName(s, owner)} to approve.</p>
    )
  }
  const yes = (
    <Button
      size={variant === 'desktop' ? 'sm' : 'touch'}
      className={variant === 'ios' ? 'flex-1' : undefined}
      onClick={() => approve(client, approval.id, true)}
    >
      Approve
    </Button>
  )
  const no = (
    <Button
      variant="outline"
      size={variant === 'desktop' ? 'sm' : 'touch'}
      className={variant === 'ios' ? 'flex-1' : undefined}
      onClick={() => approve(client, approval.id, false)}
    >
      Deny
    </Button>
  )
  return (
    <div className={cx('mt-2 flex flex-wrap gap-2', variant === 'android' && 'justify-end')}>
      {variant === 'android' ? (
        <>
          {no}
          {yes}
        </>
      ) : (
        <>
          {yes}
          {no}
        </>
      )}
    </div>
  )
}

export function MessageItem({
  message,
  showStatus,
  variant = 'desktop',
  last,
}: {
  message: Message
  showStatus: boolean
  variant?: ActionsVariant
  last?: boolean
}) {
  const s = useAstraState()
  const mobile = variant !== 'desktop'
  const time = hhmm(message.at)
  let avatar: React.ReactNode
  let name: string
  let meta: string
  let status: React.ReactNode = null

  if (message.author.type === 'person') {
    const p = s.people[message.author.personId]
    name = p?.name ?? message.author.personId
    avatar = <Avatar initials={p?.initials ?? '?'} />
    const steering = message.steering ? s.missionAgents[message.steering] : undefined
    meta =
      message.author.personId === s.me
        ? `YOU · ${time}`
        : steering
          ? `STEERING ${roleName[steering.role].toUpperCase()} · ${time}`
          : time
  } else {
    const ma = s.missionAgents[message.author.missionAgentId]
    const agent = agentOf(s, ma)
    name = ma ? roleName[ma.role] : 'Agent'
    avatar = <AgentGlyph letter={ma ? roleLetter(ma.role) : 'A'} />
    meta = `${agent?.name.toUpperCase() ?? ''} · ${time}`
    if (showStatus && ma && (ma.status === 'working' || ma.status === 'needsYou')) {
      const st = agentStatus[ma.status]
      status = <Status tone={st.tone}>{st.label}</Status>
    }
  }
  const approval = message.approvalId ? s.approvals[message.approvalId] : undefined

  return (
    <li
      className={cx(
        'flex [content-visibility:auto]',
        mobile ? 'gap-3 py-3' : 'gap-4 py-4',
        !last && 'border-b border-line-subtle',
      )}
    >
      {avatar}
      <div className="flex min-w-0 grow flex-col gap-1">
        <div className={cx('flex flex-wrap items-baseline', mobile ? 'gap-x-2 gap-y-1' : 'gap-2')}>
          <b className="font-semibold">{name}</b>
          <Spec>{meta}</Spec>
          {status}
        </div>
        {message.body ? <Markdown>{message.body}</Markdown> : null}
        {message.plan.length > 0 ? (
          <ul className="m-0 mt-1 list-none p-0 text-ink-muted">
            {message.plan.map((step) => (
              <li
                key={step.text}
                className={cx('flex items-center gap-2', step.state === 'active' && 'text-ink')}
              >
                {step.state === 'active' ? (
                  <ArrowRightIcon />
                ) : step.state === 'done' ? (
                  <CheckIcon />
                ) : (
                  <span aria-hidden="true" className="inline-block size-3.5" />
                )}
                {step.text}
              </li>
            ))}
          </ul>
        ) : null}
        {message.log.length > 0 ? (
          <LogWell lines={mobile ? message.log.slice(0, 1) : message.log} compact={mobile} className="mt-1" />
        ) : null}
        {approval ? <ApprovalActions approval={approval} variant={variant} /> : null}
      </div>
    </li>
  )
}
