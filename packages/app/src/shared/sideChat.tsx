import { newSideThreadId, sendMessage } from '@astra/client'
import { buttonClass, CloseIcon, cx, Eyebrow, Select, Spec, Status } from '@astra/ui'
import { useEffect, useRef, useState } from 'react'
import { useAstraState, useClient, useUi } from '../context'
import { canSteer, roleName } from '../select'
import { Composer } from './composer'
import { agentMentions, sideQuestion } from './harness'
import { MessageItem, sideMessagesOf, sideThreadsOf } from './thread'

/** Sends a side question on a mission, in `side` or a new side thread; resolves with the thread id or an error. */
export async function askSide(
  client: Parameters<typeof sendMessage>[0],
  missionId: string,
  to: string | null,
  question: string,
  side: string | null,
): Promise<{ side: string; error: string | null }> {
  const id = side ?? newSideThreadId()
  const outcome = await sendMessage(client, missionId, to, question, id)
  return {
    side: id,
    error: outcome && !outcome.ok ? (outcome.error ?? 'The question was not delivered.') : null,
  }
}

/**
 * Sends what someone typed in a mission composer: `/btw <question>` goes to the side chat (and opens it), anything
 * else to the main thread. Resolves with an error to show, or null.
 */
export async function sendFromComposer(
  client: Parameters<typeof sendMessage>[0],
  ui: Pick<ReturnType<typeof useUi>, 'sideThreads' | 'setSideThread' | 'setSideOpen'>,
  missionId: string,
  to: string | null,
  text: string,
  onSide?: () => void,
): Promise<string | null> {
  const question = sideQuestion(text)
  if (question !== null) {
    if (!question) return 'Add a question after /btw.'
    const res = await askSide(client, missionId, to, question, ui.sideThreads[missionId] ?? null)
    ui.setSideThread(missionId, res.side)
    ui.setSideOpen(true)
    onSide?.()
    return res.error
  }
  const outcome = await sendMessage(client, missionId, to, text)
  return outcome && !outcome.ok ? (outcome.error ?? 'The message was not delivered.') : null
}

/**
 * Side chat: questions about a mission that are answered beside the thread. They never reach the running
 * agents or take steering, and their answers stay out of the main thread. Teammates see the same side threads.
 */
export function SideChat({
  missionId,
  onClose,
  className,
}: {
  missionId: string
  onClose?: () => void
  className?: string
}) {
  const s = useAstraState()
  const client = useClient()
  const ui = useUi()
  const threads = sideThreadsOf(s, missionId)
  const chosen = ui.sideThreads[missionId]
  // Undefined means nothing was picked yet: show the latest side thread. Null is a new one.
  const side = chosen === undefined ? (threads[0]?.id ?? null) : chosen
  const messages = side ? sideMessagesOf(s, missionId, side) : []
  const [to, setTo] = useState<string | null>(null)
  const [draft, setDraft] = useState('')
  const [error, setError] = useState<string | null>(null)
  const end = useRef<HTMLDivElement>(null)
  const mayAsk = canSteer(s, s.me, missionId)
  const lastFromPerson = messages[messages.length - 1]?.author.type === 'person'

  // biome-ignore lint/correctness/useExhaustiveDependencies: follow the newest message
  useEffect(() => {
    end.current?.scrollIntoView?.({ block: 'end' })
  }, [messages.length])

  const send = async () => {
    const text = draft.trim()
    if (!text || !mayAsk) return
    setDraft('')
    setError(null)
    const res = await askSide(client, missionId, to, text, side)
    ui.setSideThread(missionId, res.side)
    if (res.error) setError(res.error)
  }

  const mentions = agentMentions(s, missionId)
  const target = to ? s.missionAgents[to] : undefined

  return (
    <section aria-label="Side chat" className={cx('flex min-h-0 min-w-0 flex-col bg-surface-alt', className)}>
      <div className="flex h-9 shrink-0 items-center gap-2 border-b border-line px-3">
        <Eyebrow as="h2" className="shrink-0 whitespace-nowrap">
          Side chat
        </Eyebrow>
        <Select
          aria-label="Side thread"
          size="sm"
          ground="page"
          value={side ?? ''}
          onChange={(e) => ui.setSideThread(missionId, e.target.value || null)}
          className="min-w-0 max-w-[180px] grow font-mono text-spec"
        >
          <option value="">New side thread</option>
          {threads.map((t) => (
            <option key={t.id} value={t.id}>
              {t.title.length > 40 ? `${t.title.slice(0, 40)}…` : t.title}
            </option>
          ))}
        </Select>
        <span className="grow" />
        <button
          type="button"
          className={buttonClass('text', 'sm', 'h-7 px-2')}
          onClick={() => ui.setSideThread(missionId, null)}
          disabled={side === null}
        >
          NEW
        </button>
        {onClose ? (
          <button
            type="button"
            aria-label="Close side chat"
            onClick={onClose}
            className="flex size-7 items-center justify-center border-0 bg-transparent p-0 text-ink-muted hover:text-ink"
          >
            <CloseIcon size={14} />
          </button>
        ) : null}
      </div>
      <div className="min-h-0 grow overflow-y-auto px-4">
        {messages.length === 0 ? (
          <div className="flex flex-col gap-2 py-6">
            <p className="m-0 text-body-sm text-ink-muted">
              Ask about the mission without interrupting it. The agents keep working, and nothing here lands
              in the main thread.
            </p>
            <Spec>TIP · TYPE /BTW IN THE THREAD TO ASK FROM THERE</Spec>
          </div>
        ) : (
          <ol aria-label="Side thread" className="m-0 list-none p-0">
            {messages.map((m, i) => (
              <MessageItem key={m.id} message={m} showStatus={false} last={i === messages.length - 1} dense />
            ))}
          </ol>
        )}
        {lastFromPerson ? (
          <div className="pb-3">
            <Status tone="working">Answering</Status>
          </div>
        ) : null}
        <div ref={end} />
      </div>
      <form
        className="flex shrink-0 flex-col gap-2 border-t border-line p-3"
        onSubmit={(e) => {
          e.preventDefault()
          void send()
        }}
      >
        <Composer
          label="Side question"
          value={draft}
          onChange={setDraft}
          onSubmit={() => void send()}
          disabled={!mayAsk}
          placeholder={mayAsk ? 'Ask a side question' : 'You can watch this mission'}
          commands={[]}
          mentions={mentions}
          repoId={s.missions[missionId]?.repoId ?? null}
          onPickMention={(m) => (m.missionAgentId ? setTo(m.missionAgentId) : undefined)}
        />
        <div className="flex items-center gap-2">
          <Spec className="min-w-0 grow truncate">
            {target ? `ASKING ${roleName[target.role].toUpperCase()}` : 'ASKING THE MISSION'} · AGENTS KEEP
            WORKING
          </Spec>
          {target ? (
            <button
              type="button"
              className={buttonClass('text', 'sm', 'h-7 px-2')}
              onClick={() => setTo(null)}
            >
              ANYONE
            </button>
          ) : null}
          <button type="submit" className={buttonClass('outline', 'sm')} disabled={!mayAsk || !draft.trim()}>
            ASK
          </button>
        </div>
        {error ? (
          <div role="alert" className="text-[13px]">
            <Status tone="danger">{error}</Status>
          </div>
        ) : null}
      </form>
    </section>
  )
}
