import { slugify } from '@astra/client'
import { Button, buttonClass, cx, Dot, Eyebrow, inlineLinkClass, TextInput } from '@astra/ui'
import { Link, useNavigate } from '@tanstack/react-router'
import { useState } from 'react'
import { useAstraState, useClient, useHost, useUi } from '../context'
import { AppHeader, Page } from '../shell/AppHeader'

const sourceLabel = { github: 'GITHUB', gitlab: 'GITLAB', local: 'LOCAL FOLDER' } as const

/** 2 · Select a repository (setup step 1 of 2). */
export function Repo() {
  const state = useAstraState()
  const client = useClient()
  const host = useHost()
  const ui = useUi()
  const navigate = useNavigate()
  const [query, setQuery] = useState('')
  const repos = Object.values(state.repositories).filter((r) =>
    r.name.toLowerCase().includes(query.trim().toLowerCase()),
  )

  const openFolder = async () => {
    const path = await host.pickFolder?.()
    if (!path) return
    const name = path.replace(/^\/(Users|home)\/[^/]+/, '~')
    const id = `r-${slugify(name)}`
    await client.submit([
      {
        type: 'put',
        entity: {
          kind: 'repository',
          id,
          name,
          source: 'local',
          defaultBranch: 'main',
          path,
          remoteUrl: null,
        },
      },
    ])
    ui.setRepoId(id)
  }

  return (
    <Page
      header={
        <AppHeader
          step="SET UP · STEP 1 OF 2"
          actions={
            <Link to="/welcome" className={cx(buttonClass('text', 'sm'))}>
              BACK
            </Link>
          }
        />
      }
    >
      <main className="mx-auto box-border flex w-full max-w-[720px] flex-col gap-8 px-6 py-12">
        <div className="flex flex-col gap-2">
          <Eyebrow>Repository</Eyebrow>
          <h1 className="m-0 text-title">Select a repository</h1>
          <p className="m-0 text-ink-muted">
            Agents work on a branch of the repository you pick. You can switch repositories at any time from
            the top bar.
          </p>
        </div>
        <div className="flex flex-col gap-2">
          <TextInput
            type="search"
            aria-label="Search repositories"
            placeholder="Search repositories"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
          />
          {repos.map((r) => {
            const selected = r.id === ui.repoId
            return (
              <button
                key={r.id}
                type="button"
                aria-pressed={selected}
                onClick={() => ui.setRepoId(r.id)}
                className={cx(
                  'box-border flex min-h-14 w-full flex-wrap items-center gap-x-4 gap-y-1 border px-4 py-2 text-left text-ink',
                  selected ? 'border-signal bg-signal-soft' : 'border-line bg-surface-raised',
                )}
              >
                <span className="flex-[1_1_220px] font-mono text-data">{r.name}</span>
                <span className="flex-[0_0_110px] font-mono text-spec text-ink-muted">
                  {sourceLabel[r.source]}
                </span>
                <span className="flex-[0_0_60px] font-mono text-data text-ink-muted">{r.defaultBranch}</span>
                <span className="flex flex-[0_0_90px] justify-end">
                  {selected ? (
                    <span className="inline-flex items-center gap-2 text-[13px] text-signal">
                      <Dot tone="working" />
                      Selected
                    </span>
                  ) : null}
                </span>
              </button>
            )
          })}
          {repos.length === 0 ? (
            <p className="m-0 border border-line p-4 text-ink-muted">No repository matches that search.</p>
          ) : null}
        </div>
        <div className="flex flex-wrap items-center gap-3 border-t border-line pt-6">
          <button
            type="button"
            className={buttonClass('primary', 'md', 'px-6')}
            onClick={() => navigate({ to: '/setup/agents' })}
          >
            Continue →
          </button>
          <Button
            variant="outline"
            onClick={openFolder}
            disabled={!host.pickFolder}
            title={host.pickFolder ? undefined : 'Available in the desktop app'}
          >
            Open a local folder
          </Button>
          <span className={cx(inlineLinkClass, 'flex-[1_1_200px] text-[14px]')}>
            Next: connect your agents.
          </span>
        </div>
      </main>
    </Page>
  )
}
