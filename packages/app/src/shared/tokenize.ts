/**
 * A small, dependency-free syntax tokenizer for the code editor. It knows just enough of TS/JS, JSON,
 * Rust, CSS and Markdown to colour keywords, strings, comments and numbers. Joining the tokens' text
 * always gives back the input exactly, so the highlighted copy lines up with the textarea above it.
 */

export type Language = 'ts' | 'json' | 'rust' | 'css' | 'md' | 'plain'
export type TokenKind = 'keyword' | 'string' | 'comment' | 'number' | 'plain'
export interface Token {
  kind: TokenKind
  text: string
}

/** Display names for the status bar. */
export const languageLabel: Record<Language, string> = {
  ts: 'TypeScript',
  json: 'JSON',
  rust: 'Rust',
  css: 'CSS',
  md: 'Markdown',
  plain: 'Plain text',
}

const byExtension: Record<string, Language> = {
  ts: 'ts',
  tsx: 'ts',
  mts: 'ts',
  cts: 'ts',
  js: 'ts',
  jsx: 'ts',
  mjs: 'ts',
  cjs: 'ts',
  json: 'json',
  jsonc: 'json',
  rs: 'rust',
  css: 'css',
  md: 'md',
  markdown: 'md',
  mdx: 'md',
}

export function languageOf(path: string): Language {
  const name = path.split('/').pop() ?? ''
  const dot = name.lastIndexOf('.')
  const ext = dot > 0 ? name.slice(dot + 1).toLowerCase() : ''
  return byExtension[ext] ?? 'plain'
}

/** Token text colours (token classes only). */
export const tokenClass: Record<TokenKind, string> = {
  keyword: 'text-chart-3',
  string: 'text-signal',
  comment: 'text-ink-muted',
  number: 'text-warning',
  plain: '',
}

const words = (s: string) => new Set(s.split(/\s+/).filter(Boolean))

const tsKeywords = words(`
  abstract any as async await bigint boolean break case catch class const constructor continue debugger
  declare default delete do else enum export extends false finally for from function get if implements
  import in infer instanceof interface is keyof let namespace never new null number object of package
  private protected public readonly return satisfies set static string super switch symbol this throw true
  try type typeof undefined unique unknown var void while with yield`)

const rustKeywords = words(`
  as async await break const continue crate dyn else enum extern false fn for if impl in let loop match mod
  move mut pub ref return self Self static struct super trait true type union unsafe use where while
  bool char str u8 u16 u32 u64 u128 usize i8 i16 i32 i64 i128 isize f32 f64 Some None Ok Err`)

const jsonKeywords = words('true false null')

type Rule = [kind: TokenKind | 'word', re: RegExp]

const ws: Rule = ['plain', /\s+/y]
const lineComment: Rule = ['comment', /\/\/[^\n]*/y]
const blockComment: Rule = ['comment', /\/\*[\s\S]*?(?:\*\/|$(?![\s\S]))/y]
const dq: Rule = ['string', /"(?:[^"\\\n]|\\.)*"?/y]
const sq: Rule = ['string', /'(?:[^'\\\n]|\\.)*'?/y]

const rules: Record<Exclude<Language, 'plain'>, { rules: Rule[]; keywords?: Set<string> }> = {
  ts: {
    keywords: tsKeywords,
    rules: [
      ws,
      lineComment,
      blockComment,
      dq,
      sq,
      ['string', /`(?:[^`\\]|\\[\s\S])*`?/y],
      ['number', /(?:0[xX][\da-fA-F_]+|0[bB][01_]+|0[oO][0-7_]+|(?:\d[\d_]*\.?[\d_]*|\.\d[\d_]*)(?:[eE][+-]?\d+)?)n?/y],
      ['word', /[A-Za-z_$][\w$]*/y],
    ],
  },
  rust: {
    keywords: rustKeywords,
    rules: [
      ws,
      lineComment,
      blockComment,
      ['string', /b?r(#*)"[\s\S]*?(?:"\1|$(?![\s\S]))/y],
      ['string', /b?"(?:[^"\\]|\\[\s\S])*"?/y],
      ['string', /b?'(?:[^'\\\n]|\\(?:x[\da-fA-F]{2}|u\{[\da-fA-F]{1,6}\}|.))'/y],
      // A lifetime or label ('a, 'static): not a char literal.
      ['plain', /'[A-Za-z_]\w*/y],
      [
        'number',
        /(?:0x[\da-fA-F_]+|0b[01_]+|0o[0-7_]+|\d[\d_]*(?:\.\d[\d_]*)?(?:[eE][+-]?\d+)?)(?:_?(?:[iu](?:8|16|32|64|128|size)|f32|f64))?/y,
      ],
      ['word', /[A-Za-z_]\w*/y],
    ],
  },
  json: {
    keywords: jsonKeywords,
    rules: [
      ws,
      lineComment,
      blockComment,
      // Object keys read as keywords; values as strings.
      ['keyword', /"(?:[^"\\\n]|\\.)*"(?=\s*:)/y],
      dq,
      ['number', /-?\d+(?:\.\d+)?(?:[eE][+-]?\d+)?/y],
      ['word', /[A-Za-z_]\w*/y],
    ],
  },
  css: {
    rules: [
      ws,
      blockComment,
      dq,
      sq,
      ['keyword', /@[\w-]+|!important\b/y],
      ['number', /#[\da-fA-F]{3,8}\b/y],
      ['plain', /-?[A-Za-z_][\w-]*/y],
      ['number', /-?(?:\d+\.?\d*|\.\d+)(?:%|[a-zA-Z]+)?/y],
    ],
  },
  md: {
    rules: [
      ['string', /^(`{3,}|~{3,})[^\n]*(?:\n[\s\S]*?(?:\n\1[^\n]*|$(?![\s\S]))|$(?![\s\S]))/my],
      ['keyword', /^#{1,6}(?:[ \t][^\n]*)?$/my],
      ['comment', /^>[^\n]*/my],
      ['number', /^[ \t]*(?:[-*+]|\d+[.)])(?=[ \t])/my],
      ['string', /`[^`\n]+`/y],
      ['string', /\[[^\]\n]*\]\([^)\n]*\)/y],
      // Line breaks on their own, so the line-start rules above still see each line's start.
      ['plain', /[^\s`[#>*+\-\d~]+|[^\S\n]+|\n/y],
    ],
  },
}

/** Splits `text` into coloured tokens. Adjacent plain text is merged. */
export function tokenize(text: string, language: Language): Token[] {
  const out: Token[] = []
  const push = (kind: TokenKind, t: string) => {
    const last = out[out.length - 1]
    if (last && last.kind === kind) last.text += t
    else out.push({ kind, text: t })
  }
  if (language === 'plain') {
    if (text) push('plain', text)
    return out
  }
  const { rules: list, keywords } = rules[language]
  let i = 0
  scan: while (i < text.length) {
    for (const [kind, re] of list) {
      re.lastIndex = i
      const m = re.exec(text)
      if (!m || m[0].length === 0) continue
      const t = m[0]
      push(kind === 'word' ? (keywords?.has(t) ? 'keyword' : 'plain') : kind, t)
      i += t.length
      continue scan
    }
    push('plain', text[i] ?? '')
    i += 1
  }
  return out
}
