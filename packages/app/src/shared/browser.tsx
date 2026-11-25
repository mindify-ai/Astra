import {
  ArrowLeftIcon,
  ArrowRightIcon,
  CloseIcon,
  cx,
  ExternalIcon,
  Eyebrow,
  GlobeIcon,
  ReloadIcon,
  Spec,
} from '@astra/ui'
import { type FormEvent, useState } from 'react'
import { useHost } from '../context'

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

/**
 * The iframe sandbox. A page from the app's own origin never gets `allow-same-origin`: with scripts it could
 * lift its own sandbox and read the app's storage (and its session). Other origins keep theirs, so sign-ins and
 * dev servers work, and still can't reach the app.
 */
export function sandboxFor(url: string): string {
  const base = 'allow-scripts allow-forms allow-popups allow-modals allow-downloads'
  try {
    const same = typeof window !== 'undefined' && new URL(url).origin === window.location.origin
    return same ? base : `${base} allow-same-origin`
  } catch {
    return base
  }
}

const QUICK = ['localhost:5173', 'localhost:3000', 'localhost:8080']

/**
 * A browser inside the app, for previewing a dev server or reading docs next to the code and the thread.
 * Back and forward follow the addresses opened here; sites that refuse to be framed can open in the system
 * browser instead.
 */
export function BrowserPane({ onClose, className }: { onClose?: () => void; className?: string }) {
  const host = useHost()
  const [history, setHistory] = useState<string[]>(() => {
    const saved = read()
    return saved ? [saved] : []
  })
  const [at, setAt] = useState(history.length - 1)
  const url = history[at] ?? null
  const [draft, setDraft] = useState(url ?? '')
  const [error, setError] = useState<string | null>(null)
  const [reloads, setReloads] = useState(0)
  const [loading, setLoading] = useState(Boolean(url))

  const go = (target: string) => {
    const next = normalizeUrl(target)
    if (!next) {
      setError('Enter a web address, like localhost:3000 or example.com.')
      return
    }
    setError(null)
    setDraft(next)
    setLoading(true)
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
    setLoading(true)
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

  const iconButton =
    'flex size-7 shrink-0 items-center justify-center border-0 bg-transparent p-0 text-ink-muted hover:text-ink disabled:opacity-40 disabled:hover:text-ink-muted'

  return (
    <section aria-label="Browser" className={cx('flex min-h-0 min-w-0 flex-col bg-surface', className)}>
      <div className="flex h-9 shrink-0 items-center gap-1 border-b border-line bg-surface-alt px-2">
        <Eyebrow as="h2" className="mr-2 ml-1 max-[520px]:hidden">
          Browser
        </Eyebrow>
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
          onClick={() => {
            setLoading(true)
            setReloads((n) => n + 1)
          }}
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
          aria-label="Open in your browser"
          title="Open in your browser"
          className={iconButton}
          disabled={!url}
          onClick={openOutside}
        >
          <ExternalIcon size={14} />
        </button>
        {onClose ? (
          <button type="button" aria-label="Close browser" className={iconButton} onClick={onClose}>
            <CloseIcon size={14} />
          </button>
        ) : null}
      </div>
      {loading && url ? <div aria-hidden="true" className="h-px shrink-0 bg-signal" /> : null}
      {error ? (
        <p role="alert" className="m-0 border-b border-line px-3 py-2 text-body-sm text-danger">
          {error}
        </p>
      ) : null}
      <div className="relative min-h-0 grow bg-surface-sunken">
        {url ? (
          <iframe
            key={`${at}:${reloads}`}
            title={`Browser: ${url}`}
            src={url}
            onLoad={() => setLoading(false)}
            sandbox={sandboxFor(url)}
            referrerPolicy="no-referrer"
            // The web's default page canvas (a system colour), so pages without a background stay readable.
            style={{ colorScheme: 'light', background: 'Canvas' }}
            className="absolute inset-0 size-full border-0"
          />
        ) : (
          <div className="flex h-full flex-col items-start justify-center gap-4 p-6">
            <GlobeIcon size={24} className="text-ink-muted" />
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
            <Spec>PAGES RUN SANDBOXED · NO ACCESS TO YOUR ASTRA SESSION</Spec>
          </div>
        )}
      </div>
    </section>
  )
}
