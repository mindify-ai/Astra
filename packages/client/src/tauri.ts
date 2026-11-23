import { HlcClock } from './hlc'
import { Store } from './store'
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
