import { describe, expect, it } from 'vitest'
import { type Language, languageOf, type TokenKind, tokenize } from '../src/shared/tokenize'

/** The non-plain tokens, as `kind:text`. */
const marks = (text: string, lang: Language) =>
  tokenize(text, lang)
    .filter((t) => t.kind !== 'plain')
    .map((t) => `${t.kind}:${t.text}`)

const kinds: TokenKind[] = ['keyword', 'string', 'comment', 'number', 'plain']

describe('tokenizer', () => {
  it('maps file names to languages', () => {
    expect(languageOf('src/app.tsx')).toBe('ts')
    expect(languageOf('index.mjs')).toBe('ts')
    expect(languageOf('package.json')).toBe('json')
    expect(languageOf('src/lib.rs')).toBe('rust')
    expect(languageOf('base.css')).toBe('css')
    expect(languageOf('README.md')).toBe('md')
    expect(languageOf('Makefile')).toBe('plain')
    expect(languageOf('.gitignore')).toBe('plain')
  })

  it('always gives back the input exactly', () => {
    const samples: [string, Language][] = [
      // biome-ignore lint/suspicious/noTemplateCurlyInString: TypeScript source as data
      ['const a = `x ${y}` // done\n/* open', 'ts'],
      ['fn main() { let s = r#"raw "q""#; \'a\'; }', 'rust'],
      ['{"a": [1, -2.5e3, true, null, "s\\"x"]}', 'json'],
      ['@media (min-width: 40rem) { .a { color: #fff !important; } }', 'css'],
      ['# Title\n\n- item `code`\n```ts\nconst x = 1\n```\n> quote', 'md'],
      ['unterminated "string', 'ts'],
      ['', 'ts'],
    ]
    for (const [text, lang] of samples) {
      const tokens = tokenize(text, lang)
      expect(tokens.map((t) => t.text).join('')).toBe(text)
      for (const t of tokens) expect(kinds).toContain(t.kind)
    }
  })

  it('highlights TypeScript', () => {
    expect(marks("export const n = 0x1f + 2.5 // why\nlet s = 'it\\'s'", 'ts')).toEqual([
      'keyword:export',
      'keyword:const',
      'number:0x1f',
      'number:2.5',
      'comment:// why',
      'keyword:let',
      "string:'it\\'s'",
    ])
    // Identifiers that contain keywords or digits stay plain.
    expect(marks('constant x1 returned', 'ts')).toEqual([])
    expect(marks('/* a\nb */ `t\n`', 'ts')).toEqual(['comment:/* a\nb */', 'string:`t\n`'])
  })

  it('highlights Rust, telling chars from lifetimes', () => {
    expect(marks("pub fn f<'a>(x: &'a str) -> u32 { let c = 'z'; 10_u32 }", 'rust')).toEqual([
      'keyword:pub',
      'keyword:fn',
      'keyword:str',
      'keyword:u32',
      'keyword:let',
      "string:'z'",
      'number:10_u32',
    ])
    expect(marks('let r = r#"a "b" c"#;', 'rust')).toEqual(['keyword:let', 'string:r#"a "b" c"#'])
  })

  it('highlights JSON keys apart from values', () => {
    expect(marks('{"name": "astra", "n": -1.5, "ok": true}', 'json')).toEqual([
      'keyword:"name"',
      'string:"astra"',
      'keyword:"n"',
      'number:-1.5',
      'keyword:"ok"',
      'keyword:true',
    ])
  })

  it('highlights CSS', () => {
    expect(marks('/* c */ @media x { .a-1 { margin: -4px 50%; color: #0af !important } }', 'css')).toEqual([
      'comment:/* c */',
      'keyword:@media',
      'number:-4px',
      'number:50%',
      'number:#0af',
      'keyword:!important',
    ])
  })

  it('highlights Markdown headings, code, quotes and list markers', () => {
    expect(
      marks('# Title\nText with `code` and C# here.\n  - item\n> quoted\n```\nraw # no\n```', 'md'),
    ).toEqual([
      'keyword:# Title',
      'string:`code`',
      'number:  -',
      'comment:> quoted',
      'string:```\nraw # no\n```',
    ])
  })

  it('leaves plain text alone', () => {
    expect(tokenize('const x = 1', 'plain')).toEqual([{ kind: 'plain', text: 'const x = 1' }])
  })
})
