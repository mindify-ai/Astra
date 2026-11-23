// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from 'vitest'
import {
  type Auth,
  AuthError,
  clearSession,
  type DeviceCode,
  loadSession,
  type PollResult,
  RelayAuth,
  type Session,
  saveSession,
  waitForApproval,
} from '../src/auth'

const code: DeviceCode = {
  deviceCode: 'dc',
  userCode: 'ABCD-EFGH',
  verificationUri: 'https://github.com/login/device',
  verificationUriComplete: null,
  interval: 5,
  expiresIn: 900,
}
const session: Session = {
  token: 't',
  workspaceId: 'ws-gh-mark',
  person: { kind: 'person', id: 'p-gh-mark', name: 'Mark', initials: 'MC' },
  expiresAt: new Date(Date.now() + 3_600_000).toISOString(),
}

/** Auth whose poll answers come from a script; records the waits between polls. */
function scripted(answers: PollResult[]) {
  const waits: number[] = []
  const auth: Auth = {
    start: async () => code,
    poll: async () => answers.shift() ?? { status: 'pending' },
  }
  const sleep = async (ms: number) => {
    waits.push(ms)
  }
  return { auth, waits, sleep }
}

describe('waitForApproval', () => {
  it('polls at the server interval and backs off on slowDown', async () => {
    const { auth, waits, sleep } = scripted([
      { status: 'pending' },
      { status: 'slowDown', interval: 7 },
      { status: 'complete', session },
    ])
    await expect(waitForApproval(auth, 'github', code, { sleep })).resolves.toEqual(session)
    // 5s, 5s, then max(5 + 5, 7) = 10s after the slow-down.
    expect(waits).toEqual([5000, 5000, 10000])
  })

  it.each([
    ['expired', 'expired'],
    ['denied', 'denied'],
  ] as const)('stops when the provider says %s', async (status, code_) => {
    const { auth, sleep } = scripted([{ status }])
    await expect(waitForApproval(auth, 'github', code, { sleep })).rejects.toMatchObject({ code: code_ })
  })

  it('gives up locally once the code has expired', async () => {
    let t = 0
    const { auth } = scripted([])
    const sleep = async (ms: number) => {
      t += ms
    }
    await expect(
      waitForApproval(auth, 'github', { ...code, expiresIn: 12 }, { sleep, now: () => t }),
    ).rejects.toMatchObject({
      code: 'expired',
    })
  })

  it('can be cancelled', async () => {
    const ctrl = new AbortController()
    const { auth } = scripted([])
    const p = waitForApproval(auth, 'github', { ...code, interval: 1 }, { signal: ctrl.signal })
    ctrl.abort()
    await expect(p).rejects.toBeInstanceOf(AuthError)
  })
})

describe('RelayAuth', () => {
  it('posts the provider and maps responses', async () => {
    const fetchImpl = vi.fn(async (url: URL | RequestInfo, init?: RequestInit) => {
      expect(String(url)).toBe('https://sync.example.com/v1/auth/device/start')
      expect(JSON.parse(String(init?.body))).toEqual({ provider: 'github' })
      return new Response(JSON.stringify(code), { status: 200 })
    })
    const auth = new RelayAuth('https://sync.example.com', fetchImpl as typeof fetch)
    await expect(auth.start('github')).resolves.toEqual(code)
  })

  it('reports a provider that is not configured', async () => {
    const auth = new RelayAuth(
      'https://sync.example.com',
      (async () => new Response(JSON.stringify({ error: 'notConfigured' }), { status: 503 })) as typeof fetch,
    )
    await expect(auth.start('gitlab')).rejects.toMatchObject({ code: 'notConfigured' })
  })

  it('reports network failures', async () => {
    const auth = new RelayAuth('https://sync.example.com', (async () => {
      throw new TypeError('Failed to fetch')
    }) as typeof fetch)
    await expect(auth.poll('github', 'dc')).rejects.toMatchObject({ code: 'network' })
  })
})

describe('session storage', () => {
  afterEach(() => clearSession())

  it('round-trips a session for the same relay only', () => {
    saveSession('https://sync.example.com', session)
    expect(loadSession('https://sync.example.com')?.person.id).toBe('p-gh-mark')
    expect(loadSession('https://other.example.com')).toBeNull()
  })

  it('drops expired sessions', () => {
    saveSession('https://sync.example.com', {
      ...session,
      expiresAt: new Date(Date.now() - 1000).toISOString(),
    })
    expect(loadSession('https://sync.example.com')).toBeNull()
  })
})
