import {
  type Auth,
  AuthError,
  type DeviceCode,
  type Provider,
  type Session,
  waitForApproval,
} from '@astra/client'
import { Button, buttonClass, Eyebrow, LogoMark, Spec, Status, TextInput } from '@astra/ui'
import { useEffect, useRef, useState } from 'react'

const providerName: Record<Provider, string> = { github: 'GitHub', gitlab: 'GitLab' }

type Step =
  | { kind: 'idle' }
  | { kind: 'starting'; provider: Provider }
  | { kind: 'code'; provider: Provider; code: DeviceCode; issuedAt: number }
  | { kind: 'done'; session: Session }
  | { kind: 'error'; provider: Provider | null; message: string }

function explain(e: unknown, provider: Provider | null): string {
  const name = provider ? providerName[provider] : 'that provider'
  if (!(e instanceof AuthError)) return 'Astra could not sign you in. Try again in a moment.'
  switch (e.code) {
    case 'notConfigured':
      return `Sign-in with ${name} is not set up on this server yet.`
    case 'network':
      return 'Astra could not reach its server. Check your connection and try again.'
    case 'expired':
      return 'The code expired before it was approved. Start again for a new one.'
    case 'denied':
      return `Sign-in was cancelled on ${name}.`
    default:
      return 'Astra could not sign you in. Try again in a moment.'
  }
}

const mmss = (secs: number) =>
  `${String(Math.floor(secs / 60)).padStart(2, '0')}:${String(secs % 60).padStart(2, '0')}`

export interface SignInProps {
  auth: Auth
  onSignedIn: (session: Session) => void
  /** Providers offered, in order. The first is the primary button. */
  providers?: Provider[]
  /** Where "Get the desktop app" points; local folders need the desktop app. */
  desktopUrl?: string
  /** Show the developer-token form (local relay with ASTRA_DEV_AUTH=1). */
  devAuth?: boolean
  /** Opens the provider's page; injectable for tests. */
  openUrl?: (url: string) => void
}

/**
 * Sign-in for the web app: the Welcome artboard, where "Continue with GitHub" runs the OAuth device flow
 * through the relay. No passwords or provider tokens ever reach the browser.
 */
export function SignIn({
  auth,
  onSignedIn,
  providers = ['github', 'gitlab'],
  desktopUrl = 'https://github.com/mindify-ai/Astra/releases/latest',
  devAuth = false,
  openUrl = (url) => window.open(url, '_blank', 'noopener,noreferrer'),
}: SignInProps) {
  const [step, setStep] = useState<Step>({ kind: 'idle' })
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

  const begin = async (provider: Provider) => {
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
      const session = await waitForApproval(auth, provider, code, { signal: ctrl.signal })
      setStep({ kind: 'done', session })
      onSignedIn(session)
    } catch (e) {
      if (e instanceof AuthError && e.code === 'cancelled') return
      setStep({ kind: 'error', provider, message: explain(e, provider) })
    }
  }

  const cancel = () => {
    abort.current?.abort()
    setStep({ kind: 'idle' })
  }

  const copy = async (text: string) => {
    try {
      await navigator.clipboard.writeText(text)
      setCopied(true)
    } catch {
      setCopied(false)
    }
  }

  const signInDev = async () => {
    if (!auth.devToken || !devName.trim()) return
    try {
      const session = await auth.devToken(devName.trim())
      setStep({ kind: 'done', session })
      onSignedIn(session)
    } catch (e) {
      setStep({ kind: 'error', provider: null, message: explain(e, null) })
    }
  }

  const primary = providers[0]

  return (
    <div className="flex min-h-dvh flex-col bg-surface text-ink">
      <header className="flex min-h-14 items-center gap-x-6 box-border px-6">
        <span className="flex items-center gap-3">
          <LogoMark />
          <span className="text-label tracking-[0.22em]">ASTRA</span>
        </span>
        <div className="grow" />
        <Spec>By Mindify AI</Spec>
      </header>

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
              <div className="mt-8 flex flex-wrap gap-3">
                {providers.map((p) => {
                  const busy = step.kind === 'starting' && step.provider === p
                  return (
                    <Button
                      key={p}
                      size="lg"
                      variant={p === primary ? 'primary' : 'outline'}
                      disabled={step.kind === 'starting'}
                      onClick={() => void begin(p)}
                    >
                      {busy ? 'Connecting…' : `Continue with ${providerName[p]}${p === primary ? ' →' : ''}`}
                    </Button>
                  )
                })}
                <a
                  href={desktopUrl}
                  className={buttonClass('outline', 'lg')}
                  title="Local folders open in the desktop app"
                >
                  Get the desktop app
                </a>
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

          {step.kind === 'done' ? (
            <p role="status" className="m-0 mt-8">
              <Status tone="success">Signed in as {step.session.person.name}. Opening your workspace…</Status>
            </p>
          ) : null}

          {devAuth && auth.devToken && step.kind !== 'code' && step.kind !== 'done' ? (
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
  code,
  secondsLeft,
  copied,
  onCopy,
  onOpen,
  onCancel,
}: {
  provider: Provider
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
          Open {name} and enter this code to let Astra see your name and email. Astra never sees your {name}{' '}
          password.
        </p>
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
