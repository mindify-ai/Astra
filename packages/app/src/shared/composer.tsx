import type { FilesHost } from '@astra/client'
import { AgentGlyph, cx, FileIcon, Spec } from '@astra/ui'
import {
  type KeyboardEvent,
  useCallback,
  useEffect,
  useId,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from 'react'
import { useAstraState, useHost } from '../context'
import { activeToken, harnessLabel, knownFiles, type Mention, rank, type SlashCommand } from './harness'

/** Folders the file walk never enters. */
const SKIP = new Set([
  '.git',
  'node_modules',
  'target',
  'dist',
  'build',
  '.next',
  '.turbo',
  '.venv',
  '__pycache__',
])
const WALK_LIMIT = 4000

/** Lists up to WALK_LIMIT files under `root`, breadth first, for `@` mentions. */
async function walkFiles(files: FilesHost, root: string): Promise<string[]> {
  const out: string[] = []
  const queue = ['']
  while (queue.length > 0 && out.length < WALK_LIMIT) {
    const dir = queue.shift() ?? ''
    let entries: Awaited<ReturnType<FilesHost['list']>>
    try {
      entries = await files.list(root, dir)
    } catch {
      continue
    }
    for (const e of entries) {
      if (e.kind === 'dir') {
        if (!SKIP.has(e.name)) queue.push(e.path)
      } else out.push(e.path)
    }
  }
  return out
}

/**
 * Files to offer after `@` on this repository: the ones missions touched, plus (on the desktop app) the
 * repository's own files, read once the first time someone types `@`.
 */
function useMentionFiles(repoId: string | null): [string[], () => void] {
  const s = useAstraState()
  const host = useHost()
  const repo = repoId ? s.repositories[repoId] : undefined
  const { missions, results, messages, missionAgents } = s
  const known = useMemo(
    () => (repoId ? knownFiles({ missions, results, messages, missionAgents }, repoId) : []),
    [repoId, missions, results, messages, missionAgents],
  )
  const [disk, setDisk] = useState<string[] | null>(null)
  const started = useRef<string | null>(null)
  const root = repo?.path ?? null
  const load = useCallback(() => {
    if (!host.files || !root || started.current === root) return
    started.current = root
    void walkFiles(host.files, root).then(setDisk, () => setDisk([]))
  }, [host.files, root])
  const all = useMemo(() => (disk ? [...new Set([...known, ...disk])].sort() : known), [known, disk])
  return [all, load]
}

type Item =
  | { type: 'command'; key: string; command: SlashCommand }
  | { type: 'mention'; key: string; mention: Mention }

export interface ComposerProps {
  id?: string
  /** Accessible name, e.g. "Message". */
  label: string
  value: string
  onChange: (value: string) => void
  onSubmit: () => void
  disabled?: boolean
  placeholder?: string
  /** Offered after `/` at the start of the message. */
  commands: SlashCommand[]
  /** Agents (and anything else) offered after `@`, before files. */
  mentions: Mention[]
  /** Repository whose files are offered after `@`; null offers none. */
  repoId: string | null
  /** Called when someone picks an agent mention (to address that agent). */
  onPickMention?: (mention: Mention) => void
  className?: string
}

/**
 * The chat prompt: a growing text area that sends on Enter (Shift+Enter breaks the line) and completes `/`
 * commands and `@` mentions the way Claude Code and Codex do, from a list under the caret.
 */
export function Composer({
  id,
  label,
  value,
  onChange,
  onSubmit,
  disabled,
  placeholder,
  commands,
  mentions,
  repoId,
  onPickMention,
  className,
}: ComposerProps) {
  const listId = useId()
  const ref = useRef<HTMLTextAreaElement>(null)
  const pendingCaret = useRef<number | null>(null)
  const [caret, setCaret] = useState(0)
  const [active, setActive] = useState(0)
  const [dismissed, setDismissed] = useState<string | null>(null)
  const [files, loadFiles] = useMentionFiles(repoId)

  const token = disabled ? null : activeToken(value, caret)
  const tokenKey = token ? `${token.trigger}${token.start}` : null
  const open = token !== null && tokenKey !== dismissed

  useEffect(() => {
    if (token?.trigger === '@') loadFiles()
  }, [token?.trigger, loadFiles])

  const items: Item[] = useMemo(() => {
    if (!token) return []
    if (token.trigger === '/') {
      return rank(commands, token.query, (c) => c.name, 50).map((c) => ({
        type: 'command' as const,
        key: `${c.harness}:${c.name}`,
        command: c,
      }))
    }
    const agents = rank(mentions, token.query, (m) => m.value, 4)
    const paths = rank(
      files.map((f): Mention => ({ kind: 'file', value: f, label: f })),
      token.query,
      (m) => m.value,
      8,
    )
    return [...agents, ...paths].map((m) => ({
      type: 'mention' as const,
      key: `${m.kind}:${m.value}`,
      mention: m,
    }))
  }, [token, commands, mentions, files])

  // biome-ignore lint/correctness/useExhaustiveDependencies: back to the first item whenever the query changes
  useEffect(() => setActive(0), [tokenKey, token?.query])

  // Grow with the text, up to about eight lines; then it scrolls.
  // biome-ignore lint/correctness/useExhaustiveDependencies: re-measure when the text changes
  useLayoutEffect(() => {
    const el = ref.current
    if (!el) return
    el.style.height = 'auto'
    el.style.height = `${Math.min(el.scrollHeight, 200)}px`
    // After a completion, put the caret just past what was inserted.
    const at = pendingCaret.current
    if (at !== null) {
      pendingCaret.current = null
      el.focus()
      el.setSelectionRange(at, at)
    }
  }, [value])

  const pick = (item: Item) => {
    if (!token) return
    const insert = item.type === 'command' ? `/${item.command.name} ` : `@${item.mention.value} `
    const next = value.slice(0, token.start) + insert + value.slice(caret)
    const at = token.start + insert.length
    onChange(next)
    setCaret(at)
    if (item.type === 'mention') onPickMention?.(item.mention)
    pendingCaret.current = at
  }

  const showList = open && items.length > 0
  const onKeyDown = (e: KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.nativeEvent.isComposing) return
    if (showList) {
      if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
        e.preventDefault()
        const d = e.key === 'ArrowDown' ? 1 : -1
        setActive((i) => (i + d + items.length) % items.length)
        return
      }
      if (e.key === 'Enter' || e.key === 'Tab') {
        const item = items[active]
        if (item) {
          e.preventDefault()
          pick(item)
          return
        }
      }
      if (e.key === 'Escape') {
        e.preventDefault()
        setDismissed(tokenKey)
        return
      }
    }
    if (e.key === 'Enter' && !e.shiftKey && !e.altKey) {
      e.preventDefault()
      onSubmit()
    }
  }

  const track = () => setCaret(ref.current?.selectionStart ?? value.length)
  const optionId = (i: number) => `${listId}-${i}`

  return (
    <div className={cx('relative flex min-w-0', className)}>
      <textarea
        ref={ref}
        id={id}
        rows={1}
        aria-label={label}
        role="combobox"
        aria-autocomplete="list"
        aria-expanded={showList}
        aria-controls={showList ? listId : undefined}
        aria-activedescendant={showList ? optionId(active) : undefined}
        value={value}
        disabled={disabled}
        placeholder={placeholder}
        onChange={(e) => {
          onChange(e.target.value)
          setCaret(e.target.selectionStart ?? e.target.value.length)
          setDismissed(null)
        }}
        onSelect={track}
        onClick={track}
        onKeyDown={onKeyDown}
        onBlur={() => setDismissed(tokenKey)}
        className="box-border min-h-10 w-full resize-none rounded-sm border border-line-strong bg-surface-raised px-3 py-[9px] text-ink leading-5"
      />
      {showList ? (
        <div
          id={listId}
          role="listbox"
          aria-label={token?.trigger === '/' ? 'Commands' : 'Mentions'}
          className="absolute bottom-full left-0 z-40 mb-1 flex max-h-80 w-full min-w-[280px] max-w-[560px] flex-col overflow-y-auto border border-line bg-surface-overlay py-1 shadow-lg"
        >
          {items.map((item, i) => (
            <div
              key={item.key}
              id={optionId(i)}
              role="option"
              aria-selected={i === active}
              tabIndex={-1}
              onMouseDown={(e) => e.preventDefault()}
              onMouseEnter={() => setActive(i)}
              onClick={() => pick(item)}
              onKeyDown={() => {}}
              className={cx(
                'flex min-h-9 cursor-pointer items-center gap-3 px-3 py-1.5',
                i === active ? 'bg-surface-raised text-ink' : 'text-ink',
              )}
            >
              {item.type === 'command' ? (
                <>
                  <span className="shrink-0 font-mono text-data">
                    /{item.command.name}
                    {item.command.args ? <span className="text-ink-muted"> {item.command.args}</span> : null}
                  </span>
                  <span className="min-w-0 grow truncate text-body-sm text-ink-muted">
                    {item.command.description}
                  </span>
                  <Spec className="shrink-0">{harnessLabel[item.command.harness]}</Spec>
                </>
              ) : item.mention.kind === 'agent' ? (
                <>
                  <AgentGlyph letter={item.mention.label.slice(0, 1)} className="size-5 text-[11px]" />
                  <span className="shrink-0">{item.mention.label}</span>
                  <span className="min-w-0 grow truncate text-body-sm text-ink-muted">
                    {item.mention.detail}
                  </span>
                  <Spec className="shrink-0">AGENT</Spec>
                </>
              ) : (
                <>
                  <FileIcon name={item.mention.value.split('/').pop() ?? item.mention.value} size={14} />
                  <span className="min-w-0 grow truncate font-mono text-data" title={item.mention.value}>
                    {item.mention.value}
                  </span>
                  <Spec className="shrink-0">FILE</Spec>
                </>
              )}
            </div>
          ))}
        </div>
      ) : null}
    </div>
  )
}
