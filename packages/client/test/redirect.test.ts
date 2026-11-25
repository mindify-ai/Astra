// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from 'vitest'
import { AuthError, pkcePair, type RedirectDriver, RelayAuth, type Session } from '../src/auth'

const session: Session = {
  token: 't',
  workspaceId: 'ws-gh-hubot',
  person: { kind: 'person', id: 'p-gh-hubot', name: 'Hu Bot', initials: 'HB' },
  expiresAt: '2099-01-01T00:00:00Z',
}

const sha256url = async (s: string) => {
  const d = new Uint8Array(await crypto.subtle.digest('SHA-256', new TextEncoder().encode(s)))
  return btoa(String.fromCharCode(...d))
    .replace(/\+/g, '-')
    .replace(/\//g, '_')
    .replace(/=+$/, '')
}

/** A relay that answers start and exchange, checking the verifier against the challenge like the real one. */
function fakeRelay(opts: { startError?: [number, string] } = {}) {
  const calls: { path: string; body: Record<string, string> }[] = []
  let challenge = ''
  const fetchImpl = (async (input: URL | RequestInfo, init?: RequestInit) => {
    const path = new URL(String(input)).pathname
    const body = JSON.parse(String(init?.body ?? '{}')) as Record<string, string>
    calls.push({ path, body })
    const reply = (status: number, json: unknown) =>
      new Response(JSON.stringify(json), { status, headers: { 'content-type': 'application/json' } })
    if (path === '/v1/auth/redirect/start') {
      if (opts.startError) return reply(opts.startError[0], { error: opts.startError[1] })
      challenge = body.challenge ?? ''
      return reply(200, { url: 'https://github.com/login/oauth/authorize?state=s' })
    }
    if (path === '/v1/auth/redirect/exchange') {
      const ok = body.code === 'one-time' && (await sha256url(body.verifier ?? '')) === challenge
      return ok ? reply(200, { session }) : reply(400, { error: 'invalidCode' })
    }
    return reply(404, {})
  }) as typeof fetch
  return { fetchImpl, calls }
}

afterEach(() => {
  sessionStorage.clear()
  history.replaceState(null, '', '/')
})

describe('redirect sign-in', () => {
  it('makes an RFC 7636 S256 pair', async () => {
    const { verifier, challenge } = await pkcePair()
    expect(verifier).toMatch(/^[A-Za-z0-9_-]{43}$/)
    expect(challenge).toBe(await sha256url(verifier))
  })

  it('is only offered with a driver', () => {
    expect(new RelayAuth('https://sync.example.com').redirect).toBeUndefined()
  })

  it('on the web: leaves for GitHub, and finishes on the page GitHub sends the browser back to', async () => {
    const relay = fakeRelay()
    const opened: string[] = []
    const driver: RedirectDriver = {
      prepare: async () => ({ redirectUri: 'http://localhost:3000/', wait: null }),
      open: (url) => opened.push(url),
    }
    const auth = new RelayAuth('https://sync.example.com', relay.fetchImpl, driver)
    expect(await auth.redirect?.('github')).toBeNull()
    expect(opened).toEqual(['https://github.com/login/oauth/authorize?state=s'])
    expect(relay.calls[0]?.body).toMatchObject({ provider: 'github', redirect: 'http://localhost:3000/' })

    // GitHub → relay → back here with a one-time code (a fresh page load: a new RelayAuth).
    history.replaceState(null, '', '/?keep=1&astraCode=one-time')
    const back = new RelayAuth('https://sync.example.com', relay.fetchImpl, driver)
    expect(back.returning()).toBe(true)
    expect(await back.complete()).toEqual(session)
    expect(location.search).toBe('?keep=1')
    expect(sessionStorage.getItem('astra.oauth.pending')).toBeNull()
    expect(back.returning()).toBe(false)
    expect(await back.complete()).toBeNull()
  })

  it('on the web: reports a denial, and a return it has no verifier for', async () => {
    const relay = fakeRelay()
    history.replaceState(null, '', '/?authError=denied')
    await expect(new RelayAuth('https://sync.example.com', relay.fetchImpl).complete()).rejects.toMatchObject(
      {
        code: 'denied',
      },
    )
    history.replaceState(null, '', '/?astraCode=one-time')
    await expect(new RelayAuth('https://sync.example.com', relay.fetchImpl).complete()).rejects.toMatchObject(
      {
        code: 'expired',
      },
    )
  })

  it('on the desktop: waits for the loopback to receive the result, then trades it in', async () => {
    const relay = fakeRelay()
    let deliver: (q: URLSearchParams) => void = () => {}
    const opened: string[] = []
    const driver: RedirectDriver = {
      prepare: async () => ({
        redirectUri: 'http://127.0.0.1:53111/callback',
        wait: new Promise((resolve) => {
          deliver = resolve
        }),
      }),
      open: (url) => opened.push(url),
    }
    const auth = new RelayAuth('https://sync.example.com', relay.fetchImpl, driver)
    let shown = ''
    const done = auth.redirect?.('github', { onOpened: (url) => (shown = url) })
    await vi.waitFor(() => expect(opened).toHaveLength(1))
    expect(shown).toBe(opened[0])
    deliver(new URLSearchParams('astraCode=one-time'))
    expect(await done).toEqual(session)
  })

  it('falls back when the relay has no client secret, and can be cancelled', async () => {
    const relay = fakeRelay({ startError: [503, 'redirectNotConfigured'] })
    const driver: RedirectDriver = {
      prepare: async () => ({ redirectUri: 'http://127.0.0.1:1/callback', wait: new Promise(() => {}) }),
      open: () => {},
    }
    await expect(
      new RelayAuth('https://sync.example.com', relay.fetchImpl, driver).redirect?.('github'),
    ).rejects.toMatchObject({ code: 'redirectUnavailable' })

    const ctrl = new AbortController()
    const waiting = new RelayAuth('https://sync.example.com', fakeRelay().fetchImpl, driver).redirect?.(
      'github',
      {
        signal: ctrl.signal,
      },
    )
    await new Promise((r) => setTimeout(r, 0))
    ctrl.abort()
    await expect(waiting).rejects.toBeInstanceOf(AuthError)
  })
})
