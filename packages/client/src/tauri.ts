import type { RedirectDriver, Session } from './auth'
import type { FileEntry, FilesHost } from './files'
import { HlcClock } from './hlc'
import { Store } from './store'
import type { TerminalHost } from './terminal'
import type { Connectivity, Op, Person, Presence, Snapshot } from './types'

/** The sync server this app signed in to, and as whom. */
export interface DesktopAccount {
  /** The relay's HTTP base URL. */
  server: string
  person: Person
  expiresAt: string
}

export interface NodeInfo {
  nodeId: string
  workspaceId: string
  me: string
  /** Null in a local-only workspace. */
  account: DesktopAccount | null
  /** The GitHub account signed in on this app directly (no sync server), or null. */
  github: { login: string; name: string } | null
}

/**
 * Client for the Tauri shell (desktop, iOS, Android). The Rust core owns storage, sync and (on
 * desktop) the agent runtime; this mirrors its state and forwards ops through Tauri commands.
 */
export class TauriClient extends Store {
  readonly kind = 'tauri' as const
  readonly ready: Promise<void>
  /** Set once `ready` resolves. */
  info: NodeInfo | null = null
  private invoke!: <T>(cmd: string, args?: Record<string, unknown>) => Promise<T>

  constructor() {
    super('pending')
    this.ready = this.init()
  }

  private async init() {
    const [{ invoke }, { listen }] = await Promise.all([
      import('@tauri-apps/api/core'),
      import('@tauri-apps/api/event'),
    ])
    this.invoke = invoke
    const info = await invoke<NodeInfo>('node_info')
    this.info = info
    this.nodeId = info.nodeId
    this.clock = new HlcClock(info.nodeId)
    await listen<{ ops: Op[]; cursor: number }>('astra://ops', (e) => this.receive(e.payload.ops))
    await listen<Presence[]>('astra://presence', (e) => {
      this.state = { ...this.state, presence: e.payload }
      this.emit()
    })
    await listen<Connectivity>('astra://connectivity', (e) => this.setConnectivity(e.payload))
    const snap = await invoke<Snapshot>('snapshot')
    this.load(snap.state)
    this.setConnectivity(await invoke<Connectivity>('connectivity'))
  }

  protected async send(ops: Op[]): Promise<void> {
    await this.invoke('apply', { ops })
  }

  protected override sendPresence(presence: Presence[]): void {
    void this.invoke('set_presence', { presence: presence[0] ?? null })
  }

  /**
   * Signs this app in to a sync server with a session from its device flow. The shell saves it, then restarts
   * on that workspace (a separate local store, so the local-only workspace is never pushed to the server).
   */
  static async signIn(server: string, session: Session): Promise<void> {
    const { invoke } = await import('@tauri-apps/api/core')
    await invoke('sign_in', { server, session })
  }

  /** Signs out of the sync server and restarts on the local-only workspace. */
  static async signOut(): Promise<void> {
    const { invoke } = await import('@tauri-apps/api/core')
    await invoke('sign_out')
  }

  /** Desktop window controls for the frameless title bar. */
  static async windowAction(action: 'close' | 'minimize' | 'toggleMaximize') {
    const { getCurrentWindow } = await import('@tauri-apps/api/window')
    const w = getCurrentWindow()
    if (action === 'close') await w.close()
    else if (action === 'minimize') await w.minimize()
    else await w.toggleMaximize()
  }
}

/** True when running inside a Tauri webview. */
export const isTauri = (): boolean => typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window

/**
 * The desktop app's one-click sign-in to a sync server: GitHub opens in the system browser (`openUrl`), and the
 * relay sends the browser back to a loopback port the Rust shell listens on (RFC 8252), which hands the result
 * to the webview and brings the window to the front.
 */
export function tauriLoopback(openUrl: (url: string) => void): RedirectDriver {
  return {
    async prepare(signal) {
      const [{ invoke }, { listen }] = await Promise.all([
        import('@tauri-apps/api/core'),
        import('@tauri-apps/api/event'),
      ])
      let port: number | null = null
      const early: { port: number; query: string }[] = []
      let deliver: (q: URLSearchParams) => void = () => {}
      const wait = new Promise<URLSearchParams>((resolve) => {
        deliver = resolve
      })
      // Listen first; the result can only come after the browser opens, but a listener costs nothing.
      const unlisten = await listen<{ port: number; query: string }>('astra://oauth', (e) => {
        if (port === null) early.push(e.payload)
        else if (e.payload.port === port) {
          unlisten()
          deliver(new URLSearchParams(e.payload.query))
        }
      })
      signal?.addEventListener('abort', () => unlisten(), { once: true })
      try {
        port = await invoke<number>('oauth_loopback')
      } catch (err) {
        unlisten()
        throw err
      }
      const mine = early.find((x) => x.port === port)
      if (mine) {
        unlisten()
        deliver(new URLSearchParams(mine.query))
      }
      return { redirectUri: `http://127.0.0.1:${port}/callback`, wait }
    },
    open: openUrl,
  }
}

/** The desktop app's terminal: a login shell in a pseudo-terminal, run by the Rust shell. */
export const tauriTerminal: TerminalHost = {
  async open({ cwd, cols, rows, onData, onExit }) {
    const [{ invoke }, { listen }] = await Promise.all([
      import('@tauri-apps/api/core'),
      import('@tauri-apps/api/event'),
    ])
    const id = crypto.randomUUID()
    // Listen first: the shell may print before `terminal_open` returns.
    const unlisten = await Promise.all([
      listen<string>(`astra://terminal/${id}/data`, (e) => onData(e.payload)),
      listen<number | null>(`astra://terminal/${id}/exit`, (e) => {
        for (const u of unlisten) u()
        onExit(e.payload)
      }),
    ])
    try {
      await invoke('terminal_open', { id, cwd, cols, rows })
    } catch (err) {
      for (const u of unlisten) u()
      throw err
    }
    return {
      write: (data) => void invoke('terminal_write', { id, data }).catch(() => {}),
      resize: (c, r) => void invoke('terminal_resize', { id, cols: c, rows: r }).catch(() => {}),
      close: () => void invoke('terminal_close', { id }).catch(() => {}),
    }
  },
}

/** The desktop app's file access for the code editor. The Rust shell confines every path to `root`. */
export const tauriFiles: FilesHost = {
  async list(root, dir) {
    const { invoke } = await import('@tauri-apps/api/core')
    return invoke<FileEntry[]>('files_list', { root, dir })
  },
  async read(root, path) {
    const { invoke } = await import('@tauri-apps/api/core')
    return invoke<string>('files_read', { root, path })
  },
  async write(root, path, text) {
    const { invoke } = await import('@tauri-apps/api/core')
    await invoke('files_write', { root, path, text })
  },
  async base(root, path) {
    const { invoke } = await import('@tauri-apps/api/core')
    return invoke<string | null>('files_base', { root, path })
  },
}
