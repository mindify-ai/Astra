import type { Person } from '@astra/client'
import { AvatarStack, buttonClass, cx, LogoMark, Select, Spec, WindowControls } from '@astra/ui'
import { Link } from '@tanstack/react-router'
import { type ReactNode, useEffect, useRef, useState } from 'react'
import { useAstraState, useConnectivity, useHost, usePlatform, useUi } from '../context'
import { currentMissionId } from '../select'

export type View = 'thread' | 'map' | 'board'

export interface AppHeaderProps {
  /** Repository switcher in the bar (most screens). */
  repoSelect?: boolean
  /** THREAD / MAP / BOARD links; pass the active one, or null for none active. */
  nav?: View | null
  /** Replace nav with a setup step label ("SET UP · STEP 1 OF 2"). */
  step?: string
  /** Right-aligned label instead of people (Welcome: "BY MINDIFY AI"). */
  byline?: string
  people?: Person[]
  actions?: ReactNode
  border?: boolean
}

const navLink =
  'flex h-14 box-border items-center border-y-2 border-transparent text-label uppercase no-underline'

function AppMenu() {
  const host = useHost()
  const [open, setOpen] = useState(false)
  const ref = useRef<HTMLDivElement>(null)
  useEffect(() => {
    if (!open) return
    const close = (e: MouseEvent) => {
      if (!ref.current?.contains(e.target as Node)) setOpen(false)
    }
    window.addEventListener('mousedown', close)
    return () => window.removeEventListener('mousedown', close)
  }, [open])
  const item = 'flex h-10 items-center px-4 text-ink no-underline hover:bg-surface-raised'
  return (
    <div ref={ref} className="relative">
      <button
        type="button"
        aria-haspopup="menu"
        aria-expanded={open}
        onClick={() => setOpen((o) => !o)}
        className="flex items-center gap-3 border-0 bg-transparent p-0 text-ink"
      >
        <LogoMark />
        <span className="text-label tracking-[0.22em]">ASTRA</span>
      </button>
      {open ? (
        <div
          role="menu"
          className="absolute top-10 left-0 z-50 flex w-56 flex-col border border-line bg-surface-overlay py-2 shadow-lg"
        >
          <Link role="menuitem" to="/" className={item} onClick={() => setOpen(false)}>
            Home
          </Link>
          <Link role="menuitem" to="/missions/new" className={item} onClick={() => setOpen(false)}>
            New mission
          </Link>
          <Link role="menuitem" to="/board" className={item} onClick={() => setOpen(false)}>
            Board
          </Link>
          <Link role="menuitem" to="/setup/agents" className={item} onClick={() => setOpen(false)}>
            Connect agents
          </Link>
          <Link role="menuitem" to="/setup/repo" className={item} onClick={() => setOpen(false)}>
            Repositories
          </Link>
          {host.signOut ? (
            <button
              type="button"
              role="menuitem"
              className={`${item} w-full border-0 border-t border-line bg-transparent text-left`}
              onClick={() => {
                setOpen(false)
                host.signOut?.()
              }}
            >
              Sign out
            </button>
          ) : null}
        </div>
      ) : null}
    </div>
  )
}

export function AppHeader({ repoSelect, nav, step, byline, people, actions, border = true }: AppHeaderProps) {
  const { windowControls, windowActions, platform } = usePlatform()
  const state = useAstraState()
  const ui = useUi()
  const online = useConnectivity().online
  const threadId = currentMissionId(state, ui.lastMissionId)
  const repos = Object.values(state.repositories)

  const navItem = (view: View, label: string) => {
    const active = nav === view
    const cls = cx(navLink, active ? 'border-b-signal text-ink' : 'text-ink-muted hover:text-ink')
    if (view === 'thread') {
      return threadId ? (
        <Link
          to="/missions/$missionId"
          params={{ missionId: threadId }}
          aria-current={active ? 'page' : undefined}
          className={cls}
        >
          {label}
        </Link>
      ) : (
        <span className={cls}>{label}</span>
      )
    }
    if (view === 'map') {
      return threadId ? (
        <Link
          to="/missions/$missionId/map"
          params={{ missionId: threadId }}
          aria-current={active ? 'page' : undefined}
          className={cls}
        >
          {label}
        </Link>
      ) : (
        <span className={cls}>{label}</span>
      )
    }
    return (
      <Link to="/board" aria-current={active ? 'page' : undefined} className={cls}>
        {label}
      </Link>
    )
  }

  // macOS keeps menus in the system menu bar; Windows, Linux and the web open them from the Astra mark.
  const menuFromMark = windowControls === 'windows' || windowControls === 'linux' || platform === 'web'

  return (
    <header
      data-tauri-drag-region
      className={cx(
        'flex min-h-14 shrink-0 flex-wrap items-center gap-x-6 box-border px-6',
        border && 'border-b border-line',
      )}
    >
      {windowControls === 'mac' ? <WindowControls platform="mac" {...windowActions} /> : null}
      {menuFromMark ? (
        <AppMenu />
      ) : (
        <Link to="/" className="flex items-center gap-3 text-ink no-underline">
          <LogoMark />
          <span className="text-label tracking-[0.22em]">ASTRA</span>
        </Link>
      )}
      {repoSelect ? (
        <Select
          aria-label="Repository"
          size="sm"
          ground="page"
          value={ui.repoId}
          onChange={(e) => ui.setRepoId(e.target.value)}
          className="px-1 font-mono text-spec uppercase"
        >
          {repos.map((r) => (
            <option key={r.id} value={r.id}>
              {r.name.replace('/', ' / ').toUpperCase()}
            </option>
          ))}
        </Select>
      ) : null}
      {step ? <Spec>{step}</Spec> : null}
      {nav !== undefined ? (
        <nav aria-label="Views" className="ml-6 flex gap-6">
          {navItem('thread', 'THREAD')}
          {navItem('map', 'MAP')}
          {navItem('board', 'BOARD')}
        </nav>
      ) : null}
      <div data-tauri-drag-region className="h-14 grow" />
      {!online ? (
        <Spec className="flex items-center gap-2" title="Working offline. Changes sync when you reconnect.">
          OFFLINE
        </Spec>
      ) : null}
      {byline ? <Spec>{byline}</Spec> : null}
      {people && people.length > 0 ? <AvatarStack people={people} /> : null}
      {actions}
      {windowControls && windowControls !== 'mac' ? (
        <WindowControls platform={windowControls} {...windowActions} />
      ) : null}
    </header>
  )
}

/** Header buttons: SHARE (outline) and NEW MISSION (primary), both 32px. */
export function ShareButton({ missionId }: { missionId: string | null }) {
  const ui = useUi()
  const state = useAstraState()
  const target =
    missionId && state.missions[missionId] ? missionId : currentMissionId(state, ui.lastMissionId)
  return (
    <button
      type="button"
      className={buttonClass('outline', 'sm')}
      onClick={() => ui.openShare(target)}
      aria-haspopup="dialog"
      disabled={!target}
    >
      SHARE
    </button>
  )
}

export function NewMissionButton() {
  return (
    <Link to="/missions/new" className={buttonClass('primary', 'sm')}>
      NEW MISSION
    </Link>
  )
}

/** Full-height page frame used by every desktop screen. */
export function Page({
  header,
  children,
  className,
}: {
  header: ReactNode
  children: ReactNode
  className?: string
}) {
  return (
    <div className={cx('flex min-h-dvh flex-col bg-surface text-ink', className)}>
      {header}
      {children}
    </div>
  )
}
