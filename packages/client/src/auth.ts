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
  /** The server can't do the one-click redirect sign-in (no client secret): use the device flow instead. */
  | 'redirectUnavailable'
  /** The server refused to send the browser back to this app's address (not an allowed origin). */
  | 'badRedirect'

export class AuthError extends Error {
  constructor(
    readonly code: AuthErrorCode,
    message?: string,
  ) {
    super(message ?? code)
    this.name = 'AuthError'
  }
}

export interface RedirectOptions {
  signal?: AbortSignal
  /** Called with the provider's page once it is open elsewhere (the system browser), to offer reopening it. */
  onOpened?: (url: string) => void
}

/** What the sign-in screen needs; implemented by the real relay client and by a mock for demos and tests. */
export interface Auth {
  start(provider: Provider): Promise<DeviceCode>
  poll(provider: Provider, deviceCode: string): Promise<PollResult>
  devToken?(name: string): Promise<Session>
  /**
   * One-click sign-in: opens the provider's sign-in page and comes back here. Resolves with the session when
   * the result returns to this page (the desktop app), or null when this page navigates away and the result
   * arrives on reload (the web; see `complete`). Rejects with `redirectUnavailable` when only the device flow
   * works.
   */
  redirect?(provider: Provider, opts?: RedirectOptions): Promise<Session | null>
  /** True when this page load is the provider sending the browser back (the web). */
  returning?(): boolean
  /** Finishes a sign-in the provider sent the browser back from; null when this load is not one. */
  complete?(): Promise<Session | null>
}

/**
 * How the browser gets to the provider and back. `prepare` picks the address the relay returns to; `wait` is
 * null when that address is this page itself (the web reloads into `complete`), or resolves with its query when
 * something else receives it (the desktop app's loopback listener).
 */
export interface RedirectDriver {
  prepare(signal?: AbortSignal): Promise<{ redirectUri: string; wait: Promise<URLSearchParams> | null }>
  open(url: string): void
}

const RESULT_PARAMS = ['astraCode', 'authError'] as const
const PENDING_KEY = 'astra.oauth.pending'

/** The web app's driver: leave for the provider in this tab, and come back to this page. */
export const browserRedirect: RedirectDriver = {
  prepare: async () => ({ redirectUri: `${location.origin}${location.pathname}`, wait: null }),
  open: (url) => location.assign(url),
}

const base64url = (bytes: Uint8Array) =>
  btoa(String.fromCharCode(...bytes))
    .replace(/\+/g, '-')
    .replace(/\//g, '_')
    .replace(/=+$/, '')

/** A PKCE verifier (32 random bytes) and its S256 challenge. */
export async function pkcePair(): Promise<{ verifier: string; challenge: string }> {
  const verifier = base64url(crypto.getRandomValues(new Uint8Array(32)))
  const digest = await crypto.subtle.digest('SHA-256', new TextEncoder().encode(verifier))
  return { verifier, challenge: base64url(new Uint8Array(digest)) }
}

const resultError = (code: string): AuthError => {
  switch (code) {
    case 'denied':
    case 'expired':
    case 'notConfigured':
    case 'badClientId':
    case 'network':
      return new AuthError(code)
    default:
      return new AuthError('server', `The sync server reported ${code}.`)
  }
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
  /** One-click sign-in; present only with a redirect driver. */
  readonly redirect?: (provider: Provider, opts?: RedirectOptions) => Promise<Session | null>

  constructor(
    private readonly relayUrl: string,
    private readonly fetchImpl: Fetch = (...args) => fetch(...args),
    driver?: RedirectDriver,
  ) {
    if (driver) this.redirect = (provider, opts) => this.runRedirect(driver, provider, opts)
  }

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
    if (res.status === 503 && json.error === 'redirectNotConfigured')
      throw new AuthError('redirectUnavailable')
    if (res.status === 400 && json.error === 'badRedirect') throw new AuthError('badRedirect')
    if (res.status === 400 && json.error === 'invalidCode') throw new AuthError('expired')
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

  private async exchange(params: URLSearchParams, verifier: string | null): Promise<Session> {
    const error = params.get('authError')
    if (error) throw resultError(error)
    const code = params.get('astraCode')
    if (!code || !verifier) throw new AuthError('expired')
    return (await this.post<{ session: Session }>('/v1/auth/redirect/exchange', { code, verifier })).session
  }

  private async runRedirect(
    driver: RedirectDriver,
    provider: Provider,
    opts: RedirectOptions = {},
  ): Promise<Session | null> {
    const { verifier, challenge } = await pkcePair()
    const target = await driver.prepare(opts.signal)
    const { url } = await this.post<{ url: string }>('/v1/auth/redirect/start', {
      provider,
      redirect: target.redirectUri,
      challenge,
    })
    if (opts.signal?.aborted) throw new AuthError('cancelled')
    if (!target.wait) {
      // This page is about to be replaced: keep the verifier for the page the provider sends the browser back to.
      try {
        sessionStorage.setItem(PENDING_KEY, JSON.stringify({ relayUrl: this.relayUrl, verifier }))
      } catch {
        /* storage blocked: the sign-in can't finish on return, and says so */
      }
      driver.open(url)
      return null
    }
    driver.open(url)
    opts.onOpened?.(url)
    const params = await abortable(target.wait, opts.signal)
    return this.exchange(params, verifier)
  }

  returning(): boolean {
    if (typeof location === 'undefined') return false
    const q = new URLSearchParams(location.search)
    return RESULT_PARAMS.some((k) => q.has(k))
  }

  async complete(): Promise<Session | null> {
    if (!this.returning()) return null
    const params = new URLSearchParams(location.search)
    // Take the result out of the address bar so a reload or a shared link doesn't replay it.
    const clean = new URL(location.href)
    for (const k of RESULT_PARAMS) clean.searchParams.delete(k)
    history.replaceState(history.state, '', clean.toString())
    let verifier: string | null = null
    try {
      const pending = JSON.parse(sessionStorage.getItem(PENDING_KEY) ?? 'null') as {
        relayUrl?: string
        verifier?: string
      } | null
      sessionStorage.removeItem(PENDING_KEY)
      if (pending?.relayUrl === this.relayUrl && typeof pending.verifier === 'string')
        verifier = pending.verifier
    } catch {
      /* nothing kept */
    }
    return this.exchange(params, verifier)
  }
}

const abortable = <T>(p: Promise<T>, signal?: AbortSignal): Promise<T> =>
  !signal
    ? p
    : new Promise<T>((resolve, reject) => {
        if (signal.aborted) return reject(new AuthError('cancelled'))
        signal.addEventListener('abort', () => reject(new AuthError('cancelled')), { once: true })
        p.then(resolve, reject)
      })

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
