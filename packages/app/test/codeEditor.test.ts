import { describe, expect, it } from 'vitest'
import {
  changeMarkers,
  createEditorState,
  diffCount,
  diffExtension,
  diffSlot,
  languageFor,
} from '../src/shared/codeEditor'

describe('code editor languages', () => {
  it('picks a language from the file name', () => {
    const cases: [string, string][] = [
      ['src/main.ts', 'TypeScript'],
      ['App.tsx', 'TypeScript JSX'],
      ['index.mjs', 'JavaScript'],
      ['Cargo.lock', 'TOML'],
      ['crates/core/src/lib.rs', 'Rust'],
      ['scripts/build.sh', 'Shell'],
      ['Dockerfile.dev', 'Dockerfile'],
      ['.env.local', 'Shell'],
      ['docs/README.md', 'Markdown'],
      ['config.yml', 'YAML'],
      ['main.go', 'Go'],
      ['tool.py', 'Python'],
      ['LICENSE', 'Plain text'],
    ]
    for (const [path, label] of cases) expect(languageFor(path).label, path).toBe(label)
  })

  it('builds a state with the file text and a labelled content area', () => {
    const state = createEditorState('fn main() {}', 'src/main.rs', 'Contents of src/main.rs')
    expect(state.doc.toString()).toBe('fn main() {}')
    expect(state.tabSize).toBe(2)
  })
})

describe('code editor changes', () => {
  const withDiff = (text: string, original: string) =>
    createEditorState(text, 'x.ts', 'x').update({ effects: diffSlot.reconfigure(diffExtension(original)) })
      .state

  it('places a marker for each change, by line', () => {
    const state = withDiff('a\nB\nc\nd\ne\nf', 'a\nb\nc\nd\ne\nX\nf')
    expect(changeMarkers(state).map(({ line, kind, top }) => ({ line, kind, top }))).toEqual([
      { line: 2, kind: 'mod', top: 1 / 6 },
      { line: 6, kind: 'del', top: 5 / 6 },
    ])
    expect(changeMarkers(createEditorState('a', 'x.ts', 'x'))).toEqual([])
  })

  it('counts the changes and says which one the cursor is in', () => {
    const state = withDiff('a\nB\nc\nd\ne\nf', 'a\nb\nc\nd\ne\nX\nf')
    expect(diffCount(state)).toMatchObject({ changes: 2, current: null })
    const inFirst = state.update({ selection: { anchor: 2 } }).state
    expect(diffCount(inFirst)).toMatchObject({ changes: 2, current: 1 })
  })
})
