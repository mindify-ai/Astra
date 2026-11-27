import {
  ArrowLeftIcon,
  ArrowRightIcon,
  CloseIcon,
  cx,
  ExternalIcon,
  Eyebrow,
  GlobeIcon,
  PopOutIcon,
  ReloadIcon,
  Spec,
} from '@astra/ui'
import { type FormEvent, type ReactNode, useState } from 'react'
import { useHost, useUi } from '../context'

const STORAGE = 'astra.browser.url'
const read = () => {
  try {
    return localStorage.getItem(STORAGE)
  } catch {
    return null
  }
}
const write = (url: string) => {
  try {
    localStorage.setItem(STORAGE, url)
  } catch {
    // Storage is unavailable: the page isn't remembered across reloads.
  }
}

/**
 * Turns what someone typed into a URL: `localhost:3000` and private addresses get http, bare hosts get https,
 * and anything else (or an unsafe scheme) is null.
 */
export function normalizeUrl(input: string): string | null {
  const t = input.trim()
  if (!t) return null
  const local =
    /^(localhost|127\.\d+\.\d+\.\d+|0\.0\.0\.0|\[::1\]|10\.\d+\.\d+\.\d+|192\.168\.\d+\.\d+)(:\d+)?(\/|$)/i
  let candidate = t
  if (!/^[a-z][a-z\d+.-]*:/i.test(t) || /^localhost:\d+/i.test(t)) {
    if (/^:\d+/.test(t)) candidate = `http://localhost${t}`
    else if (/^\d+$/.test(t)) candidate = `http://localhost:${t}`
    else candidate = `${local.test(t) ? 'http' : 'https'}://${t}`
  }
  try {
    const u = new URL(candidate)
    if (u.protocol !== 'http:' && u.protocol !== 'https:') return null
    if (!u.hostname.includes('.') && u.hostname !== 'localhost' && !u.hostname.startsWith('[')) return null
    return u.toString()
  } catch {
    return null
  }
}

/** What a page may do in the default sandbox: scripts, forms, new windows, dialogs and downloads. */
const SANDBOX = 'allow-scripts allow-forms allow-popups allow-modals allow-downloads'
/** The isolated sandbox: scripts and forms only, and never the page's own origin. */
const ISOLATED_SANDBOX = 'allow-scripts allow-forms'

/**
 * The iframe sandbox. A page from the app's own origin never gets `allow-same-origin`: with scripts it could
 * lift its own sandbox and read the app's storage (and its session). Other origins keep theirs, so sign-ins and
 * dev servers work, and still can't reach the app.
 *
 * `isolated` takes the origin away from every page. The page runs in an opaque origin, so it has no cookies,
 * storage or service workers to read or leave behind, and it can't open windows, show dialogs or download files.
 */
export function sandboxFor(url: string, isolated = false): string {
  if (isolated) return ISOLATED_SANDBOX
  try {
    const same = typeof window !== 'undefined' && new URL(url).origin === window.location.origin
    return same ? SANDBOX : `${SANDBOX} allow-same-origin`
  } catch {
    return SANDBOX
  }
}

const QUICK = ['localhost:5173', 'localhost:3000', 'localhost:8080']

/** The icon-button look of the browser's bars, shared with the windows that host a `BrowserView`. */
export const browserIconButton =
  'flex size-7 shrink-0 items-center justify-center border-0 bg-transparent p-0 text-ink-muted hover:text-ink disabled:opacity-40 disabled:hover:text-ink-muted'
const iconButton = browserIconButton

/**
 * The browser itself: the address bar and the page. Back and forward follow the addresses opened here; sites
 * that refuse to be framed can open in the system browser instead. `lead` and `trail` are the host's own
 * controls at the bar's ends (a heading, a close button).
 */
