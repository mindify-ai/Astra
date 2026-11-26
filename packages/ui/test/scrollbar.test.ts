// jsdom can't render scrollbars, so this guards the rule itself: hidden at rest, a token colour on hover.
import { readFileSync } from 'node:fs'
import { describe, expect, it } from 'vitest'

// Read from disk: Vitest replaces CSS imports (even `?raw`) with an empty string.
const css = readFileSync(new URL('../src/base.css', import.meta.url), 'utf8')

function rule(selector: string): string {
  const escaped = selector.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')
  const match = css.match(new RegExp(`(?:^|\\n)\\s*${escaped}\\s*\\{([^}]*)\\}`))
  return match?.[1] ?? ''
}

describe('scrollbars', () => {
  it('are transparent by default', () => {
    expect(rule('*')).toContain('scrollbar-color: transparent transparent')
    expect(rule('::-webkit-scrollbar-thumb')).toContain('background: transparent')
  })

  it('show a token colour on hover', () => {
    expect(rule('*:hover')).toContain('scrollbar-color: var(--line-strong) transparent')
    expect(rule('*:hover::-webkit-scrollbar-thumb')).toContain('background: var(--line-strong)')
  })

  it('never hide scrolling itself', () => {
    expect(css).not.toMatch(/::-webkit-scrollbar[^{]*\{[^}]*display:\s*none/)
    expect(css).not.toMatch(/scrollbar-width:\s*none/)
  })
})
