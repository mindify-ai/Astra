import type { Person } from './types'

// Sign-in through the relay's OAuth device flow (docs/PROTOCOL.md "Auth").

export type Provider = 'github' | 'gitlab'

export interface Session {
  token: string
  workspaceId: string
  person: Person
  expiresAt: string
}

export interface DeviceCode {
  deviceCode: string
  userCode: string
  verificationUri: string
  verificationUriComplete: string | null
  /** Seconds between polls. */
  interval: number
  /** Seconds until the code expires. */
  expiresIn: number
}

export type PollResult =
  | { status: 'pending' }
  | { status: 'slowDown'; interval: number }
  | { status: 'expired' }
  | { status: 'denied' }
  | { status: 'complete'; session: Session }

export type AuthErrorCode =
  | 'notConfigured'
  /** The server's OAuth app has no device flow enabled (GitHub: "Enable Device Flow"). */
  | 'deviceFlowDisabled'
  /** The server's OAuth client id is unknown to the provider. */
  | 'badClientId'
  /** No GitHub session on this machine (never signed in, or GitHub no longer accepts its token). */
  | 'signedOut'
  | 'badProvider'
  | 'network'
  | 'server'
  | 'expired'
  | 'denied'
  | 'cancelled'

export class AuthError extends Error {
  constructor(
    readonly code: AuthErrorCode,
    message?: string,
  ) {
    super(message ?? code)
    this.name = 'AuthError'
  }
}

/** What the sign-in screen needs; implemented by the real relay client and by a mock for demos and tests. */
export interface Auth {
  start(provider: Provider): Promise<DeviceCode>
  poll(provider: Provider, deviceCode: string): Promise<PollResult>
  devToken?(name: string): Promise<Session>
}

type Fetch = typeof fetch

/**
 * A sync server address as typed (`sync.example.com`, `http://127.0.0.1:8787/`) → its HTTP base URL
 * (`https://sync.example.com`). Hosts without a scheme get https, except localhost. Null if it isn't a URL.
 */
export function serverBaseUrl(input: string): string | null {
  const raw = input.trim()
  if (!raw) return null
  const local = /^(localhost|127\.|\[::1\])/.test(raw)
  const withScheme = /^[a-z][a-z0-9+.-]*:\/\//i.test(raw) ? raw : `${local ? 'http' : 'https'}://${raw}`
  try {
    const url = new URL(withScheme)
    if (url.protocol !== 'http:' && url.protocol !== 'https:') return null
    return url.origin + url.pathname.replace(/\/+$/, '')
  } catch {
    return null
  }
}

export class RelayAuth implements Auth {
  constructor(
    private readonly relayUrl: string,
    private readonly fetchImpl: Fetch = (...args) => fetch(...args),
  ) {}

  private async post<T>(path: string, body: unknown): Promise<T> {
    let res: Response
    try {
      res = await this.fetchImpl(new URL(path, this.relayUrl), {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify(body),
      })
    } catch (e) {
      throw new AuthError('network', `Could not reach the sync server (${String(e)}).`)
    }
    const json = (await res.json().catch(() => ({}))) as { error?: string }
    if (res.status === 503 && json.error === 'notConfigured') throw new AuthError('notConfigured')
    if (res.status === 503 && json.error === 'deviceFlowDisabled') throw new AuthError('deviceFlowDisabled')
    if (res.status === 503 && json.error === 'badClientId') throw new AuthError('badClientId')
    if (res.status === 400 && json.error === 'badProvider') throw new AuthError('badProvider')
    if (!res.ok) throw new AuthError('server', `The sync server answered ${res.status}.`)
    return json as T
  }

  start(provider: Provider) {
    return this.post<DeviceCode>('/v1/auth/device/start', { provider })
  }

  poll(provider: Provider, deviceCode: string) {
    return this.post<PollResult>('/v1/auth/device/poll', { provider, deviceCode })
  }

