import type { ComponentPropsWithoutRef, ElementType, ReactNode } from 'react'
import { cx } from './cx'

// ── Buttons ────────────────────────────────────────────────────────────────
// Square, capitals voice (label: 13/16, 600, 0.14em). Heights follow the size tokens:
// sm 32 (toolbars, cards), md 40 (forms), lg 52 (hero), touch 48 / touchSm 44 (phones).

export type ButtonVariant = 'primary' | 'outline' | 'text'
export type ButtonSize = 'sm' | 'md' | 'lg' | 'touch' | 'touchSm'

const heights: Record<ButtonSize, string> = {
  sm: 'h-8 px-4',
  md: 'h-10 px-5',
  lg: 'h-13 px-7',
  touch: 'h-12 px-5',
  touchSm: 'h-11 px-5',
}

const variants: Record<ButtonVariant, string> = {
  primary: 'bg-invert text-on-invert hover:text-on-invert border-0',
  outline: 'border-2 border-ink text-ink bg-transparent hover:bg-invert hover:text-on-invert',
  text: 'text-ink-muted bg-transparent border-0 px-0 hover:text-ink',
}

/** Class string for anything that should look like a button (also router links). */
export function buttonClass(
  variant: ButtonVariant = 'primary',
  size: ButtonSize = 'md',
  extra?: string,
): string {
  return cx(
    'inline-flex shrink-0 items-center justify-center gap-3 box-border rounded-0 no-underline',
    'text-label uppercase whitespace-nowrap transition-colors duration-100',
    'disabled:cursor-not-allowed disabled:opacity-50',
    heights[size],
    variants[variant],
    extra,
  )
}

export function Button({
  variant = 'primary',
  size = 'md',
  className,
  type = 'button',
  ...props
}: ComponentPropsWithoutRef<'button'> & { variant?: ButtonVariant; size?: ButtonSize }) {
  return <button type={type} className={buttonClass(variant, size, className)} {...props} />
}

/** Sentence-case inline link look: 13px, muted ("+ Add a repository", "Manage →"). */
export const inlineLinkClass = 'text-[13px] leading-5 text-ink-muted no-underline hover:text-ink'

// ── Text voices ────────────────────────────────────────────────────────────

type Polymorphic<T extends ElementType> = { as?: T; className?: string; children?: ReactNode } & Omit<
  ComponentPropsWithoutRef<T>,
  'as' | 'className' | 'children'
>

/** Kicker / eyebrow: 12/16, 500, 0.24em, uppercase, muted. */
export function Eyebrow<T extends ElementType = 'span'>({ as, className, ...props }: Polymorphic<T>) {
  const Tag = (as ?? 'span') as ElementType
  return <Tag className={cx('m-0 text-kicker uppercase text-ink-muted', className)} {...props} />
}

/** Geist Mono spec tag: 11/16, 500, 0.08em, uppercase, muted. */
export function Spec<T extends ElementType = 'span'>({ as, className, ...props }: Polymorphic<T>) {
  const Tag = (as ?? 'span') as ElementType
  return <Tag className={cx('font-mono text-spec uppercase text-ink-muted', className)} {...props} />
}

/** Geist Mono data: 13/20 — identifiers, branches, diff stats. */
export function Data<T extends ElementType = 'span'>({ as, className, ...props }: Polymorphic<T>) {
  const Tag = (as ?? 'span') as ElementType
  return <Tag className={cx('font-mono text-data', className)} {...props} />
}

/** Product page title block: eyebrow, 22/28 title, optional mono meta. */
export function TitleBlock({
  eyebrow,
  title,
  meta,
  titleAs = 'h1',
}: {
  eyebrow?: ReactNode
  title: ReactNode
  meta?: ReactNode
  titleAs?: 'h1' | 'h2'
}) {
  const H = titleAs
  return (
    <div className="flex flex-col gap-2">
      {eyebrow ? <Eyebrow>{eyebrow}</Eyebrow> : null}
      <H className="m-0 text-title">{title}</H>
      {meta ? <Data className="text-ink-muted">{meta}</Data> : null}
    </div>
  )
}

// ── Status ─────────────────────────────────────────────────────────────────
// Status is always a dot and a word.

export type Tone = 'working' | 'success' | 'needsYou' | 'idle' | 'danger'

