import { buttonClass, Eyebrow, Spec, Status } from '@astra/ui'
import { Link } from '@tanstack/react-router'
import { useAstraState, useHost } from '../context'
import { AppHeader, Page } from '../shell/AppHeader'

/**
 * 1 · Welcome. In the desktop app, before it has signed in to a sync server, GitHub starts sign-in (GitLab is
 * hidden for now); otherwise (signed in, or the web app, which signs in before it opens) they go on to choosing a repository.
 */
export function Welcome() {
  const { account, github, signOut } = useHost()
  const s = useAstraState()
  // Not signed in anywhere yet: GitHub starts sign-in. Signed in: it goes on to your repositories.
  const signIn = account !== undefined && account.person === null && !github?.login
  // Back from signing in (to a sync server, or with GitHub on this machine): say so, and offer the next step.
  const signedInAs = signIn
    ? null
    : (account?.person?.name ?? github?.login ?? (signOut ? s.people[s.me]?.name : undefined) ?? null)
  return (
    <Page header={<AppHeader border={false} byline="BY MINDIFY AI" />}>
      <main className="relative flex grow items-end overflow-hidden box-border p-6 sm:p-12">
        <div aria-hidden="true" className="pointer-events-none absolute top-6 -right-40 h-[540px] w-[760px]">
          <div className="absolute -top-[72px] -left-24 box-border h-[684px] w-[952px] rounded-[50%] border border-line-subtle" />
          <div className="absolute top-0 left-0 box-border h-[540px] w-[760px] rounded-[50%] border border-line" />
          <div className="absolute top-[266px] -left-1 size-2 rounded-full bg-signal" />
        </div>
        <div className="relative flex max-w-[960px] flex-col">
          <Eyebrow>Multiagent development environment</Eyebrow>
          <h1 className="m-0 mt-6 text-display-md uppercase sm:text-display-lg">
            Every agent. One workspace.
          </h1>
          <p className="m-0 mt-6 max-w-[640px] text-lead text-ink-muted">
            Run agents from any vendor or harness on the same repository, together with your team.
          </p>
          {signedInAs ? (
            <p role="status" className="m-0 mt-8">
              <Status tone="success">Signed in as {signedInAs}. Pick a repository to start.</Status>
            </p>
          ) : null}
          <div className="mt-8 flex flex-wrap gap-3">
            {signIn ? (
              <Link to="/signin" search={{ provider: 'github' }} className={buttonClass('primary', 'lg')}>
                Continue with GitHub →
              </Link>
            ) : (
              <Link to="/setup/repo" search={{ source: 'github' }} className={buttonClass('primary', 'lg')}>
                {signedInAs ? 'Choose a GitHub repository →' : 'Continue with GitHub →'}
              </Link>
            )}
            <Link to="/setup/repo" search={{ source: 'local' }} className={buttonClass('outline', 'lg')}>
              Open a local folder
            </Link>
          </div>
          <Spec as="p" className="m-0 mt-12">
            Your code stays in your repository. Agents run where you put them.
          </Spec>
        </div>
      </main>
    </Page>
  )
}
