// @vitest-environment jsdom
import { cleanup, render, screen } from '@testing-library/react'
import { afterEach, describe, expect, it } from 'vitest'
import { DiffView, formatDiffStats, LogWell, Status, WindowControls } from '../src'

afterEach(cleanup)

describe('ui primitives', () => {
  it('formats diff stats with a true minus sign', () => {
    expect(formatDiffStats({ files: 2, additions: 38, deletions: 6 })).toBe('2 files · +38 −6')
    expect(formatDiffStats({ files: 1, additions: 1, deletions: 0 })).toBe('1 file · +1 −0')
  })

  it('pads log verbs into a column', () => {
    const { container } = render(
      <LogWell
        lines={[
          { verb: 'edit', target: 'a.ts' },
          { verb: 'run', target: 'pnpm test' },
        ]}
      />,
    )
    expect(container.textContent).toBe('edit  a.ts\nrun   pnpm test')
  })

  it('renders diff prefixes', () => {
    const { container } = render(
      <DiffView
        files={[
          {
            path: 'x.ts',
            lines: [
              { kind: 'del', text: 'a' },
              { kind: 'add', text: 'b' },
            ],
          },
        ]}
      />,
    )
    expect(container.textContent).toBe('x.ts\n- a\n+ b')
  })

  it('always pairs a status dot with a word', () => {
    render(<Status tone="needsYou">Needs you</Status>)
    expect(screen.getByText('Needs you')).toBeTruthy()
  })

  it.each(['mac', 'windows', 'linux'] as const)('draws labelled window controls on %s', (platform) => {
    render(<WindowControls platform={platform} />)
    expect(screen.getByLabelText('Close window')).toBeTruthy()
    expect(screen.getByLabelText('Minimize window')).toBeTruthy()
  })
})
