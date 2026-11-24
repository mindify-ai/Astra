import { describe, expect, it } from 'vitest'
import { memoryFiles, relativeSegments } from '../src/files'

describe('memory files', () => {
  const host = () =>
    memoryFiles({
      'README.md': '# Hi',
      'src/main.ts': 'export {}',
      'src/lib/util.ts': 'export const a = 1',
      'node_modules/x/index.js': '',
      '.git/HEAD': 'ref: refs/heads/main',
    })

  it('lists folders first and hides build and VCS folders', async () => {
    expect(await host().list('/r', '')).toEqual([
      { name: 'src', path: 'src', kind: 'dir' },
      { name: 'README.md', path: 'README.md', kind: 'file' },
    ])
    expect(await host().list('/r', 'src')).toEqual([
      { name: 'lib', path: 'src/lib', kind: 'dir' },
      { name: 'main.ts', path: 'src/main.ts', kind: 'file' },
    ])
  })

  it('reads and writes files', async () => {
    const h = host()
    expect(await h.read('/r', 'src/main.ts')).toBe('export {}')
    await h.write('/r', 'src/main.ts', 'export const b = 2')
    expect(await h.read('/r', './src/main.ts')).toBe('export const b = 2')
    await expect(h.read('/r', 'missing.ts')).rejects.toThrow('No such file')
    await expect(h.write('/r', 'src', 'x')).rejects.toThrow('Not a file')
  })

  it('refuses paths that leave the root', async () => {
    const h = host()
    await expect(h.read('/r', '../etc/passwd')).rejects.toThrow('leaves the repository')
    await expect(h.read('/r', '/etc/passwd')).rejects.toThrow('relative')
    await expect(h.write('/r', 'src/../../x', 'x')).rejects.toThrow('leaves the repository')
    expect(() => relativeSegments('C:\\x')).toThrow('relative')
    expect(relativeSegments('./a//b/')).toEqual(['a', 'b'])
  })
})

describe('memoryFiles base', () => {
  it('serves committed copies when given, and has no base otherwise', async () => {
    const withBase = memoryFiles({ 'a.ts': 'new' }, { 'a.ts': 'old' })
    expect(await withBase.base?.('/r', 'a.ts')).toBe('old')
    expect(await withBase.base?.('/r', 'b.ts')).toBeNull()
    expect(memoryFiles({ 'a.ts': 'x' }).base).toBeUndefined()
  })
})
