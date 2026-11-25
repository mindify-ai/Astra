import { buttonClass, Eyebrow, Spec } from '@astra/ui'
import { Link } from '@tanstack/react-router'
import { useHost } from '../context'
import { AppHeader, Page } from '../shell/AppHeader'

/**
 * 1 · Welcome. In the desktop app, before it has signed in to a sync server, GitHub and GitLab start sign-in;
 * otherwise (signed in, or the web app, which signs in before it opens) they go on to choosing a repository.
 */
export function Welcome() {
  const { account } = useHost()
  const signIn = account !== undefined && account.person === null
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
          <div className="mt-8 flex flex-wrap gap-3">
            {signIn ? (
              <>
                <Link to="/signin" search={{ provider: 'github' }} className={buttonClass('primary', 'lg')}>
                  Continue with GitHub →
                </Link>
                <Link to="/signin" search={{ provider: 'gitlab' }} className={buttonClass('outline', 'lg')}>
                  Continue with GitLab
                </Link>
              </>
            ) : (
              <>
                <Link to="/setup/repo" search={{ source: 'github' }} className={buttonClass('primary', 'lg')}>
                  Continue with GitHub →
                </Link>
                <Link to="/setup/repo" search={{ source: 'gitlab' }} className={buttonClass('outline', 'lg')}>
                  Continue with GitLab
                </Link>
              </>
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
