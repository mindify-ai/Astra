import type { Message } from '@astra/client'
import { describe, expect, it } from 'vitest'
import {
  anchorOf,
  formatChangeRequest,
  formatLineComment,
  groupByLine,
  lineCommentsOf,
  lineRef,
  numberLines,
  parseLineComment,
  splitRows,
} from '../src/shared/review'

const msg = (id: string, body: string): Message => ({
  kind: 'message',
  id,
  missionId: 'm-1',
  author: { type: 'person', personId: 'p-mark' },
  steering: null,
  to: null,
  body,
  plan: [],
  log: [],
  approvalId: null,
  side: null,
  at: `2026-11-21T09:0${id.length}:00Z`,
})

describe('review comments in the thread', () => {
  it('formats a line comment that parses back to the same line', () => {
    const body = formatLineComment('A', { path: 'src/foo.ts', side: 'new', line: 42 }, '  Version the key. ')
    expect(body).toBe('Result A · `src/foo.ts:42` — Version the key.')
    expect(parseLineComment(body)).toEqual({
      label: 'A',
      path: 'src/foo.ts',
      side: 'new',
      line: 42,
      text: 'Version the key.',
    })
  })

  it('marks removed lines with a minus and keeps multi-line text', () => {
    const body = formatLineComment(
      'B',
      { path: 'a/b.rs', side: 'old', line: 7 },
      'Why remove this?\n\nIt was used.',
    )
    expect(body.startsWith('Result B · `a/b.rs:-7` — ')).toBe(true)
    expect(parseLineComment(body)).toMatchObject({
      side: 'old',
      line: 7,
      text: 'Why remove this?\n\nIt was used.',
    })
  })

  it('accepts a bare reference and looser separators', () => {
    expect(parseLineComment('`src/foo.ts:3` nit: rename')).toMatchObject({
      label: null,
      path: 'src/foo.ts',
      line: 3,
      text: 'nit: rename',
    })
    expect(parseLineComment('**Result C** · `x.ts:1`: ok')).toMatchObject({ label: 'C', text: 'ok' })
    expect(parseLineComment('`C:/repo/x.ts:12` - windows path')).toMatchObject({
      path: 'C:/repo/x.ts',
      line: 12,
    })
  })

  it('ignores messages that are not line comments', () => {
    expect(parseLineComment('Looks good to me.')).toBeNull()
    expect(parseLineComment('See `src/foo.ts:3` — mid-sentence references are not comments')).toBeNull()
    expect(parseLineComment('Result A · `src/foo.ts:3` — ')).toBeNull()
    expect(parseLineComment('Result A · `src/foo.ts:0` — line zero')).toBeNull()
    expect(parseLineComment(formatChangeRequest('A', 'Add tests.'))).toBeNull()
  })

  it('keeps comments for the result and unlabelled ones, grouped by line', () => {
    const messages = [
      msg('1', 'Result A · `src/foo.ts:3` — first'),
      msg('22', 'Result B · `src/foo.ts:3` — for B'),
      msg('333', '`src/foo.ts:3` — unlabelled'),
      msg('4444', 'Result A · `src/foo.ts:-2` — removed'),
      msg('55555', 'Plain chat'),
    ]
    const forA = lineCommentsOf(messages, 'A')
    expect(forA.map((c) => c.comment.text)).toEqual(['first', 'unlabelled', 'removed'])
    const grouped = groupByLine(forA)
    expect(grouped.get('src/foo.ts:3')?.map((c) => c.message.id)).toEqual(['1', '333'])
    expect(grouped.get('src/foo.ts:-2')?.length).toBe(1)
  })
})

describe('diff line numbers', () => {
  const lines = numberLines([
    { kind: 'ctx', text: 'a' },
    { kind: 'del', text: 'b' },
    { kind: 'del', text: 'c' },
    { kind: 'add', text: 'B' },
    { kind: 'ctx', text: 'd' },
    { kind: 'add', text: 'e' },
  ])

  it('numbers old and new sides from 1', () => {
    expect(lines.map((l) => [l.old, l.new])).toEqual([
      [1, 1],
      [2, null],
      [3, null],
      [null, 2],
      [4, 3],
      [null, 4],
    ])
  })

  it('anchors removals to the old file and the rest to the new', () => {
    expect(lineRef(anchorOf('f', lines[1]!))).toBe('f:-2')
    expect(lineRef(anchorOf('f', lines[3]!))).toBe('f:2')
    expect(lineRef(anchorOf('f', lines[4]!))).toBe('f:3')
  })

  it('pairs removals with the additions after them in split view', () => {
    expect(splitRows(lines).map((r) => [r.left?.text ?? null, r.right?.text ?? null])).toEqual([
      ['a', 'a'],
      ['b', 'B'],
      ['c', null],
      ['d', 'd'],
      [null, 'e'],
    ])
  })
})
