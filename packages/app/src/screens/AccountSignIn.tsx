import { type Auth, AuthError, type Provider, serverBaseUrl } from '@astra/client'
import { buttonClass } from '@astra/ui'
import { Link, Navigate } from '@tanstack/react-router'
import { useMemo, useState } from 'react'
import { useHost } from '../context'
import { AppHeader } from '../shell/AppHeader'
import { SignIn } from './SignIn'

/** Stands in until there is a way to sign in (the buttons are disabled meanwhile). */
const noServer: Auth = {
  start: () => Promise.reject(new AuthError('network')),
  poll: () => Promise.reject(new AuthError('network')),
}

/**
 * Welcome's "Continue with GitHub" in the desktop app. With no sync server it signs in with GitHub on this
 * machine (the app's own OAuth client), so you can clone your repositories. With a server address it signs in
 * through that relay, which reopens the app on the team's workspace so teammates, the web and phones join it. Without the capability (web, tests) it goes on to choosing a repository.
 */
export function AccountSignIn({ provider }: { provider?: Provider }) {
  const { account, github, openUrl, copy, focusWindow } = useHost()
  const [server, setServer] = useState(account?.server ?? '')
  const typed = server.trim() !== ''
  const base = serverBaseUrl(server)
  // A server address: sign in to it (team sync). None: sign in with GitHub on this machine, when the app can.
  const auth = useMemo(
    () => (typed ? (account && base ? account.auth(base) : null) : (github?.auth ?? null)),
    [account, github, typed, base],
  )
  if (!account) return <Navigate to="/setup/repo" search={provider ? { source: provider } : {}} />
  const local = !typed && github !== undefined
  return (
    <SignIn
      auth={auth ?? noServer}
      server={{ value: server, onChange: setServer, valid: auth !== null, optional: github !== undefined }}
      purpose={local ? 'see your profile and list and clone your repositories' : undefined}
      autoStart={auth ? provider : undefined}
      openUrl={openUrl}
      copyText={copy}
      // GitHub on this machine uses a code (no client secret can ship in an app): open GitHub with the code
      // already copied, and come back to the front once it is approved.
      openOnCode={local && openUrl !== undefined}
      onSignedIn={(session) => {
        focusWindow?.()
        return account.signIn(local ? '' : (base ?? ''), session)
      }}
      header={<AppHeader border={false} byline="BY MINDIFY AI" />}
      localAction={
        <Link to="/setup/repo" search={{ source: 'local' }} className={buttonClass('outline', 'lg')}>
          Open a local folder
        </Link>
      }
    />
  )
}