  async devToken(name: string) {
    return (await this.post<{ session: Session }>('/v1/auth/dev-token', { name })).session
  }
}

const sleepFor = (ms: number, signal?: AbortSignal) =>
  new Promise<void>((resolve, reject) => {
    if (signal?.aborted) return reject(new AuthError('cancelled'))
    const t = setTimeout(resolve, ms)
    signal?.addEventListener(
      'abort',
      () => {
        clearTimeout(t)
        reject(new AuthError('cancelled'))
      },
      { once: true },
    )
  })

/**
 * Polls until the person approves (or denies) the code on the provider's site.
 * Respects the server's interval, backs off on `slowDown`, and gives up when the code expires.
 */
export async function waitForApproval(
  auth: Auth,
  provider: Provider,
  code: DeviceCode,
  opts: {
    signal?: AbortSignal
    sleep?: (ms: number, signal?: AbortSignal) => Promise<void>
    now?: () => number
  } = {},
): Promise<Session> {
  const sleep = opts.sleep ?? sleepFor
  const now = opts.now ?? Date.now
  const deadline = now() + code.expiresIn * 1000
  let interval = Math.max(1, code.interval)
  for (;;) {
    await sleep(interval * 1000, opts.signal)
    if (now() > deadline) throw new AuthError('expired')
    const r = await auth.poll(provider, code.deviceCode)
    if (r.status === 'complete') return r.session
    if (r.status === 'expired') throw new AuthError('expired')
    if (r.status === 'denied') throw new AuthError('denied')
    if (r.status === 'slowDown') interval = Math.max(interval + 5, r.interval)
  }
}

// ── Session storage (per relay) ────────────────────────────────────────────

const KEY = 'astra.session'

export function loadSession(relayUrl: string): Session | null {
  try {
    const raw = localStorage.getItem(KEY)
    if (!raw) return null
    const s = JSON.parse(raw) as Session & { relayUrl?: string }
    if (s.relayUrl !== relayUrl || !s.token || Date.parse(s.expiresAt) <= Date.now()) return null
    return s
  } catch {
    return null
  }
}

export function saveSession(relayUrl: string, session: Session): void {
  try {
    localStorage.setItem(KEY, JSON.stringify({ ...session, relayUrl }))
  } catch {
    /* storage blocked: the session lasts until the tab closes */
  }
}

export function clearSession(): void {
  try {
    localStorage.removeItem(KEY)
  } catch {
    /* nothing stored */
  }
}

/**
 * Simulated device flow for the demo workspace and tests: a fixed code that is approved after a few polls.
 */
export class MockAuth implements Auth {
  private polls = 0
  constructor(
    private readonly person: Person = { kind: 'person', id: 'p-mark', name: 'Mark', initials: 'MC' },
    private readonly approveAfter = 2,
  ) {}

  async start(provider: Provider): Promise<DeviceCode> {
    this.polls = 0
    const host = provider === 'github' ? 'https://github.com/login/device' : 'https://gitlab.com/oauth/device'
    return {
      deviceCode: 'demo-device-code',
      userCode: 'WDJB-MJHT',
      verificationUri: host,
      verificationUriComplete: null,
      interval: 1,
      expiresIn: 900,
    }
  }

  async poll(): Promise<PollResult> {
    this.polls += 1
    if (this.polls < this.approveAfter) return { status: 'pending' }
    return {
      status: 'complete',
      session: {
        token: 'demo',
        workspaceId: 'ws-mindify',
        person: this.person,
        expiresAt: new Date(Date.now() + 86_400_000).toISOString(),
      },
    }
  }

  async devToken(name: string): Promise<Session> {
    return {
      token: 'demo',
      workspaceId: 'ws-dev',
      person: { kind: 'person', id: 'p-dev', name, initials: name.slice(0, 2).toUpperCase() },
      expiresAt: new Date(Date.now() + 86_400_000).toISOString(),
    }
  }
}
