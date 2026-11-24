import type { ReactNode } from 'react'
import { cx } from './cx'

/**
 * File-type icons for the explorer, tabs and file lists. Code files get a bold mono monogram ("TS", "RS"),
 * everything else a 1.5px stroke glyph. Colours are token classes, so they follow the theme; the file name
 * beside the icon always carries the meaning (icons are decorative).
 */

type Glyph = { mono: string } | { draw: ReactNode }
interface IconSpec {
  glyph: Glyph
  tone: string
}

const stroke = {
  fill: 'none',
  stroke: 'currentColor',
  strokeWidth: 1.5,
  strokeLinecap: 'round' as const,
  strokeLinejoin: 'round' as const,
}

const page = (
  <>
    <path {...stroke} d="M4 1.75h5.25L12.5 5v9.25H4z" />
    <path {...stroke} d="M9 1.75V5.25h3.5" />
  </>
)
const draw = {
  page,
  text: (
    <>
      {page}
      <path {...stroke} d="M6 8h4.5M6 10.5h4.5M6 13h2.5" strokeWidth={1.2} />
    </>
  ),
  image: (
    <>
      <rect {...stroke} x="2" y="3" width="12" height="10" rx="0.5" />
      <circle {...stroke} cx="6" cy="6.5" r="1.25" strokeWidth={1.2} />
      <path {...stroke} d="M2.5 12l3.5-3.5 2.5 2.5 2-2 3 3" strokeWidth={1.2} />
    </>
  ),
  gear: (
    <>
      <circle {...stroke} cx="8" cy="8" r="2.25" />
      <path
        {...stroke}
        d="M8 1.75v2M8 12.25v2M1.75 8h2M12.25 8h2M3.6 3.6l1.4 1.4M11 11l1.4 1.4M3.6 12.4L5 11M11 5l1.4-1.4"
      />
    </>
  ),
  lock: (
    <>
      <rect {...stroke} x="3.25" y="7" width="9.5" height="7" rx="0.5" />
      <path {...stroke} d="M5.25 7V5a2.75 2.75 0 015.5 0v2" />
    </>
  ),
  git: (
    <>
      <circle {...stroke} cx="5" cy="3.5" r="1.5" />
      <circle {...stroke} cx="5" cy="12.5" r="1.5" />
      <circle {...stroke} cx="11.5" cy="6" r="1.5" />
      <path {...stroke} d="M5 5v6M11.5 7.5c0 2.5-2 3-6.5 3.5" />
    </>
  ),
  terminal: (
    <>
      <rect {...stroke} x="1.75" y="2.75" width="12.5" height="10.5" rx="0.5" />
      <path {...stroke} d="M4.5 6.5L6.5 8l-2 1.5M8 10.5h3.5" />
    </>
  ),
  book: (
    <>
      <path {...stroke} d="M2.5 3.25h4a1.5 1.5 0 011.5 1.5v8.5a1.25 1.25 0 00-1.25-1.25H2.5z" />
      <path {...stroke} d="M13.5 3.25h-4A1.5 1.5 0 008 4.75v8.5a1.25 1.25 0 011.25-1.25h4.25z" />
    </>
  ),
  folder: (
    <path
      {...stroke}
      d="M1.75 4.25a1 1 0 011-1h3.5l1.5 1.75h5.5a1 1 0 011 1v6.75a1 1 0 01-1 1h-10.5a1 1 0 01-1-1z"
    />
  ),
  folderOpen: (
    <>
      <path {...stroke} d="M1.75 12.25V4.25a1 1 0 011-1h3.5l1.5 1.75h4.5a1 1 0 011 1V7" />
      <path {...stroke} d="M1.75 12.25L3.6 7.6a1 1 0 01.93-.6h9.22a.6.6 0 01.56.82l-1.86 4.43z" />
    </>
  ),
}

const mono = (text: string, tone: string): IconSpec => ({ glyph: { mono: text }, tone })
const glyph = (d: ReactNode, tone: string): IconSpec => ({ glyph: { draw: d }, tone })

const BLUE = 'text-chart-3'
const AMBER = 'text-warning'
const RED = 'text-danger'
const GREEN = 'text-success'
const MUTED = 'text-ink-muted'
const PERIWINKLE = 'text-marking-internal'
const ORANGE = 'text-marking-confidential'

