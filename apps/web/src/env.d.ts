/// <reference types="vite/client" />
interface ImportMetaEnv {
  /** Sync relay (astra-server) base URL. Unset: the web app runs on the in-browser demo workspace. */
  readonly VITE_ASTRA_RELAY_URL?: string
  /** "1" shows the developer sign-in form (relay running with ASTRA_DEV_AUTH=1). */
  readonly VITE_ASTRA_DEV_AUTH?: string
}
