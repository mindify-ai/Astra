import { Button, cx, Eyebrow, Select, Status } from '@astra/ui'
import { Link } from '@tanstack/react-router'
import { useEffect, useRef, useState } from 'react'
import { useAstraState, useClient, useUi } from '../context'
import { canSteer, missionNo, missionStatus, orderedMissions, roleName } from '../select'
import { Composer } from './composer'
import { agentMentions, commandsFor, missionHarnesses } from './harness'
import { SideChat, sendFromComposer } from './sideChat'
import { latestPerAgent, MessageItem, messagesOf } from './thread'

type Tab = 'thread' | 'side'

/**
 * The mission thread as a pane: pick a mission on this repository, follow its thread or its side chat, and
 * message the agents with the same `/` commands and `@` mentions as the thread screen.
 */
export function MissionChatPane({ onClose, className }: { onClose?: () => void; className?: string }) {
  const s = useAstraState()
  const ui = useUi()
  const missions = orderedMissions(s, ui.repoId)
  const fallback = missions[0]?.id ?? null
  const missionId =
    ui.lastMissionId && s.missions[ui.lastMissionId]?.repoId === ui.repoId ? ui.lastMissionId : fallback
  const [tab, setTab] = useState<Tab>('thread')

  return (
    <section aria-label="Chat" className={cx('flex min-h-0 min-w-0 flex-col bg-surface', className)}>
      <div className="flex h-9 shrink-0 items-center gap-2 border-b border-line bg-surface-alt px-3">
        <Eyebrow as="h2">Chat</Eyebrow>
        {missions.length > 0 ? (
          <Select
            aria-label="Mission"
            size="sm"
            ground="page"
            value={missionId ?? ''}
            onChange={(e) => ui.setLastMissionId(e.target.value)}
            className="min-w-0 max-w-[220px] grow font-mono text-spec uppercase"
          >
            {missions.map((m) => (
              <option key={m.id} value={m.id}>
                {missionNo(m)} · {m.title}
              </option>
            ))}
          </Select>
        ) : null}
        <span className="grow" />
        {onClose ? (
          <button
            type="button"
            className="h-7 border-0 bg-transparent px-2 text-label text-ink-muted hover:text-ink"
            onClick={onClose}
          >
            HIDE
          </button>
        ) : null}
      </div>
      {!missionId ? (
        <div className="flex flex-col items-start gap-4 p-6">
          <p className="m-0 text-body-sm text-ink-muted">No mission on this repository yet.</p>
          <Link to="/missions/new" className="text-[13px] text-ink-muted hover:text-ink">
            Start a mission →
          </Link>
        </div>
      ) : (
        <>
          <div
            role="tablist"
            aria-label="Chat view"
            className="flex h-9 shrink-0 gap-5 border-b border-line px-4"
          >
            {(
              [
                ['thread', 'THREAD'],
                ['side', 'SIDE CHAT'],
              ] as const
            ).map(([value, label]) => (
              <button
                key={value}
                type="button"
                role="tab"
                aria-selected={tab === value}
                onClick={() => setTab(value)}
                className={cx(
                  'box-border h-9 border-0 border-y-2 border-transparent bg-transparent p-0 text-label uppercase',
                  tab === value ? 'border-b-signal text-ink' : 'text-ink-muted hover:text-ink',
                )}
              >
                {label}
              </button>
            ))}
          </div>
          {tab === 'thread' ? (
            <CompactThread key={missionId} missionId={missionId} onSide={() => setTab('side')} />
          ) : (
            <SideChat key={missionId} missionId={missionId} className="min-h-0 grow" />
          )}
        </>
      )}
    </section>
  )
}

function CompactThread({ missionId, onSide }: { missionId: string; onSide: () => void }) {
  const s = useAstraState()
  const client = useClient()
  const ui = useUi()
  const mission = s.missions[missionId]
  const [to, setTo] = useState<string | null>(null)
  const [draft, setDraft] = useState('')
  const [error, setError] = useState<string | null>(null)
  const end = useRef<HTMLDivElement>(null)
  const messages = messagesOf(s, missionId)
  const latest = latestPerAgent(messages)
  const mayWrite = canSteer(s, s.me, missionId)

  // biome-ignore lint/correctness/useExhaustiveDependencies: follow the newest message
  useEffect(() => {
    end.current?.scrollIntoView?.({ block: 'end' })
  }, [messages.length])

  if (!mission) return null
  const st = missionStatus[mission.status]
  const target = to ? s.missionAgents[to] : undefined

  const send = async () => {
    const text = draft.trim()
    if (!text || !mayWrite) return
    if (text === '/btw') {
      setError('Add a question after /btw.')
      return
    }
    setDraft('')
    setError(null)
    const err = await sendFromComposer(client, ui, missionId, to, text, onSide)
    if (err) setError(err)
  }

  return (
    <>
      <div className="flex shrink-0 items-center gap-3 border-b border-line-subtle px-4 py-2">
        <Status tone={st.tone}>{st.label}</Status>
        <Link
          to="/missions/$missionId"
          params={{ missionId }}
          className="ml-auto text-[13px] text-ink-muted no-underline hover:text-ink"
        >
          Open thread →
        </Link>
      </div>
      <ol aria-label="Thread" className="m-0 min-h-0 grow list-none overflow-y-auto px-4 py-0">
        {messages.map((m, i) => (
          <MessageItem
            key={m.id}
            message={m}
            showStatus={latest.has(m.id)}
            last={i === messages.length - 1}
            dense
          />
        ))}
        <div ref={end} />
      </ol>
      <form
        className="flex shrink-0 flex-col gap-2 border-t border-line p-3"
        onSubmit={(e) => {
          e.preventDefault()
          void send()
        }}
      >
        <Composer
          label="Message"
          value={draft}
          onChange={setDraft}
          onSubmit={() => void send()}
          disabled={!mayWrite}
          placeholder={mayWrite ? '/ for commands, @ for files and agents' : 'You can watch this mission'}
          commands={commandsFor(missionHarnesses(s, missionId, to))}
          mentions={agentMentions(s, missionId)}
          repoId={mission.repoId}
          onPickMention={(m) => (m.missionAgentId ? setTo(m.missionAgentId) : undefined)}
        />
        <div className="flex items-center gap-2">
          <span className="min-w-0 grow truncate font-mono text-spec uppercase text-ink-muted">
            TO {target ? roleName[target.role] : 'ALL AGENTS'}
          </span>
          {target ? (
            <button
              type="button"
              className="h-7 border-0 bg-transparent px-2 text-label text-ink-muted hover:text-ink"
              onClick={() => setTo(null)}
            >
              ALL
            </button>
          ) : null}
          <Button type="submit" size="sm" disabled={!mayWrite || !draft.trim()}>
            Send
          </Button>
        </div>
        {error ? (
          <div role="alert" className="text-[13px]">
            <Status tone="danger">{error}</Status>
          </div>
        ) : null}
      </form>
    </>
  )
}
