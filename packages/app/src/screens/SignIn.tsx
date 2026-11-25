import {
  type Auth,
  AuthError,
  type DeviceCode,
  type Provider,
  type Session,
  waitForApproval,
} from '@astra/client'
import { Button, buttonClass, Eyebrow, Field, LogoMark, Spec, Status, TextInput } from '@astra/ui'
import { type ReactNode, useEffect, useRef, useState } from 'react'

const providerName: Record<Provider, string> = { github: 'GitHub', gitlab: 'GitLab' }

type Step =
  | { kind: 'idle' }
  | { kind: 'starting'; provider: Provider }
  | { kind: 'code'; provider: Provider; code: DeviceCode; issuedAt: number }
  /** The provider's page is open in the system browser; waiting for it to send the result back (desktop). */
  | { kind: 'browser'; provider: Provider; url: string }
  /** The provider sent this page back; trading the result in for a session (web). */
  | { kind: 'returning' }
  | { kind: 'done'; session: Session }
  | { kind: 'error'; provider: Provider | null; message: string }

function explain(e: unknown, provider: Provider | null): string {
  const name = provider ? providerName[provider] : 'that provider'
  if (!(e instanceof AuthError)) return 'Astra could not sign you in. Try again in a moment.'
  switch (e.code) {
    case 'notConfigured':
      return `Sign-in with ${name} is not set up on this server yet.`
    case 'deviceFlowDisabled':
      return `Sign-in with ${name} is turned off on this server: its ${name} OAuth app needs device flow enabled.`
    case 'badClientId':
      return `This server's ${name} OAuth app is not set up correctly: ${name} does not know its client ID.`
    case 'network':
      return 'Astra could not reach its server. Check your connection and try again.'
    case 'expired':
      return 'The sign-in expired before it finished. Start again.'
    case 'denied':
      return `Sign-in was cancelled on ${name}.`
    case 'badRedirect':
      return `This sync server doesn't accept sign-ins from this address. Ask whoever runs it to add it to ASTRA_ALLOWED_ORIGINS.`
    default:
      return 'Astra could not sign you in. Try again in a moment.'
  }
}

const mmss = (secs: number) =>
  `${String(Math.floor(secs / 60)).padStart(2, '0')}:${String(secs % 60).padStart(2, '0')}`

export interface SignInProps {
  auth: Auth
  /** May return a promise (the desktop app saves the session); a rejection shows as an error. */
  onSignedIn: (session: Session) => void | Promise<void>
  /** Providers offered, in order. The first is the primary button. GitHub only for now; the relay also speaks GitLab. */
  providers?: Provider[]
  /** Where "Get the desktop app" points; local folders need the desktop app. */
  desktopUrl?: string
  /** Show the developer-token form (local relay with ASTRA_DEV_AUTH=1). */
  devAuth?: boolean
  /** Opens the provider's page; injectable for tests and for the desktop app's browser opener. */
  openUrl?: (url: string) => void
  /** Copies the code; defaults to the browser clipboard. */
  copyText?: (text: string) => Promise<void>
  /** An editable sync server address (the desktop app, which can sign in to any relay). */
  server?: { value: string; onChange: (value: string) => void; valid?: boolean; optional?: boolean }
  /** What approving lets Astra do, after "to let Astra" (default: see your name and email). */
  purpose?: string
  /**
   * Opens the provider's page (and copies the code) as soon as a device code arrives, instead of waiting for
   * "Open GitHub". For hosts whose `openUrl` needs no click to be allowed (the desktop app's system browser).
   */
  openOnCode?: boolean
  /** Starts signing in with this provider right away (arriving from "Continue with GitHub"). */
  autoStart?: Provider
  /** Replaces "Get the desktop app" (the desktop app offers to stay local instead). */
  localAction?: ReactNode
  /** Replaces the plain brand header (the desktop app needs its title bar and window controls). */
  header?: ReactNode
}

/**
 * Sign-in for the web app: the Welcome artboard. "Continue with GitHub" goes to GitHub's sign-in page and comes
 * back signed in (the redirect flow, when the relay has a client secret); otherwise it runs the OAuth device
 * flow through the relay. No passwords or provider tokens ever reach the browser.
 */
