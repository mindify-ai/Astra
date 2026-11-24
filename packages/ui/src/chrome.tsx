import type { ReactNode } from 'react'
import { cx } from './cx'
import { ArrowLeftIcon, ChevronLeftIcon } from './icons'

export type DesktopPlatform = 'mac' | 'windows' | 'linux'
export type MobilePlatform = 'ios' | 'android'

export interface WindowActions {
  onClose?: () => void
  onMinimize?: () => void
  onMaximize?: () => void
}

const Glyph = ({ d, rect }: { d?: string; rect?: boolean }) => (
  <svg
    aria-hidden="true"
    width="10"
    height="10"
    viewBox="0 0 10 10"
    fill="none"
    stroke="currentColor"
    strokeWidth="1"
  >
    {rect ? <rect x="1.5" y="1.5" width="7" height="7" /> : <path d={d} />}
  </svg>
)
const MacGlyph = ({ d, className }: { d: string; className?: string }) => (
  <svg
    aria-hidden="true"
    width="8"
    height="8"
    viewBox="0 0 8 8"
    fill="none"
    stroke="currentColor"
    strokeWidth="1.2"
    strokeLinecap="round"
    className={className}
  >
    <path d={d} />
  </svg>
)
const MIN = 'M1 5h8'
const CLOSE = 'M1.5 1.5l7 7M8.5 1.5l-7 7'

/**
 * Window controls drawn per platform (Platforms artboard):
 * macOS — red, amber and green lights on the left; Windows — 46px flush buttons on the right;
 * Linux — three 24px filled circles on the right.
 */
export function WindowControls({
  platform,
  onClose,
  onMinimize,
  onMaximize,
  height = 56,
}: WindowActions & { platform: DesktopPlatform; height?: number }) {
  if (platform === 'mac') {
    // Filled red / amber / green lights; their glyphs show while the pointer is over the group, as on macOS.
    const dot =
      'box-border flex size-3 items-center justify-center rounded-full border-0 p-0 text-on-window-control'
    const glyph = 'opacity-0 transition-opacity group-hover:opacity-100 group-focus-within:opacity-100'
    return (
      <div className="group flex items-center gap-2">
        <button
          type="button"
          aria-label="Close window"
          className={cx(dot, 'bg-window-close')}
          onClick={onClose}
        >
          <MacGlyph d="M2.5 2.5l3 3M5.5 2.5l-3 3" className={glyph} />
        </button>
        <button
          type="button"
          aria-label="Minimize window"
          className={cx(dot, 'bg-window-minimize')}
          onClick={onMinimize}
        >
          <MacGlyph d="M2 4h4" className={glyph} />
        </button>
        <button
          type="button"
          aria-label="Zoom window"
          className={cx(dot, 'bg-window-zoom')}
          onClick={onMaximize}
        >
          <MacGlyph d="M4 2v4M2 4h4" className={glyph} />
        </button>
      </div>
    )
  }
  if (platform === 'windows') {
    const btn = 'flex w-[46px] items-center justify-center border-0 bg-transparent p-0 text-ink hover:bg-line'
    return (
      <div className="-mr-6 flex" style={{ height }}>
        <button type="button" aria-label="Minimize window" className={btn} onClick={onMinimize}>
          <Glyph d={MIN} />
        </button>
        <button type="button" aria-label="Maximize window" className={btn} onClick={onMaximize}>
          <Glyph rect />
        </button>
        <button
          type="button"
          aria-label="Close window"
          className={cx(btn, 'hover:bg-danger hover:text-on-marking')}
          onClick={onClose}
        >
          <Glyph d={CLOSE} />
        </button>
      </div>
    )
  }
  const btn =
    'flex size-6 items-center justify-center rounded-full border-0 bg-line p-0 text-ink hover:bg-line-strong'
  return (
    <div className="flex items-center gap-2">
      <button type="button" aria-label="Minimize window" className={btn} onClick={onMinimize}>
        <Glyph d={MIN} />
      </button>
      <button type="button" aria-label="Maximize window" className={btn} onClick={onMaximize}>
        <Glyph rect />
      </button>
      <button type="button" aria-label="Close window" className={btn} onClick={onClose}>
        <Glyph d={CLOSE} />
      </button>
    </div>
  )
}

// ── Phone chrome ───────────────────────────────────────────────────────────

export type LinkRenderer = (props: {
  to: string
  className: string
  children: ReactNode
  ariaLabel?: string
  current?: boolean
}) => ReactNode

export interface TabItem {
  key: string
  label: string
  icon: ReactNode
  to?: string
  active?: boolean
  onSelect?: () => void
}

