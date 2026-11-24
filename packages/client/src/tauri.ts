import type { FileEntry, FilesHost } from './files'
import { HlcClock } from './hlc'
import { Store } from './store'
import type { TerminalHost } from './terminal'
import type { Connectivity, Op, Presence, Snapshot } from './types'

interface NodeInfo {
  nodeId: string
  workspaceId: string
  me: string
}

/**
 * Client for the Tauri shell (desktop, iOS, Android). The Rust core owns storage, sync and (on
 * desktop) the agent runtime; this mirrors its state and forwards ops through Tauri commands.
 */
export class TauriClient extends Store {
  readonly kind = 'tauri' as const
  readonly ready: Promise<void>
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