const toneText: Record<Tone, string> = {
  working: 'text-signal',
  success: 'text-success',
  needsYou: 'text-warning',
  idle: 'text-ink-muted',
  danger: 'text-danger',
}
const toneDot: Record<Tone, string> = {
  working: 'bg-signal',
  success: 'bg-success',
  needsYou: 'bg-warning',
  idle: 'bg-line-strong',
  danger: 'bg-danger',
}

export const toneTextClass = (tone: Tone) => toneText[tone]

export function Dot({ tone, className }: { tone: Tone; className?: string }) {
  return (
    <span
      aria-hidden="true"
      className={cx('inline-block size-1.5 shrink-0 rounded-full', toneDot[tone], className)}
    />
  )
}

/** 13px word with a matching 6px dot. */
export function Status({
  tone,
  children,
  className,
}: {
  tone: Tone
  children: ReactNode
  className?: string
}) {
  return (
    <span className={cx('inline-flex items-center gap-2 text-[13px] leading-5', toneText[tone], className)}>
      <Dot tone={tone} />
      {children}
    </span>
  )
}

/** Mono status line used under list titles: "014 · WORKING". */
export function SpecStatus({ tone, children }: { tone: Tone; children: ReactNode }) {
  return (
    <span
      className={cx(
        'inline-flex items-center gap-2 font-mono text-spec uppercase',
        tone === 'idle' ? 'text-ink-muted' : toneText[tone],
      )}
    >
      <Dot tone={tone} />
      {children}
    </span>
  )
}

export function Badge({ children, className }: { children: ReactNode; className?: string }) {
  return (
    <span
      className={cx(
        'inline-flex items-center rounded-sm bg-signal-soft px-2 py-0.5 font-mono text-spec uppercase text-signal',
        className,
      )}
    >
      {children}
    </span>
  )
}

// ── People and agents ──────────────────────────────────────────────────────

const avatarSizes = {
  28: 'size-7 text-[11px]',
  24: 'size-6 text-[10px]',
  20: 'size-5 text-[9px]',
} as const

/** A person: a white square with initials. */
export function Avatar({
  initials,
  name,
  size = 28,
  className,
}: {
  initials: string
  name?: string
  size?: 28 | 24 | 20
  className?: string
}) {
  return (
    <span
      title={name}
      {...(name ? { role: 'img', 'aria-label': name } : {})}
      className={cx(
        'inline-flex shrink-0 items-center justify-center bg-invert font-semibold tracking-[0.04em] text-on-invert',
        avatarSizes[size],
        className,
      )}
    >
      {initials}
    </span>
  )
}

/** An agent: an outlined square with its role letter in mono. */
export function AgentGlyph({ letter, className }: { letter: string; className?: string }) {
  return (
    <span
      aria-hidden="true"
      className={cx(
        'box-border inline-flex size-7 shrink-0 items-center justify-center border border-line-strong font-mono text-[11px] font-medium',
        className,
      )}
    >
      {letter}
    </span>
  )
}

export function AvatarStack({
  people,
  size = 28,
}: {
  people: { initials: string; name: string }[]
  size?: 28 | 24 | 20
}) {
  return (
    <div className="flex items-center gap-1">
      {people.map((p) => (
        <Avatar key={p.initials} initials={p.initials} name={p.name} size={size} />
      ))}
    </div>
  )
}

// ── Surfaces ───────────────────────────────────────────────────────────────

export function Card({
  emphasis = 'default',
  className,
  ...props
}: ComponentPropsWithoutRef<'article'> & { emphasis?: 'default' | 'strong' | 'quiet' }) {
  return (
    <article
      className={cx(
        'flex flex-col gap-2 border p-4',
        emphasis === 'strong' && 'border-ink bg-surface-raised',
        emphasis === 'default' && 'border-line bg-surface-raised',
        emphasis === 'quiet' && 'border-line bg-transparent',
        className,
      )}
      {...props}
    />
  )
}

/** Kicker section heading with an optional status dot: "NEEDS YOU · 2". */
export function SectionHeading({
  tone,
  children,
  className,
}: {
  tone?: Tone
  children: ReactNode
  className?: string
}) {
  const colored = tone && tone !== 'idle'
  return (
    <h2
      className={cx(
        'm-0 flex items-center gap-2 text-kicker uppercase',
        colored ? toneText[tone] : 'text-ink-muted',
        className,
      )}
    >
      {tone ? <Dot tone={tone} /> : null}
      {children}
    </h2>
  )
}
