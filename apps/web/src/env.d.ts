/// <reference types="vite/client" />
interface ImportMetaEnv {
  /** Sync relay (astra-server) base URL. Unset: the web app runs on the in-browser demo workspace. */
  readonly VITE_ASTRA_RELAY_URL?: string
  readonly VITE_ASTRA_WORKSPACE?: string
}
