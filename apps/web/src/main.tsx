import { AstraApp, type Platform, SignIn } from '@astra/app'
import {
  type AstraClient,
  clearSession,
  demoSeed,
  loadSession,
  MockAuth,
  MockClient,
  RelayAuth,
  RemoteClient,
  type Session,
  saveSession,
} from '@astra/client'
import { StrictMode, useMemo, useState } from 'react'
import { createRoot } from 'react-dom/client'
import './index.css'

const params = new URLSearchParams(location.search)
const relayUrl = import.meta.env.VITE_ASTRA_RELAY_URL
const devAuth = import.meta.env.VITE_ASTRA_DEV_AUTH === '1'

/** `?platform=mac|windows|linux|ios|android` previews another platform's chrome (design review, screenshots). */
function platformFromUrl(): Platform {
  const p = params.get('platform')
  return p === 'mac' || p === 'windows' || p === 'linux' || p === 'ios' || p === 'android' ? p : 'web'
}

function nodeId(): string {
  let id = localStorage.getItem('astra.nodeId')
  if (!id) {
    id = `n-web-${crypto.randomUUID().slice(0, 8)}`
    localStorage.setItem('astra.nodeId', id)
  }
  return id
}

function Workspace({ client, onSignOut }: { client: AstraClient; onSignOut?: () => void }) {
  ;(window as unknown as { astra?: AstraClient }).astra = client
  return (
    <AstraApp
      client={client}
      platform={platformFromUrl()}
      host={onSignOut ? { signOut: onSignOut } : undefined}
    />
  )
}

/** Signed in to a relay: the real workspace, synced and cached in IndexedDB. */
function Hosted({ relay }: { relay: string }) {
  const [session, setSession] = useState<Session | null>(() => loadSession(relay))
  const auth = useMemo(() => new RelayAuth(relay), [relay])
  const signOut = () => {
    clearSession()
    setSession(null)
  }
  const client = useMemo(
    () =>
      session
        ? new RemoteClient({
            url: relay,
            token: session.token,
            workspaceId: session.workspaceId,
            me: session.person.id,
            nodeId: nodeId(),
            onUnauthorized: signOut,
          })
        : null,
    // biome-ignore lint/correctness/useExhaustiveDependencies: a new client only when the session changes
    [session, relay],
  )
  if (!session || !client) {
    return (
      <SignIn
        auth={auth}
        devAuth={devAuth}
        onSignedIn={(s) => {
          saveSession(relay, s)
          setSession(s)
        }}
      />
    )
  }
  return (
    <Workspace
      client={client}
      onSignOut={() => {
        if (client instanceof RemoteClient) {
          client.close()
          void client.forget()
        }
        signOut()
      }}
    />
  )
}

/** No relay configured: the design's demo workspace, fully in the browser. `?signin` previews the sign-in flow. */
function Demo() {
  const [signedIn, setSignedIn] = useState(!params.has('signin'))
  const client = useMemo(
    () => new MockClient({ seed: demoSeed, latency: params.has('instant') ? 0 : 600 }),
    [],
  )
  const auth = useMemo(() => new MockAuth(), [])
  if (!signedIn)
    return (
      <SignIn
        auth={auth}
        devAuth={devAuth}
        onSignedIn={() => {
          setTimeout(() => setSignedIn(true), 600)
        }}
      />
    )
  return <Workspace client={client} onSignOut={params.has('signin') ? () => setSignedIn(false) : undefined} />
}

if ('serviceWorker' in navigator && import.meta.env.PROD) {
  navigator.serviceWorker.register('/sw.js').catch(() => {
    /* offline cache unavailable; the app still works online */
  })
}

createRoot(document.getElementById('root')!).render(
  <StrictMode>{relayUrl ? <Hosted relay={relayUrl} /> : <Demo />}</StrictMode>,
)