const byExtension: Record<string, IconSpec> = {
  ts: mono('TS', BLUE),
  mts: mono('TS', BLUE),
  cts: mono('TS', BLUE),
  tsx: mono('TSX', BLUE),
  js: mono('JS', AMBER),
  mjs: mono('JS', AMBER),
  cjs: mono('JS', AMBER),
  jsx: mono('JSX', AMBER),
  json: mono('{}', AMBER),
  jsonc: mono('{}', AMBER),
  json5: mono('{}', AMBER),
  css: mono('#', BLUE),
  scss: mono('#', RED),
  less: mono('#', BLUE),
  html: mono('<>', ORANGE),
  htm: mono('<>', ORANGE),
  xml: mono('<>', ORANGE),
  vue: mono('V', GREEN),
  svelte: mono('S', ORANGE),
  md: glyph(draw.book, PERIWINKLE),
  mdx: glyph(draw.book, PERIWINKLE),
  markdown: glyph(draw.book, PERIWINKLE),
  rs: mono('RS', ORANGE),
  py: mono('PY', BLUE),
  pyi: mono('PY', BLUE),
  go: mono('GO', BLUE),
  java: mono('J', RED),
  kt: mono('KT', PERIWINKLE),
  swift: mono('SW', ORANGE),
  rb: mono('RB', RED),
  php: mono('PHP', PERIWINKLE),
  c: mono('C', BLUE),
  h: mono('H', PERIWINKLE),
  cpp: mono('C++', BLUE),
  cs: mono('C#', GREEN),
  sql: mono('SQL', AMBER),
  graphql: mono('GQ', RED),
  proto: mono('PB', MUTED),
  tf: mono('TF', PERIWINKLE),
  hcl: mono('TF', PERIWINKLE),
  sh: glyph(draw.terminal, GREEN),
  bash: glyph(draw.terminal, GREEN),
  zsh: glyph(draw.terminal, GREEN),
  fish: glyph(draw.terminal, GREEN),
  yaml: glyph(draw.gear, PERIWINKLE),
  yml: glyph(draw.gear, PERIWINKLE),
  toml: glyph(draw.gear, MUTED),
  ini: glyph(draw.gear, MUTED),
  conf: glyph(draw.gear, MUTED),
  env: glyph(draw.gear, AMBER),
  lock: glyph(draw.lock, MUTED),
  png: glyph(draw.image, PERIWINKLE),
  jpg: glyph(draw.image, PERIWINKLE),
  jpeg: glyph(draw.image, PERIWINKLE),
  gif: glyph(draw.image, PERIWINKLE),
  webp: glyph(draw.image, PERIWINKLE),
  ico: glyph(draw.image, PERIWINKLE),
  svg: glyph(draw.image, AMBER),
  txt: glyph(draw.text, MUTED),
  log: glyph(draw.text, MUTED),
  csv: glyph(draw.text, GREEN),
  pdf: glyph(draw.text, RED),
}

const byName: Record<string, IconSpec> = {
  'package.json': mono('{}', GREEN),
  'tsconfig.json': mono('TS', MUTED),
  'cargo.toml': mono('RS', MUTED),
  'cargo.lock': glyph(draw.lock, MUTED),
  'pnpm-lock.yaml': glyph(draw.lock, MUTED),
  'package-lock.json': glyph(draw.lock, MUTED),
  'yarn.lock': glyph(draw.lock, MUTED),
  dockerfile: mono('DK', BLUE),
  makefile: glyph(draw.terminal, MUTED),
  license: glyph(draw.text, AMBER),
  'license.md': glyph(draw.text, AMBER),
  'readme.md': glyph(draw.book, BLUE),
  '.gitignore': glyph(draw.git, RED),
  '.gitattributes': glyph(draw.git, RED),
  '.gitmodules': glyph(draw.git, RED),
  '.editorconfig': glyph(draw.gear, MUTED),
  '.npmrc': glyph(draw.gear, RED),
  '.dockerignore': glyph(draw.gear, BLUE),
}

function specFor(name: string): IconSpec {
  const lower = name.toLowerCase()
  const named = byName[lower]
  if (named) return named
  if (lower.startsWith('.env')) return glyph(draw.gear, AMBER)
  if (lower.startsWith('dockerfile')) return mono('DK', BLUE)
  if (lower.endsWith('.d.ts')) return mono('TS', MUTED)
  if (/\.(test|spec)\.[jt]sx?$/.test(lower))
    return mono(lower.match(/\.([jt])sx?$/)?.[1] === 't' ? 'TS' : 'JS', GREEN)
  const ext = lower.includes('.') ? lower.slice(lower.lastIndexOf('.') + 1) : ''
  return byExtension[ext] ?? glyph(draw.page, MUTED)
}

/** An icon for a file (by name) or a folder (open or closed). Decorative: pair it with the name. */
export function FileIcon({
  name,
  folder,
  open,
  size = 16,
  className,
}: {
  name: string
  folder?: boolean
  open?: boolean
  size?: number
  className?: string
}) {
  const spec = folder
    ? glyph(open ? draw.folderOpen : draw.folder, name.startsWith('.') ? MUTED : 'text-ink-muted')
    : specFor(name)
  const g = spec.glyph
  if ('mono' in g) {
    // Drawn by CSS from a data attribute, so the badge adds no text to the row (names, copy and paste).
    const len = g.mono.length
    return (
      <span
        aria-hidden="true"
        data-icon={g.mono}
        style={{ width: size, height: size, fontSize: size * (len >= 3 ? 0.36 : len === 2 ? 0.48 : 0.64) }}
        className={cx(
          'inline-flex shrink-0 items-center justify-center font-mono leading-none font-bold tracking-[-0.04em] before:content-[attr(data-icon)]',
          spec.tone,
          className,
        )}
      />
    )
  }
  return (
    <svg
      aria-hidden="true"
      width={size}
      height={size}
      viewBox="0 0 16 16"
      className={cx('shrink-0', spec.tone, className)}
    >
      {g.draw}
    </svg>
  )
}
