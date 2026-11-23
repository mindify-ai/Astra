import {
  Avatar,
  ChipIcon,
  cx,
  HomeIcon,
  type LinkRenderer,
  ListIcon,
  LogoMark,
  MobileTopBar,
  TabBar,
  UsersIcon,
} from '@astra/ui'
import { useRouter } from '@tanstack/react-router'
import type { ReactNode } from 'react'
import { useAstraState, useConnectivity, usePlatform, useUi } from '../context'
import { currentMissionId } from '../select'

/** Anchor that navigates through the router with a plain path (works with hash or browser history). */
export function useLinkRenderer(): LinkRenderer {
  const router = useRouter()
  return ({ to, className, children, ariaLabel, current }) => (
    <a
      href={router.history.createHref(to)}
      aria-label={ariaLabel}
      aria-current={current ? 'page' : undefined}
      className={className}
      onClick={(e) => {
        if (e.metaKey || e.ctrlKey || e.shiftKey || e.button !== 0) return
        e.preventDefault()
        router.history.push(to)
      }}
    >
      {children}
    </a>
  )
}

export type MobileTab = 'home' | 'missions' | 'agents' | 'people'

export function MobileTabs({ active }: { active: MobileTab }) {
  const { mobileVariant } = usePlatform()
  const s = useAstraState()
  const ui = useUi()
  const render = useLinkRenderer()
  const mission = currentMissionId(s, ui.lastMissionId)
  return (
    <TabBar
      variant={mobileVariant}
      renderLink={render}
      items={[
        { key: 'home', label: 'Home', icon: <HomeIcon />, to: '/', active: active === 'home' },
        {
          key: 'missions',
          label: 'Missions',
          icon: <ListIcon />,
          to: mission ? `/missions/${mission}` : '/board',
          active: active === 'missions',
        },
        { key: 'agents', label: 'Agents', icon: <ChipIcon />, to: '/agents', active: active === 'agents' },
        { key: 'people', label: 'People', icon: <UsersIcon />, to: '/people', active: active === 'people' },
      ]}
    />
  )
}

/** Phone frame: status-bar inset, a top bar, scrolling content and a bottom bar (tabs or a composer). */
export function MobileFrame({
  top,
  bottom,
  children,
  className,
}: {
  top: ReactNode
  bottom?: ReactNode
  children: ReactNode
  className?: string
}) {
  return (
    <div className="flex h-dvh flex-col overflow-hidden bg-surface pt-[env(safe-area-inset-top)] text-ink">
      {top}
      <div className={cx('min-h-0 flex-[1_1_0] overflow-y-auto', className)}>{children}</div>
      {bottom}
    </div>
  )
}

/** Brand bar for top-level tabs: mark on the left, your avatar on the right. */
export function BrandBar() {
  const { mobileVariant } = usePlatform()
  const s = useAstraState()
  const me = s.people[s.me]
  const online = useConnectivity().online
  return (
    <div
      className={cx(
        'flex shrink-0 items-center justify-between px-4',
        mobileVariant === 'ios' ? 'h-11' : 'h-16',
      )}
    >
      <span className="flex items-center gap-3">
        <LogoMark />
        <span className="text-label tracking-[0.22em]">ASTRA</span>
      </span>
      <span className="flex items-center gap-3">
        {!online ? <span className="font-mono text-spec text-ink-muted">OFFLINE</span> : null}
        {me ? <Avatar initials={me.initials} name={`${me.name}, you`} /> : null}
      </span>
    </div>
  )
}

export function MissionTopBar({
  title,
  people,
}: {
  title: string
  people: { initials: string; name: string }[]
}) {
  const { mobileVariant } = usePlatform()
  const render = useLinkRenderer()
  return (
    <MobileTopBar
      variant={mobileVariant}
      title={title}
      back={{ to: '/', label: 'Home' }}
      renderLink={render}
      trailing={
        <span className="flex items-center gap-1">
          {people.slice(0, 3).map((p) => (
            <Avatar key={p.initials} initials={p.initials} name={p.name} size={24} />
          ))}
        </span>
      }
    />
  )
}
