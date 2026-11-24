import type { ComponentPropsWithoutRef } from 'react'
import { cx } from './cx'

// Line-level diff pieces for review tables: a number gutter (optionally a button), and a code cell with
// its +/− sign. Additions sit on success-soft, removals on danger-soft, context on the sunken ground.

export type DiffLineKind = 'ctx' | 'add' | 'del'

export const diffGroundClass: Record<DiffLineKind, string> = {
  ctx: '',
  add: 'bg-success-soft',
  del: 'bg-danger-soft',
}

const signText: Record<DiffLineKind, string> = { ctx: ' ', add: '+', del: '−' }
const signColor: Record<DiffLineKind, string> = {
  ctx: 'text-ink-muted',
  add: 'text-success',
  del: 'text-danger',
}

/** One gutter number column ("12", or blank for the side a line does not exist on). */
export function DiffLineNo({ n }: { n: number | null }) {
  return <span className="inline-block min-w-[4ch] text-right">{n ?? ''}</span>
}

/** The number gutter as a button: clicking a line's numbers starts a comment on it. */
export function DiffGutterButton({ className, children, ...props }: ComponentPropsWithoutRef<'button'>) {
  return (
    <button
      type="button"
      className={cx(
        'flex w-full cursor-pointer items-start justify-end gap-2 border-0 bg-transparent px-2 font-mono text-code text-ink-muted',
        'hover:bg-signal-soft hover:text-ink disabled:cursor-default disabled:hover:bg-transparent',
        className,
      )}
      {...props}
    >
      {children}
    </button>
  )
}

/** Sign and text of one line, as a table cell. A null kind draws an empty half (split view). */
export function DiffCode({
  kind,
  text,
  className,
}: {
  kind: DiffLineKind | null
  text: string
  className?: string
}) {
  return (
    <td
      className={cx(
        'whitespace-pre px-2 align-top font-mono text-code text-ink',
        kind ? diffGroundClass[kind] : 'bg-surface-alt',
        className,
      )}
    >
      {kind ? (
        <>
          <span aria-hidden="true" className={cx('inline-block w-[2ch] select-none', signColor[kind])}>
            {signText[kind]}
          </span>
          {kind !== 'ctx' ? (
            <span className="sr-only">{kind === 'add' ? 'Added: ' : 'Removed: '}</span>
          ) : null}
          {text}
        </>
      ) : null}
    </td>
  )
}
