import type {
  AstraClient,
  Auth,
  Connectivity,
  FilesHost,
  Person,
  Session,
  State,
  TerminalHost,
} from '@astra/client'
import type { DesktopPlatform, MobilePlatform, WindowActions } from '@astra/ui'
import {
  createContext,
  type ReactNode,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useState,
  useSyncExternalStore,
} from 'react'

// ── Client ─────────────────────────────────────────────────────────────────

const ClientContext = createContext<AstraClient | null>(null)

export function useClient(): AstraClient {
  const c = useContext(ClientContext)
  if (!c) throw new Error('useClient outside <AstraProvider>')
  return c
}

/** The current state; re-renders when it changes (the reducer keeps unchanged collections by reference). */
export function useAstraState(): State {
  const client = useClient()
  return useSyncExternalStore(client.subscribe, client.getState)
}

/** Derives a value from state. Selectors run on each render, so keep them cheap. */
export function useAstra<T>(selector: (s: State) => T): T {
  return selector(useAstraState())
}

export function useConnectivity(): Connectivity {
  const client = useClient()
  return useSyncExternalStore(client.subscribe, client.getConnectivity)
}

// ── Platform ───────────────────────────────────────────────────────────────

export type Platform = DesktopPlatform | MobilePlatform | 'web'

export interface PlatformInfo {
  platform: Platform
  /** Which set of screens to render. */
  layout: 'desktop' | 'mobile'
  /** Window controls to draw in the title bar, or null (web, phones). */
  windowControls: DesktopPlatform | null
  mobileVariant: MobilePlatform
  windowActions: WindowActions
}

const PlatformContext = createContext<PlatformInfo>({
  platform: 'web',
  layout: 'desktop',
  windowControls: null,
  mobileVariant: 'android',
  windowActions: {},
})

export const usePlatform = () => useContext(PlatformContext)

function useNarrow(): boolean {
  const query = '(max-width: 639px)'
  const get = useCallback(
    () => (typeof window !== 'undefined' && window.matchMedia ? window.matchMedia(query).matches : false),
    [],
  )
  const [narrow, setNarrow] = useState(get)
  useEffect(() => {
    if (!window.matchMedia) return
    const m = window.matchMedia(query)
    const on = () => setNarrow(m.matches)
    m.addEventListener('change', on)
    return () => m.removeEventListener('change', on)
  }, [])
  return narrow
}

// ── Host capabilities (provided by the desktop/mobile shell; absent on the web) ──

/** Signing this app in to a sync server (the desktop app; the web app signs in before it opens). */
export interface HostAccount {
  /** The server signed in to, or the one to offer; empty when none is known. */
  server: string
  /** Who is signed in; null in a local-only workspace. */
  person: Person | null
  /** The OAuth device flow, run through `server`. */
  auth: (server: string) => Auth
  /** Saves the session and reopens the app on that server's workspace. */
  signIn: (server: string, session: Session) => Promise<void>
}

export interface HostCapabilities {
  /** Native folder picker; returns an absolute path. */
  pickFolder?: () => Promise<string | null>
  /** Copies text to the clipboard. */
  copy?: (text: string) => Promise<void>
  /** Opens a web page in the default browser (the desktop webview cannot open one itself). */
  openUrl?: (url: string) => void
  /** Ends the session (signed in to a relay). */
  signOut?: () => void
  /** Signs in to a sync server from inside the app (desktop app). */
  account?: HostAccount
  /** Runs shells on this machine for the terminal dock (desktop app). */
  terminal?: TerminalHost
  /** Reads and writes the repository's files for the code editor (desktop app). */
  files?: FilesHost
}

const HostContext = createContext<HostCapabilities>({})
export const useHost = () => useContext(HostContext)

// ── UI preferences shared across screens (current repo, last mission, share panel) ──

interface UiState {
  repoId: string
  setRepoId: (id: string) => void
  lastMissionId: string | null
  setLastMissionId: (id: string) => void
  shareMissionId: string | null
  openShare: (missionId: string | null) => void
  terminalOpen: boolean
  setTerminalOpen: (open: boolean) => void
  paletteOpen: boolean
  setPaletteOpen: (open: boolean) => void
}

const UiContext = createContext<UiState | null>(null)

export function useUi(): UiState {
  const u = useContext(UiContext)
  if (!u) throw new Error('useUi outside <AstraProvider>')
  return u
}

const read = (k: string): string | null => {
  try {
    return localStorage.getItem(k)
  } catch {
    return null
  }
}
const write = (k: string, v: string) => {
  try {
    localStorage.setItem(k, v)
  } catch {
    /* storage unavailable: preference lives for this session only */
  }
}

export function AstraProvider({
  client,
  platform,
  windowActions = {},
  host = {},
  children,
}: {
  client: AstraClient
  platform: Platform
  windowActions?: WindowActions
  host?: HostCapabilities
  children: ReactNode
}) {
  const narrow = useNarrow()
  const info = useMemo<PlatformInfo>(() => {
    const mobile = platform === 'ios' || platform === 'android'
    const ua = typeof navigator !== 'undefined' ? navigator.userAgent : ''
    return {
      platform,
      layout: mobile || (platform === 'web' && narrow) ? 'mobile' : 'desktop',
      windowControls: platform === 'mac' || platform === 'windows' || platform === 'linux' ? platform : null,
      mobileVariant: platform === 'ios' || (platform === 'web' && /iPhone|iPad/.test(ua)) ? 'ios' : 'android',
      windowActions,
    }
  }, [platform, narrow, windowActions])

  const [savedRepoId, setRepo] = useState(() => read('astra.repo') ?? '')
  const repositories = useSyncExternalStore(client.subscribe, () => client.getState().repositories)
  // A remembered repository may not exist in this workspace (or there are none yet): use the first one.
  const repoId = repositories[savedRepoId] ? savedRepoId : (Object.keys(repositories)[0] ?? '')
  const [lastMissionId, setLast] = useState(() => read('astra.lastMission'))
  const [shareMissionId, openShare] = useState<string | null>(null)
  const [terminalOpen, setTerminalOpen] = useState(false)
  const [paletteOpen, setPaletteOpen] = useState(false)
  const ui = useMemo<UiState>(
    () => ({
      repoId,
      setRepoId: (id) => {
        setRepo(id)
        write('astra.repo', id)
      },
      lastMissionId,
      setLastMissionId: (id) => {
        setLast(id)
        write('astra.lastMission', id)
      },
      shareMissionId,
      openShare,
      terminalOpen,
      setTerminalOpen,
      paletteOpen,
      setPaletteOpen,
    }),
    [repoId, lastMissionId, shareMissionId, terminalOpen, paletteOpen],
  )

  return (
    <ClientContext.Provider value={client}>
      <PlatformContext.Provider value={info}>
        <HostContext.Provider value={host}>
          <UiContext.Provider value={ui}>{children}</UiContext.Provider>
        </HostContext.Provider>
      </PlatformContext.Provider>
    </ClientContext.Provider>
  )
}
