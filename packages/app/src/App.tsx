import type { AstraClient } from '@astra/client'
import type { WindowActions } from '@astra/ui'
import { RouterProvider } from '@tanstack/react-router'
import { useMemo } from 'react'
import { AstraProvider, type HostCapabilities, type Platform } from './context'
import { createAstraRouter, type HistoryKind } from './router'

export interface AstraAppProps {
  client: AstraClient
  platform: Platform
  history?: HistoryKind
  windowActions?: WindowActions
  host?: HostCapabilities
}

/** The whole Astra UI. Desktop (Tauri), phones (Tauri mobile) and the web SaaS all mount this. */
export function AstraApp({ client, platform, history = 'browser', windowActions, host }: AstraAppProps) {
  const router = useMemo(() => createAstraRouter(history), [history])
  return (
    <AstraProvider client={client} platform={platform} windowActions={windowActions} host={host}>
      <RouterProvider router={router} />
    </AstraProvider>
  )
}
