import { describe, expect, it } from 'vitest'
import { createEditorState, languageFor } from '../src/shared/codeEditor'

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