export function SignIn({
  auth,
  onSignedIn,
  providers = ['github'],
  desktopUrl = 'https://github.com/mindify-ai/Astra/releases/latest',
  devAuth = false,
  openUrl = (url) => window.open(url, '_blank', 'noopener,noreferrer'),
  copyText = (text) => navigator.clipboard.writeText(text),
  server,
  purpose = 'see your name and email',
  autoStart,
  openOnCode = false,
  localAction,
  header,
}: SignInProps) {
  const [step, setStep] = useState<Step>(() =>
    auth.returning?.() ? { kind: 'returning' } : { kind: 'idle' },
  )
  const [copied, setCopied] = useState(false)
  const [now, setNow] = useState(() => Date.now())
  const [devName, setDevName] = useState('')
  const abort = useRef<AbortController | null>(null)

  useEffect(() => () => abort.current?.abort(), [])

  // Countdown while a code is showing.
  useEffect(() => {
    if (step.kind !== 'code') return
    const t = setInterval(() => setNow(Date.now()), 1000)
    return () => clearInterval(t)
  }, [step.kind])

  const fail = (e: unknown, provider: Provider | null) => {
    if (e instanceof AuthError && e.code === 'cancelled') return
    setStep({ kind: 'error', provider, message: explain(e, provider) })
  }

  const finish = async (session: Session) => {
    setStep({ kind: 'done', session })
    await onSignedIn(session)
  }

  /** The device flow: a code to enter on the provider's site, then polling until it is approved. */
  const beginDevice = async (provider: Provider) => {
    abort.current?.abort()
    const ctrl = new AbortController()
    abort.current = ctrl
    setCopied(false)
    setStep({ kind: 'starting', provider })
    try {
      const code = await auth.start(provider)
      if (ctrl.signal.aborted) return
      setNow(Date.now())
      setStep({ kind: 'code', provider, code, issuedAt: Date.now() })
      if (openOnCode) {
        void copy(code.userCode)
        openUrl(code.verificationUriComplete ?? code.verificationUri)
      }
      const session = await waitForApproval(auth, provider, code, { signal: ctrl.signal })
      await finish(session)
    } catch (e) {
      fail(e, provider)
    }
  }

  /** One click: the provider's own sign-in page, then back here. Falls back to the device flow when unavailable. */
  const begin = async (provider: Provider) => {
    if (!auth.redirect) return beginDevice(provider)
    abort.current?.abort()
    const ctrl = new AbortController()
    abort.current = ctrl
    setStep({ kind: 'starting', provider })
    try {
      const session = await auth.redirect(provider, {
        signal: ctrl.signal,
        onOpened: (url) => setStep({ kind: 'browser', provider, url }),
      })
      // Null: this page is leaving for the provider and the sign-in finishes when it comes back.
      if (session) await finish(session)
    } catch (e) {
      if (e instanceof AuthError && e.code === 'redirectUnavailable') return beginDevice(provider)
      fail(e, provider)
    }
  }

  // Back from the provider's page (web): trade the result in.
  // biome-ignore lint/correctness/useExhaustiveDependencies: once, on arrival
  useEffect(() => {
    if (step.kind !== 'returning' || !auth.complete) return
    auth.complete().then(
      (session) => (session ? finish(session) : setStep({ kind: 'idle' })),
      (e: unknown) => fail(e, 'github'),
    )
  }, [])

  const cancel = () => {
    abort.current?.abort()
    setStep({ kind: 'idle' })
  }

  const copy = async (text: string) => {
    try {
      await copyText(text)
      setCopied(true)
    } catch {
      setCopied(false)
    }
  }

  const signInDev = async () => {
    if (!auth.devToken || !devName.trim()) return
    try {
      await finish(await auth.devToken(devName.trim()))
    } catch (e) {
      fail(e, null)
    }
  }

  // biome-ignore lint/correctness/useExhaustiveDependencies: start once, on arrival
  useEffect(() => {
    if (autoStart && !auth.returning?.()) void begin(autoStart)
  }, [])

  const primary = providers[0]
  const noServer = server !== undefined && (server.valid === false || server.value.trim() === '')

  return (
    <div className="flex min-h-dvh flex-col bg-surface text-ink">
      {header ?? (
        <header className="flex min-h-14 items-center gap-x-6 box-border px-6">
          <span className="flex items-center gap-3">
            <LogoMark />
            <span className="text-label tracking-[0.22em]">ASTRA</span>
          </span>
          <div className="grow" />
          <Spec>By Mindify AI</Spec>
        </header>
      )}

      <main className="relative flex grow items-end overflow-hidden box-border p-6 sm:p-12">
        <div aria-hidden="true" className="pointer-events-none absolute top-6 -right-40 h-[540px] w-[760px]">
          <div className="absolute -top-[72px] -left-24 box-border h-[684px] w-[952px] rounded-[50%] border border-line-subtle" />
          <div className="absolute top-0 left-0 box-border h-[540px] w-[760px] rounded-[50%] border border-line" />
          <div className="absolute top-[266px] -left-1 size-2 rounded-full bg-signal" />
        </div>

        <div className="relative flex max-w-[960px] flex-col">
          <Eyebrow>Multiagent development environment</Eyebrow>
          <h1 className="m-0 mt-6 text-display-md uppercase sm:text-display-lg">
            Every agent. One workspace.
          </h1>
          <p className="m-0 mt-6 max-w-[640px] text-lead text-ink-muted">
            Run agents from any vendor or harness on the same repository, together with your team.
          </p>

          {step.kind === 'idle' || step.kind === 'starting' || step.kind === 'error' ? (
            <>
              {server ? (
                <Field
                  label={server.optional ? 'Sync server (optional)' : 'Sync server'}
                  className="mt-8 max-w-[420px]"
                >
                  {(id) => (
                    <TextInput
                      id={id}
                      mono
                      inputMode="url"
                      placeholder={
                        server.optional ? 'Leave empty to use this machine only' : 'https://sync.example.com'
                      }
                      value={server.value}
                      onChange={(e) => server.onChange(e.target.value)}
                    />
                  )}
                </Field>
              ) : null}
              <div className="mt-8 flex flex-wrap gap-3">
                {providers.map((p) => {
                  const busy = step.kind === 'starting' && step.provider === p
                  return (
                    <Button
                      key={p}
                      size="lg"
                      variant={p === primary ? 'primary' : 'outline'}
                      disabled={step.kind === 'starting' || noServer}
                      onClick={() => void begin(p)}
                    >
                      {busy
                        ? auth.redirect
                          ? `Opening ${providerName[p]}…`
                          : 'Connecting…'
                        : `Continue with ${providerName[p]}${p === primary ? ' →' : ''}`}
                    </Button>
                  )
                })}
                {localAction ?? (
                  <a
                    href={desktopUrl}
                    className={buttonClass('outline', 'lg')}
                    title="Local folders open in the desktop app"
                  >
                    Get the desktop app
                  </a>
                )}
              </div>
              {step.kind === 'error' ? (
                <p role="alert" className="m-0 mt-6">
                  <Status tone="danger">{step.message}</Status>
                </p>
              ) : null}
            </>
          ) : null}

          {step.kind === 'code' ? (
            <DeviceStep
              provider={step.provider}
              purpose={purpose}
              code={step.code}
              secondsLeft={Math.max(0, Math.round(step.code.expiresIn - (now - step.issuedAt) / 1000))}
              copied={copied}
              onCopy={() => void copy(step.code.userCode)}
              onOpen={() => {
                void copy(step.code.userCode)
                openUrl(step.code.verificationUriComplete ?? step.code.verificationUri)
              }}
              onCancel={cancel}
            />
          ) : null}

          {step.kind === 'browser' ? (
            <section
              aria-labelledby="browser-step"
              className="mt-8 flex max-w-[640px] flex-col gap-6 border border-line bg-surface-raised p-6"
            >
              <div className="flex flex-col gap-2">
                <Eyebrow as="h2" id="browser-step">
                  Sign in with {providerName[step.provider]}
                </Eyebrow>
                <p className="m-0 text-ink-muted">
                  Finish in your browser. Astra comes back here by itself once {providerName[step.provider]}{' '}
                  lets it {purpose}. Astra never sees your {providerName[step.provider]} password.
                </p>
              </div>
              <div className="flex flex-wrap items-center gap-x-4 gap-y-3">
                <Button variant="outline" onClick={() => openUrl(step.url)}>
                  Open {providerName[step.provider]} again
                </Button>
                <Button variant="text" size="md" onClick={() => void beginDevice(step.provider)}>
                  Use a code instead
                </Button>
                <Button variant="text" size="md" onClick={cancel}>
                  Cancel
                </Button>
              </div>
              <div className="border-t border-line pt-4">
                <Status tone="working">Waiting for {providerName[step.provider]}</Status>
              </div>
            </section>
          ) : null}

          {step.kind === 'returning' ? (
            <p role="status" className="m-0 mt-8">
              <Status tone="working">Signing you in…</Status>
            </p>
          ) : null}

          {step.kind === 'done' ? (
            <p role="status" className="m-0 mt-8">
              <Status tone="success">Signed in as {step.session.person.name}. Opening your workspace…</Status>
            </p>
          ) : null}

          {devAuth &&
          auth.devToken &&
          step.kind !== 'code' &&
          step.kind !== 'done' &&
          step.kind !== 'browser' &&
          step.kind !== 'returning' ? (
            <form
              className="mt-8 flex flex-wrap items-center gap-3"
              onSubmit={(e) => {
                e.preventDefault()
                void signInDev()
              }}
            >
              <TextInput
                aria-label="Developer name"
                placeholder="Your name (developer sign-in)"
                value={devName}
                onChange={(e) => setDevName(e.target.value)}
              />
              <Button type="submit" variant="outline">
                Dev sign-in
              </Button>
            </form>
          ) : null}

          <Spec as="p" className="m-0 mt-12">
            Your code stays in your repository. Agents run where you put them.
          </Spec>
        </div>
      </main>
    </div>
  )
}

