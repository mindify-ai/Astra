import { type Auth, AuthError, type Provider, serverBaseUrl } from '@astra/client'
import { buttonClass } from '@astra/ui'
import { Link, Navigate } from '@tanstack/react-router'
import { useMemo, useState } from 'react'
import { useHost } from '../context'
import { AppHeader } from '../shell/AppHeader'
import { SignIn } from './SignIn'

/** Stands in until a valid server address is typed (the buttons are disabled meanwhile). */
const noServer: Auth = {
  start: () => Promise.reject(new AuthError('network')),
  poll: () => Promise.reject(new AuthError('network')),
}

/**
 * Welcome's "Continue with GitHub" in the desktop app: the OAuth device flow through a sync server, which
 * then reopens the app on that workspace so teammates, the web and phones join it. "Use this machine only"
 * keeps the local workspace. Without the capability (web, tests) it goes on to choosing a repository.
 */
export function AccountSignIn({ provider }: { provider?: Provider }) {
  const { account } = useHost()
  const [server, setServer] = useState(account?.server ?? '')
  const base = serverBaseUrl(server)
  const auth = useMemo(() => (account && base ? account.auth(base) : null), [account, base])
  if (!account) return <Navigate to="/setup/repo" search={provider ? { source: provider } : {}} />
  return (
    <SignIn
      auth={auth ?? noServer}
      server={{ value: server, onChange: setServer, valid: base !== null }}
      autoStart={base ? provider : undefined}
      onSignedIn={(session) => (base ? account.signIn(base, session) : Promise.resolve())}
      header={<AppHeader border={false} byline="BY MINDIFY AI" />}
      localAction={
        <Link to="/setup/repo" search={{ source: 'local' }} className={buttonClass('outline', 'lg')}>
          Use this machine only
        </Link>
      }
    />
  )
}
