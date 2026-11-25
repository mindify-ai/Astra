import { AstraApp, type HostAccount, type Platform } from '@astra/app'
import {
  isTauri,
  MockClient,
  RelayAuth,
  starterSeed,
  TauriClient,
  tauriFiles,
  tauriGithub,
  tauriLoopback,
  tauriTerminal,
} from '@astra/client'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { writeText } from '@tauri-apps/plugin-clipboard-manager'
import { open } from '@tauri-apps/plugin-dialog'
import { openUrl } from '@tauri-apps/plugin-opener'
import { platform as osPlatform } from '@tauri-apps/plugin-os'
import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'
import './index.css'

function detectPlatform(): Platform {
  if (!isTauri()) {
    // Plain browser: honour `?platform=` like the web app, otherwise guess from the user agent.
    const q = new URLSearchParams(location.search).get('platform')
    if (q === 'mac' || q === 'windows' || q === 'linux' || q === 'ios' || q === 'android') return q
    const ua = navigator.userAgent
    if (/Mac/.test(ua)) return 'mac'
    if (/Win/.test(ua)) return 'windows'
    return 'linux'
  }
  const p = osPlatform()
  if (p === 'macos') return 'mac'
  if (p === 'windows' || p === 'linux' || p === 'ios' || p === 'android') return p
  return 'linux'
}

const root = createRoot(document.getElementById('root')!)

// The Vite dev server (:1420) can also be opened in a plain browser, where there is no Tauri bridge
// and no local node. Fall back to an in-browser workspace (no demo data) there instead of failing on `invoke`.
if (!isTauri()) {
  console.info('Astra: not running inside the Tauri shell; using an in-browser workspace.')
  root.render(
    <StrictMode>
      <AstraApp client={new MockClient({ seed: starterSeed })} platform={detectPlatform()} history="hash" />
    </StrictMode>,
  )
} else {
  startNode()
}

function startNode() {
  const client = new TauriClient()
  const platform = detectPlatform()
  const desktop = platform !== 'ios' && platform !== 'android'
  client.ready
    .then(() => {
      const signedIn = client.info?.account ?? null
      // "Continue with GitHub" signs this app in to a sync server (the one it signed in to before, else the
      // build's default), then the shell restarts on that workspace.
      const account: HostAccount = {
        server: signedIn?.server ?? import.meta.env.VITE_ASTRA_RELAY_URL ?? '',
        person: signedIn?.person ?? null,
        // One click: GitHub in the system browser, then back to this window through a loopback port.
        auth: (server) =>
          new RelayAuth(
            server,
            undefined,
            tauriLoopback((url) => void openUrl(url)),
          ),
        signIn: async (server, session) => {
          if (server) return TauriClient.signIn(server, session)
          // Signed in with GitHub on this machine: reopen on your repositories, with the account loaded.
          location.hash = '#/setup/repo?source=github'
          location.reload()
        },
      }
      // GitHub on this machine (sign in without a server, list and clone your repositories): desktops only.
      const github = desktop ? tauriGithub(client.info?.github?.login ?? null) : undefined
      root.render(
        <StrictMode>
          <AstraApp
            client={client}
            platform={platform}
            history="hash"
            windowActions={{
              onClose: () => void TauriClient.windowAction('close'),
              onMinimize: () => void TauriClient.windowAction('minimize'),
              onMaximize: () => void TauriClient.windowAction('toggleMaximize'),
            }}
            host={{
              pickFolder: async () => {
                const picked = await open({ directory: true, multiple: false, title: 'Open a local folder' })
                return typeof picked === 'string' ? picked : null
              },
              copy: (text) => writeText(text),
              openUrl: (url) => void openUrl(url),
              focusWindow: () => {
                const w = getCurrentWindow()
                void w
                  .unminimize()
                  .then(() => w.show())
                  .then(() => w.setFocus())
                  .catch(() => {})
              },
              account,
              github,
              signOut: signedIn ? () => void TauriClient.signOut() : undefined,
              terminal: desktop ? tauriTerminal : undefined,
              files: desktop ? tauriFiles : undefined,
            }}
          />
        </StrictMode>,
      )
    })
    .catch((err: unknown) => {
      // The Rust core failed to start (corrupt data dir, permissions). Show why instead of a blank window.
      const el = document.getElementById('root')!
      el.textContent = `Astra could not start its local node: ${String(err)}`
      el.style.cssText =
        'padding:48px;font:14px/22px Barlow,system-ui;color:#F97066;background:#000;min-height:100vh'
    })
}
