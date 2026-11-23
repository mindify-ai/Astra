import { AstraApp, type Platform } from '@astra/app'
import { type AstraClient, demoSeed, MockClient, RemoteClient } from '@astra/client'
import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'
import './index.css'

const params = new URLSearchParams(location.search)

/** `?platform=mac|windows|linux|ios|android` previews another platform's chrome (design review, screenshots). */
function platformFromUrl(): Platform {
  const p = params.get('platform')
  return p === 'mac' || p === 'windows' || p === 'linux' || p === 'ios' || p === 'android' ? p : 'web'
}

function makeClient(): AstraClient {
  const relay = import.meta.env.VITE_ASTRA_RELAY_URL
  const token = localStorage.getItem('astra.token')
  if (relay && token) {
    const nodeId = localStorage.getItem('astra.nodeId') ?? `n-web-${crypto.randomUUID().slice(0, 8)}`
    localStorage.setItem('astra.nodeId', nodeId)
    return new RemoteClient({
      url: relay,
      token,
      workspaceId: import.meta.env.VITE_ASTRA_WORKSPACE ?? 'ws-default',
      me: localStorage.getItem('astra.me') ?? 'p-me',
      nodeId,
    })
  }
  // No relay configured: a self-contained demo workspace that behaves like a connected desktop node.
  return new MockClient({ seed: demoSeed, latency: params.has('instant') ? 0 : 600 })
}

const client = makeClient()
;(window as unknown as { astra?: AstraClient }).astra = client

if ('serviceWorker' in navigator && import.meta.env.PROD) {
  navigator.serviceWorker.register('/sw.js').catch(() => {
    /* offline cache unavailable; the app still works online */
  })
}

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <AstraApp client={client} platform={platformFromUrl()} />
  </StrictMode>,
)
