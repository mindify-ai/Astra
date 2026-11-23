import { buttonClass, Eyebrow } from '@astra/ui'
import { Link } from '@tanstack/react-router'
import { useEffect } from 'react'
import { useClient } from '../context'
import { AppHeader, Page } from '../shell/AppHeader'

/** Tells teammates what you're looking at ("Lin is reading result B"). Cleared on leave. */
export function useMissionPresence(missionId: string, activity: string) {
  const client = useClient()
  useEffect(() => {
    client.setPresence({ missionId, activity })
    return () => client.setPresence(null)
  }, [client, missionId, activity])
}

export function MissionNotFound() {
  return (
    <Page header={<AppHeader repoSelect nav={null} />}>
      <main className="mx-auto flex w-full max-w-[720px] flex-col gap-6 px-6 py-24">
        <Eyebrow>Mission</Eyebrow>
        <h1 className="m-0 text-title">This mission is not here.</h1>
        <p className="m-0 text-ink-muted">
          It may still be syncing, or it belongs to a workspace you are not in.
        </p>
        <Link to="/" className={buttonClass('primary', 'md', 'self-start')}>
          Back home
        </Link>
      </main>
    </Page>
  )
}