export function BrowserView({ lead, trail }: { lead?: ReactNode; trail?: ReactNode }) {
  const host = useHost()
  const { browserIsolated: isolated, setBrowserIsolated } = useUi()
  const [history, setHistory] = useState<string[]>(() => {
    const saved = read()
    return saved ? [saved] : []
  })
  const [at, setAt] = useState(history.length - 1)
  const url = history[at] ?? null
  const [draft, setDraft] = useState(url ?? '')
  const [error, setError] = useState<string | null>(null)
  const [reloads, setReloads] = useState(0)
  // The sandbox only applies to a page as it loads, so changing it loads the page again in a new frame.
  const frame = `${at}:${reloads}:${isolated ? 'isolated' : 'open'}`
  const [loaded, setLoaded] = useState<string | null>(null)
  const loading = Boolean(url) && loaded !== frame

  const go = (target: string) => {
    const next = normalizeUrl(target)
    if (!next) {
      setError('Enter a web address, like localhost:3000 or example.com.')
      return
    }
    setError(null)
    setDraft(next)
    write(next)
    setHistory((h) => [...h.slice(0, at + 1), next])
    setAt((i) => i + 1)
  }
  const move = (d: number) => {
    const i = at + d
    const next = history[i]
    if (!next) return
    setAt(i)
    setDraft(next)
    write(next)
  }
  const submit = (e: FormEvent) => {
    e.preventDefault()
    go(draft)
  }
  const openOutside = () => {
    if (!url) return
    if (host.openUrl) host.openUrl(url)
    else window.open(url, '_blank', 'noopener,noreferrer')
  }

  return (
    <>
      {/* A container, so the bar sheds its heading and then the toggle's word as its pane or window narrows. */}
      <div className="@container flex h-9 shrink-0 items-center gap-1 border-b border-line bg-surface-alt px-2">
        {lead}
        <button
          type="button"
          aria-label="Back"
          className={iconButton}
          disabled={at <= 0}
          onClick={() => move(-1)}
        >
          <ArrowLeftIcon size={14} />
        </button>
        <button
          type="button"
          aria-label="Forward"
          className={iconButton}
          disabled={at >= history.length - 1}
          onClick={() => move(1)}
        >
          <ArrowRightIcon size={14} />
        </button>
        <button
          type="button"
          aria-label="Reload"
          className={iconButton}
          disabled={!url}
          onClick={() => setReloads((n) => n + 1)}
        >
          <ReloadIcon size={14} />
        </button>
        <form onSubmit={submit} className="flex min-w-0 grow">
          <input
            type="text"
            aria-label="Address"
            inputMode="url"
            spellCheck={false}
            autoComplete="off"
            value={draft}
            onChange={(e) => setDraft(e.target.value)}
            onFocus={(e) => e.currentTarget.select()}
            placeholder="localhost:3000 or a web address"
            className="box-border h-7 w-full min-w-0 rounded-sm border border-line-strong bg-surface-raised px-2 font-mono text-data text-ink"
          />
        </form>
        <button
          type="button"
          aria-label="Isolated sandbox"
          aria-pressed={isolated}
          title={
            isolated
              ? 'Isolated sandbox is on: pages get no cookies, storage, pop-ups or downloads. Some sign-ins and dev servers need it off.'
              : 'Turn on the isolated sandbox: pages get no cookies, storage, pop-ups or downloads.'
          }
          onClick={() => setBrowserIsolated(!isolated)}
          className={cx(
            'flex h-7 shrink-0 items-center gap-2 border px-2 font-mono text-spec uppercase',
            isolated
              ? 'border-line-strong bg-surface-raised text-ink'
              : 'border-line bg-transparent text-ink-muted hover:text-ink',
          )}
        >
          <span
            aria-hidden="true"
            className={cx(
              'inline-block size-2 shrink-0 rounded-full',
              isolated ? 'bg-signal' : 'border border-line-strong',
            )}
          />
          <span className="@max-[400px]:hidden">Isolated</span>
        </button>
        <button
          type="button"
          aria-label="Open in your browser"
          title="Open in your browser"
          className={iconButton}
          disabled={!url}
          onClick={openOutside}
        >
          <ExternalIcon size={14} />
        </button>
        {trail}
      </div>
      {loading ? <div aria-hidden="true" className="h-px shrink-0 bg-signal" /> : null}
      {error ? (
        <p role="alert" className="m-0 border-b border-line px-3 py-2 text-body-sm text-danger">
          {error}
        </p>
      ) : null}
      <div className="relative min-h-0 grow bg-surface-sunken">
        {url ? (
          <iframe
            key={frame}
            title={`Browser: ${url}`}
            src={url}
            onLoad={() => setLoaded(frame)}
            sandbox={sandboxFor(url, isolated)}
            referrerPolicy="no-referrer"
            // Where the engine supports it (Chromium), an isolated page also loads without the cookies its site
            // already has, into storage that is thrown away with the frame.
            {...{ credentialless: isolated }}
            // The web's default page canvas (a system colour), so pages without a background stay readable.
            style={{ colorScheme: 'light', background: 'Canvas' }}
            className="absolute inset-0 size-full border-0"
          />
        ) : (
          <div className="flex h-full flex-col items-start justify-center gap-4 overflow-auto p-6">
            <GlobeIcon size={24} className="shrink-0 text-ink-muted" />
            <p className="m-0 max-w-[420px] text-body-sm text-ink-muted">
              Preview your dev server or read docs beside the code. Some sites refuse to open inside apps;
              open those in your browser.
            </p>
            <div className="flex flex-wrap gap-2">
              {QUICK.map((q) => (
                <button
                  key={q}
                  type="button"
                  onClick={() => go(q)}
                  className="h-7 rounded-sm border border-line bg-surface-raised px-2 font-mono text-data text-ink hover:border-line-strong"
                >
                  {q}
                </button>
              ))}
            </div>
            <Spec>
              {isolated
                ? 'ISOLATED SANDBOX · NO COOKIES, STORAGE, POP-UPS OR DOWNLOADS'
                : 'PAGES RUN SANDBOXED · NO ACCESS TO YOUR ASTRA SESSION'}
            </Spec>
          </div>
        )}
      </div>
    </>
  )
}

/**
 * A browser inside the app, for previewing a dev server or reading docs next to the code and the thread: the
 * workbench's pane. `onPopOut` moves it into the floating mini browser.
 */
export function BrowserPane({
  onClose,
  onPopOut,
  className,
}: {
  onClose?: () => void
  onPopOut?: () => void
  className?: string
}) {
  return (
    <section aria-label="Browser" className={cx('flex min-h-0 min-w-0 flex-col bg-surface', className)}>
      <BrowserView
        lead={
          <Eyebrow as="h2" className="mr-2 ml-1 @max-[560px]:hidden">
            Browser
          </Eyebrow>
        }
        trail={
          <>
            {onPopOut ? (
              <button
                type="button"
                aria-label="Pop out to the mini browser"
                title="Pop out to the mini browser"
                className={iconButton}
                onClick={onPopOut}
              >
                <PopOutIcon size={14} />
              </button>
            ) : null}
            {onClose ? (
              <button type="button" aria-label="Close browser" className={iconButton} onClick={onClose}>
                <CloseIcon size={14} />
              </button>
            ) : null}
          </>
        }
      />
    </section>
  )
}
