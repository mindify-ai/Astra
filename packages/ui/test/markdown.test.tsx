// @vitest-environment jsdom
import { cleanup, render, screen } from '@testing-library/react'
import { afterEach, describe, expect, it } from 'vitest'
import { Markdown } from '../src'

afterEach(cleanup)

const md = `## Plan

1. Read **README.md**
2. Run \`pnpm test\`

- [x] tokens
- [ ] docs

| File | Change |
| ---- | ------ |
| a.ts | +3 |

\`\`\`ts
const x: number = 1
\`\`\`

<script>alert(1)</script>
![tracker](https://example.com/pixel.png)
[docs](https://example.com)`

describe('Markdown', () => {
  it('renders GitHub-flavoured Markdown with highlighted code', () => {
    const { container } = render(<Markdown>{md}</Markdown>)
    expect(screen.getByRole('heading', { name: 'Plan' })).toBeTruthy()
    expect(container.querySelector('strong')?.textContent).toBe('README.md')
    expect(container.querySelectorAll('input[type="checkbox"]')).toHaveLength(2)
    expect(screen.getByRole('table')).toBeTruthy()
    expect(container.querySelector('pre code.language-ts .hljs-keyword')?.textContent).toBe('const')
    expect(screen.getByRole('button', { name: 'Copy' })).toBeTruthy()
    expect(screen.getByRole('link', { name: 'docs' }).getAttribute('rel')).toContain('noopener')
  })

  it('never renders raw HTML or loads images', () => {
    const { container } = render(<Markdown>{md}</Markdown>)
    expect(container.querySelector('script')).toBeNull()
    expect(container.querySelector('img')).toBeNull()
    expect(container.textContent).toContain('[image: tracker]')
  })
})
