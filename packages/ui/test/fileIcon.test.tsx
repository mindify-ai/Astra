// @vitest-environment jsdom
import { cleanup, render } from '@testing-library/react'
import { afterEach, describe, expect, it } from 'vitest'
import { FileIcon } from '../src'

const icon = (name: string, folder = false) => {
  const { container } = render(<FileIcon name={name} folder={folder} />)
  return container.firstElementChild as HTMLElement
}

afterEach(cleanup)

describe('FileIcon', () => {
  it('gives code files a monogram that adds no text', () => {
    for (const [name, mono] of [
      ['main.ts', 'TS'],
      ['App.tsx', 'TSX'],
      ['lib.rs', 'RS'],
      ['package.json', '{}'],
      ['Dockerfile', 'DK'],
      ['util.test.ts', 'TS'],
    ] as const) {
      const el = icon(name)
      expect(el.getAttribute('data-icon')).toBe(mono)
      expect(el.textContent).toBe('')
      expect(el.getAttribute('aria-hidden')).toBe('true')
    }
  })

  it('draws other files and folders', () => {
    for (const name of ['README.md', 'logo.png', '.gitignore', 'pnpm-lock.yaml', 'notes']) {
      expect(icon(name).tagName.toLowerCase()).toBe('svg')
    }
    expect(icon('src', true).tagName.toLowerCase()).toBe('svg')
  })
})