/** iOS tab bar (49px items, 10px labels) or Material navigation bar (56px items, square indicator). */
export function TabBar({
  variant,
  items,
  renderLink,
}: {
  variant: MobilePlatform
  items: TabItem[]
  renderLink: LinkRenderer
}) {
  const ios = variant === 'ios'
  return (
    <nav
      aria-label="Tabs"
      className={cx(
        'flex border-t border-line bg-surface-alt',
        ios ? 'pb-[max(env(safe-area-inset-bottom),0px)]' : 'pt-2 pb-[max(env(safe-area-inset-bottom),8px)]',
      )}
    >
      {items.map((it) => {
        const cls = cx(
          'flex flex-1 flex-col items-center justify-center border-0 bg-transparent no-underline',
          ios ? 'h-[49px] gap-0.5' : 'h-14 gap-1',
          it.active ? 'text-ink' : 'text-ink-muted',
        )
        const body = (
          <>
            {ios ? (
              it.icon
            ) : (
              <span
                className={cx(
                  'flex h-7 w-14 items-center justify-center',
                  it.active ? 'bg-line' : 'bg-transparent',
                )}
              >
                {it.icon}
              </span>
            )}
            <span className={cx('font-medium', ios ? 'text-[10px] leading-3' : 'text-[12px] leading-4')}>
              {it.label}
            </span>
          </>
        )
        if (it.to)
          return (
            <span key={it.key} className="flex flex-1">
              {renderLink({ to: it.to, className: cls, children: body, current: it.active })}
            </span>
          )
        return (
          <button key={it.key} type="button" className={cls} onClick={it.onSelect} aria-pressed={it.active}>
            {body}
          </button>
        )
      })}
    </nav>
  )
}

/** iOS navigation bar (44px, chevron + previous title, centred title) or Material top app bar (64px, icon back, left title). */
export function MobileTopBar({
  variant,
  title,
  back,
  trailing,
  renderLink,
}: {
  variant: MobilePlatform
  title: ReactNode
  back?: { to: string; label: string }
  trailing?: ReactNode
  renderLink: LinkRenderer
}) {
  if (variant === 'ios') {
    return (
      <div className="flex h-11 shrink-0 items-center px-2">
        <div className="flex flex-1 items-center">
          {back
            ? renderLink({
                to: back.to,
                className: 'flex h-11 items-center gap-0.5 pr-2 text-ink no-underline',
                children: (
                  <>
                    <ChevronLeftIcon />
                    {back.label}
                  </>
                ),
              })
            : null}
        </div>
        <span className="text-heading">{title}</span>
        <div className="flex flex-1 items-center justify-end pr-2">{trailing}</div>
      </div>
    )
  }
  return (
    <div className="flex h-16 shrink-0 items-center gap-1 px-1">
      {back
        ? renderLink({
            to: back.to,
            ariaLabel: `Back to ${back.label.toLowerCase()}`,
            className: 'flex size-12 items-center justify-center text-ink no-underline',
            children: <ArrowLeftIcon />,
          })
        : null}
      <span className="grow text-heading">{title}</span>
      <div className="flex items-center pr-3">{trailing}</div>
    </div>
  )
}

/** iOS segmented control: bordered, selected segment is an invert fill. */
export function Segmented<T extends string>({
  options,
  value,
  onChange,
}: {
  options: { value: T; label: string }[]
  value: T
  onChange: (v: T) => void
}) {
  return (
    <div className="flex rounded-sm border border-line-strong">
      {options.map((o) => (
        <button
          key={o.value}
          type="button"
          aria-pressed={o.value === value}
          onClick={() => onChange(o.value)}
          className={cx(
            'h-11 flex-1 border-0 text-label uppercase',
            o.value === value ? 'bg-invert text-on-invert' : 'bg-transparent text-ink',
          )}
        >
          {o.label}
        </button>
      ))}
    </div>
  )
}

/** Material tabs: 48px, 2px signal underline on the active tab. */
export function UnderlineTabs<T extends string>({
  options,
  value,
  onChange,
}: {
  options: { value: T; label: string }[]
  value: T
  onChange: (v: T) => void
}) {
  return (
    <div className="flex border-b border-line" role="tablist">
      {options.map((o) => (
        <button
          key={o.value}
          type="button"
          role="tab"
          aria-selected={o.value === value}
          onClick={() => onChange(o.value)}
          className={cx(
            'h-12 flex-1 border-0 border-b-2 bg-transparent text-label uppercase',
            o.value === value ? 'border-signal text-ink' : 'border-transparent text-ink-muted',
          )}
        >
          {o.label}
        </button>
      ))}
    </div>
  )
}
