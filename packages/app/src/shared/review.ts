import type { DiffFile, Message } from '@astra/client'

// Code review on top of the mission thread. The protocol has no review comments, so a line comment is a
// normal thread message whose body starts with a reference to the line:
//
//   Result A · `src/pricing/handler.ts:3` — Cache key needs a version.
//
// `path:N` is line N of the new file (an added or unchanged line). `path:-N` is line N of the old file
// (a removed line). The "Result X · " prefix names the race result; a reference without it applies to
// every result that changes that path.

export type DiffKind = 'ctx' | 'add' | 'del'
export type Side = 'old' | 'new'

/** A diff line with its line numbers in the old and new file (null on the side it does not exist). */
export interface NumberedLine {
  kind: DiffKind
  text: string
  old: number | null
  new: number | null
}

export interface LineAnchor {
  path: string
  side: Side
  line: number
}

export interface LineComment extends LineAnchor {
  /** Result label ("A"), or null when the message named no result. */
  label: string | null
  text: string
}

export interface ReviewComment {
  comment: LineComment
  message: Message
}

/** Numbers a hunk from line 1: context advances both sides, a removal the old, an addition the new. */
export function numberLines(lines: DiffFile['lines']): NumberedLine[] {
  let o = 0
  let n = 0
  return lines.map((l) => {
    if (l.kind === 'add') return { ...l, old: null, new: ++n }
    if (l.kind === 'del') return { ...l, old: ++o, new: null }
    return { ...l, old: ++o, new: ++n }
  })
}

/** Where a comment on this line attaches: removals to the old file, everything else to the new. */
export function anchorOf(path: string, l: NumberedLine): LineAnchor {
  return l.kind === 'del' ? { path, side: 'old', line: l.old ?? 0 } : { path, side: 'new', line: l.new ?? 0 }
}

/** "src/foo.ts:42" (new file) or "src/foo.ts:-42" (old file). Also the key comments are grouped by. */
export function lineRef(a: LineAnchor): string {
  return `${a.path}:${a.side === 'old' ? '-' : ''}${a.line}`
}

/** The thread message body for a line comment. */
export function formatLineComment(label: string, anchor: LineAnchor, text: string): string {
  return `Result ${label} · \`${lineRef(anchor)}\` — ${text.trim()}`
}

/** The thread message body for a request for changes on a whole result. */
export function formatChangeRequest(label: string, text: string): string {
  return `Result ${label} · Changes requested — ${text.trim()}`
}

const COMMENT =
  /^\s*(?:\*{0,2}Result\s+([^\s·*]+)\*{0,2}\s*[·•|]\s*)?`([^`\n]+):(-?)(\d+)`\s*(?:[—–:-]\s*)?([\s\S]*)$/

/** Reads a line comment back from a message body, or null if the body does not start with a line reference. */
export function parseLineComment(body: string): LineComment | null {
  const m = COMMENT.exec(body)
  if (!m) return null
  const [, label, path, minus, line, rest] = m
  const text = (rest ?? '').trim()
  const n = Number(line)
  if (!path || !text || !Number.isFinite(n) || n < 1) return null
  return { label: label ?? null, path: path.trim(), side: minus ? 'old' : 'new', line: n, text }
}

/** Line comments on the result labelled `label`, in thread order. */
export function lineCommentsOf(messages: Message[], label: string): ReviewComment[] {
  const out: ReviewComment[] = []
  for (const message of messages) {
    const comment = parseLineComment(message.body)
    if (comment && (comment.label === null || comment.label === label)) out.push({ comment, message })
  }
  return out
}

/** Comments keyed by `lineRef`. */
export function groupByLine(comments: ReviewComment[]): Map<string, ReviewComment[]> {
  const map = new Map<string, ReviewComment[]>()
  for (const c of comments) {
    const key = lineRef(c.comment)
    const list = map.get(key)
    if (list) list.push(c)
    else map.set(key, [c])
  }
  return map
}

export interface SplitRow {
  left: NumberedLine | null
  right: NumberedLine | null
}

/** Side-by-side rows: context on both sides, each run of removals paired with the additions that follow it. */
export function splitRows(lines: NumberedLine[]): SplitRow[] {
  const rows: SplitRow[] = []
  let i = 0
  while (i < lines.length) {
    const l = lines[i] as NumberedLine
    if (l.kind === 'ctx') {
      rows.push({ left: l, right: l })
      i++
      continue
    }
    const dels: NumberedLine[] = []
    const adds: NumberedLine[] = []
    while (i < lines.length && lines[i]?.kind === 'del') dels.push(lines[i++] as NumberedLine)
    while (i < lines.length && lines[i]?.kind === 'add') adds.push(lines[i++] as NumberedLine)
    for (let k = 0; k < Math.max(dels.length, adds.length); k++) {
      rows.push({ left: dels[k] ?? null, right: adds[k] ?? null })
    }
  }
  return rows
}

/** "+5 −1" for one file, with a true minus sign. */
export function fileStats(f: DiffFile): { additions: number; deletions: number } {
  let additions = 0
  let deletions = 0
  for (const l of f.lines) {
    if (l.kind === 'add') additions++
    else if (l.kind === 'del') deletions++
  }
  return { additions, deletions }
}
