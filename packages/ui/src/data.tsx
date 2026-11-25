import type { ReactNode } from 'react'
import { cx } from './cx'

export interface LogLineView {
  verb: string
  target: string
}

/** Sunken mono well for an agent's recent actions: "edit  src/limiter/bucket.ts". */
export function LogWell({
  lines,
  compact,
  collapsible,
  defaultOpen = false,
  label = 'Output',
  className,
}: {
  lines: LogLineView[]
  compact?: boolean
  /**
   * Fold the output behind a summary row ("OUTPUT · 4 LINES") that people open to examine it. The latest line
   * stays visible in the summary so the gist reads without opening it.
   */
  collapsible?: boolean
  /** Start opened (collapsible only). */
  defaultOpen?: boolean
  /** Summary word (collapsible only). */
  label?: string
  className?: string
}) {
  if (lines.length === 0) return null
  const width = Math.max(...lines.map((l) => l.verb.length)) + 2
  const well = (extra?: string) => (
    <pre
      className={cx(
        'overflow-x-auto border border-line bg-surface-sunken font-mono text-ink-muted',
        compact ? 'px-3 py-2 text-[12px] leading-[18px]' : 'px-4 py-3 text-code',
        extra,
      )}
    >
      {lines.map((l) => `${l.verb.padEnd(width)}${l.target}`).join('\n')}
    </pre>
  )
  if (!collapsible) return well(className)
  const last = lines[lines.length - 1]
  return (
    <details open={defaultOpen || undefined} className={cx('group/log', className)}>
      <summary className="flex h-8 cursor-pointer list-none items-center gap-2 border border-line bg-surface-sunken px-3 font-mono text-spec uppercase text-ink-muted select-none hover:text-ink [&::-webkit-details-marker]:hidden">
        <svg
          aria-hidden="true"
          width={12}
          height={12}
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          strokeWidth={1.5}
          className="shrink-0 transition-transform group-open/log:rotate-90"
        >
          <path d="M9 6l6 6-6 6" />
        </svg>
        <span className="shrink-0">
          {label} · {lines.length} {lines.length === 1 ? 'line' : 'lines'}
        </span>
        <span className="min-w-0 truncate normal-case tracking-normal group-open/log:hidden">
          {last ? `${last.verb} ${last.target}` : null}
        </span>
      </summary>
      {well('m-0 border-t-0')}
    </details>
  )
}

export interface DiffFileView {
  path: string
  lines: { kind: 'ctx' | 'add' | 'del'; text: string }[]
}

const linePrefix = { ctx: '  ', add: '+ ', del: '- ' } as const
const lineColor = { ctx: 'text-ink-muted', add: 'text-success', del: 'text-danger' } as const

/** Plain unified diff: path in ink, additions green, removals red. */
export function DiffView({
  files,
  compact,
  className,
}: {
  files: DiffFileView[]
  compact?: boolean
  className?: string
}) {
  return (
    <pre
      className={cx(
        'overflow-x-auto bg-surface-sunken font-mono text-ink-muted',
        compact ? 'border border-line p-3 text-[12px] leading-[18px]' : 'p-4 text-code',
        className,
      )}
    >
      {files.map((f, fi) => (
        <span key={f.path}>
          {fi > 0 ? '\n' : null}
          <span className="text-ink">{f.path}</span>
          {'\n'}
          {f.lines.map((l, i) => (
            // biome-ignore lint/suspicious/noArrayIndexKey: diff lines have no identity beyond position
            <span key={i} className={lineColor[l.kind]}>
              {linePrefix[l.kind]}
              {l.text}
              {i < f.lines.length - 1 ? '\n' : null}
            </span>
          ))}
        </span>
      ))}
    </pre>
  )
}

/** "2 files · +38 −6" with a true minus sign. */
export function formatDiffStats(
  s: { files: number; additions: number; deletions: number },
  filesWord = 'files',
): string {
  const word = s.files === 1 ? filesWord.replace(/s$/, '') : filesWord
  return `${s.files} ${word} · +${s.additions} −${s.deletions}`
}

/** Key/value rows with hairlines (Map aside, Merged summary). */
export function KVList({ items, dense }: { items: { k: ReactNode; v: ReactNode }[]; dense?: boolean }) {
  return (
    <dl className="m-0 flex flex-col">
      {items.map((it, i) => (
        <div
          // biome-ignore lint/suspicious/noArrayIndexKey: static rows
          key={i}
          className={cx(
            'flex flex-wrap items-center justify-between gap-x-4 gap-y-1 border-t border-line',
            dense ? 'py-2.5' : 'py-3',
            i === items.length - 1 && 'border-b',
          )}
        >
          <dt className="text-ink-muted">{it.k}</dt>
          <dd className="m-0 text-right">{it.v}</dd>
        </div>
      ))}
    </dl>
  )
}