function DeviceStep({
  provider,
  purpose,
  code,
  secondsLeft,
  copied,
  onCopy,
  onOpen,
  onCancel,
}: {
  provider: Provider
  purpose: string
  code: DeviceCode
  secondsLeft: number
  copied: boolean
  onCopy: () => void
  onOpen: () => void
  onCancel: () => void
}) {
  const name = providerName[provider]
  return (
    <section
      aria-labelledby="device-step"
      className="mt-8 flex max-w-[640px] flex-col gap-6 border border-line bg-surface-raised p-6"
    >
      <div className="flex flex-col gap-2">
        <Eyebrow as="h2" id="device-step">
          Sign in with {name}
        </Eyebrow>
        <p className="m-0 text-ink-muted">
          Open {name} and enter this code to let Astra {purpose}. Astra never sees your {name} password.
        </p>
        <Spec as="p" className="m-0 break-all normal-case tracking-normal">
          {code.verificationUri}
        </Spec>
      </div>
      <div className="flex flex-wrap items-center gap-x-6 gap-y-3">
        <output aria-label="Your code" className="font-mono text-metric-sm tracking-[0.12em] tabular-nums">
          {code.userCode}
        </output>
        <Button variant="outline" size="sm" onClick={onCopy}>
          {copied ? 'Copied' : 'Copy code'}
        </Button>
      </div>
      <div className="flex flex-wrap items-center gap-x-4 gap-y-3">
        <Button onClick={onOpen}>Open {name} →</Button>
        <Button variant="text" size="md" onClick={onCancel}>
          Cancel
        </Button>
      </div>
      <div className="flex flex-wrap items-center justify-between gap-x-4 gap-y-2 border-t border-line pt-4">
        <Status tone="working">Waiting for you to approve Astra on {name}</Status>
        <Spec>Code expires in {mmss(secondsLeft)}</Spec>
      </div>
    </section>
  )
}
