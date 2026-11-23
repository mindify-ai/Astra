import { createStore, del, get, set, type UseStore } from 'idb-keyval'
import { emptyState } from './reducer'
import { Store } from './store'
import type { Op, Presence } from './types'

export interface RemoteOptions {
  /** Relay base URL, e.g. https://sync.astra.example or ws://192.168.1.20:7420 for a LAN desktop. */
  url: string
  token: string
  workspaceId: string
  me: string
  nodeId: string
  /** Called when the relay rejects the token (expired or revoked): the app should sign in again. */
  onUnauthorized?: () => void
}

type ServerFrame =
  | { t: 'welcome'; cursor: number }
  | { t: 'ops'; ops: Op[]; cursor: number }
  | { t: 'ack'; ids: string[] }
  | { t: 'presence'; presence: Presence[] }
  | { t: 'error'; message: string }

/**
 * Web SaaS (and LAN) client: talks the sync protocol over WebSocket (JSON frames). Every op is
 * cached in IndexedDB, and ops made while offline queue there until the relay acknowledges them.
 */
export class RemoteClient extends Store {
  readonly kind = 'remote' as const
  readonly ready: Promise<void>
  private ws: WebSocket | null = null
  private cursor = 0
  private log: Op[] = []
  private pending: Op[] = []
  private backoff = 500
  private closed = false
  private readonly db: UseStore

  constructor(private readonly opts: RemoteOptions) {
    super(opts.nodeId)
    this.db = createStore(`astra-${opts.workspaceId}`, 'sync')
    this.connectivity = {
      online: typeof navigator === 'undefined' ? true : navigator.onLine,
      relay: 'disconnected',
    }
    this.ready = this.init()
  }

  private async init() {
    this.state = emptyState(this.opts.workspaceId, this.opts.me)
    const [log, pending, cursor] = await Promise.all([
      get<Op[]>('log', this.db),
      get<Op[]>('pending', this.db),
      get<number>('cursor', this.db),
    ])
    this.log = log ?? []
    this.pending = pending ?? []
    this.cursor = cursor ?? 0
    this.receive([...this.log, ...this.pending])
    this.emit()
    if (typeof window !== 'undefined') {
      window.addEventListener('online', () => this.connect())
      window.addEventListener('offline', () => this.setConnectivity({ online: false, relay: 'disconnected' }))
    }
    this.connect()
  }

  close() {
    this.closed = true
    this.ws?.close()
  }

  /** Drops the local cache (sign out). */
  async forget() {
    await Promise.all([del('log', this.db), del('pending', this.db), del('cursor', this.db)])
  }

  private connect() {
    if (this.closed || (this.ws && this.ws.readyState <= 1)) return
    const url = new URL('/v1/sync', this.opts.url)
    url.protocol = url.protocol.replace('http', 'ws')
    url.searchParams.set('format', 'json')
    const ws = new WebSocket(url)
    this.ws = ws
    ws.onopen = () => {
      this.backoff = 500
      ws.send(
        JSON.stringify({
          t: 'hello',
          workspaceId: this.opts.workspaceId,
          nodeId: this.opts.nodeId,
          cursor: this.cursor,
          token: this.opts.token,
        }),
      )
    }
    ws.onmessage = (ev) => this.onFrame(JSON.parse(String(ev.data)) as ServerFrame)
    ws.onclose = () => {
      this.ws = null
      this.setConnectivity({ online: navigator.onLine, relay: 'disconnected' })
      if (!this.closed) {
        setTimeout(() => this.connect(), this.backoff)
        this.backoff = Math.min(this.backoff * 2, 30_000)
      }
    }
  }

  private onFrame(f: ServerFrame) {
    if (f.t === 'welcome') {
      this.setConnectivity({ online: true, relay: 'connected' })
      this.flush()
    } else if (f.t === 'ops') {
      this.receive(f.ops)
      const known = new Set(this.log.map((o) => o.id))
      this.log.push(...f.ops.filter((o) => !known.has(o.id)))
      this.cursor = Math.max(this.cursor, f.cursor)
      void set('log', this.log, this.db)
      void set('cursor', this.cursor, this.db)
    } else if (f.t === 'ack') {
      const acked = new Set(f.ids)
      this.pending = this.pending.filter((o) => !acked.has(o.id))
      void set('pending', this.pending, this.db)
    } else if (f.t === 'presence') {
      this.state = { ...this.state, presence: f.presence }
      this.emit()
    } else if (f.t === 'error' && f.message === 'unauthorized') {
      this.closed = true
      this.ws?.close()
      this.setConnectivity({ online: navigator.onLine, relay: 'disconnected' })
      this.opts.onUnauthorized?.()
    }
  }

  private flush() {
    if (this.ws?.readyState === 1 && this.pending.length > 0)
      this.ws.send(JSON.stringify({ t: 'push', ops: this.pending }))
  }

  protected async send(ops: Op[]): Promise<void> {
    this.pending.push(...ops)
    await set('pending', this.pending, this.db)
    this.flush()
  }

  protected override sendPresence(presence: Presence[]): void {
    if (this.ws?.readyState === 1) this.ws.send(JSON.stringify({ t: 'presence', presence }))
  }
}
