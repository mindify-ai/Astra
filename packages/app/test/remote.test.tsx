import { demoSeed, MockAuth, MockClient, MockGithub, type Session, starterSeed } from '@astra/client'
import { render, screen, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { describe, expect, it, vi } from 'vitest'
import { AstraApp, type HostAccount } from '../src'

describe('attaching agents to runner nodes', () => {
  it('lists this machine and the server node, and connects an agent on the server', async () => {
    const client = new MockClient({ seed: demoSeed, latency: 0, nodeId: 'n-mark-desktop' })
    render(<AstraApp client={client} platform="mac" history={{ memory: '/setup/agents' }} />)
    const user = userEvent.setup()

    await user.click(await screen.findByRole('button', { name: 'OpenCode' }))
    const runsOn = screen.getByRole('group', { name: 'Runs on' })
    expect(within(runsOn).getByRole('radio', { name: /This machine/ })).toHaveProperty('checked', true)
    await user.click(within(runsOn).getByRole('radio', { name: /build-01/ }))
    await user.click(screen.getByRole('button', { name: 'Connect agent' }))

    expect(await screen.findByText('Connected on build-01')).toBeTruthy()
    expect(client.getState().agents['a-opencode']).toMatchObject({
      nodeId: 'n-build-01',
      runsOn: 'server',
      status: 'connected',
    })
    expect(screen.getByText(/· on build-01/)).toBeTruthy()
  })

  it('a workspace on a sync server starts from the catalog, and says when no runner node has joined', async () => {
    const empty = { ...starterSeed, entities: [] }
    const client = new MockClient({ seed: empty, latency: 0 })
    render(<AstraApp client={client} platform="mac" history={{ memory: '/setup/agents' }} />)

    expect(await screen.findByRole('button', { name: 'Claude Code' })).toBeTruthy()
    expect(screen.getByText(/No runner node has joined this workspace yet/)).toBeTruthy()
    expect(screen.getByRole('button', { name: 'Connect agent' })).toHaveProperty('disabled', true)
  })
})

describe('desktop sign-in', () => {
  const account = (signIn: HostAccount['signIn']): HostAccount => ({
    server: 'sync.example.com',
    person: null,
    auth: () => new MockAuth(undefined, 1),
    signIn,
  })

  it('Continue with GitHub runs the device flow through the sync server, then hands the session over', async () => {
    const signIn = vi.fn<(server: string, s: Session) => Promise<void>>(async () => {})
    const client = new MockClient({ seed: starterSeed, latency: 0 })
    render(
      <AstraApp
        client={client}
        platform="mac"
        history={{ memory: '/' }}
        host={{ account: account(signIn) }}
      />,
    )
    const user = userEvent.setup()
    await user.click(await screen.findByRole('link', { name: 'Continue with GitHub →' }))

    // It starts at once with the server it knows (the address field shows again only if you go back).
    expect(await screen.findByLabelText('Your code')).toHaveProperty('textContent', 'WDJB-MJHT')
    await vi.waitFor(() => expect(signIn).toHaveBeenCalledOnce(), { timeout: 3000 })
    expect(signIn.mock.calls[0]?.[0]).toBe('https://sync.example.com')
    expect(signIn.mock.calls[0]?.[1].workspaceId).toBe('ws-mindify')
  })

  it('can stay local, and needs a valid server address to sign in', async () => {
    const client = new MockClient({ seed: starterSeed, latency: 0 })
    render(
      <AstraApp
        client={client}
        platform="mac"
        history={{ memory: '/signin' }}
        host={{ account: { ...account(async () => {}), server: '' } }}
      />,
    )
    const user = userEvent.setup()
    expect(await screen.findByRole('button', { name: 'Continue with GitHub →' })).toHaveProperty(
      'disabled',
      true,
    )
    await user.type(screen.getByLabelText('Sync server'), 'https://')
    expect(screen.getByRole('button', { name: 'Continue with GitHub →' })).toHaveProperty('disabled', true)
    await user.type(screen.getByLabelText('Sync server'), 'sync.example.com')
    expect(screen.getByRole('button', { name: 'Continue with GitHub →' })).toHaveProperty('disabled', false)

    await user.click(screen.getByRole('link', { name: 'Open a local folder' }))
    expect((await screen.findAllByText(/local folder/i)).length).toBeGreaterThan(0)
    expect(screen.queryByLabelText('Sync server')).toBeNull()
  })

  it('without the capability (web), Continue with GitHub goes straight to repositories', async () => {
    const client = new MockClient({ seed: starterSeed, latency: 0 })
    render(<AstraApp client={client} platform="mac" history={{ memory: '/' }} />)
    const link = await screen.findByRole('link', { name: 'Continue with GitHub →' })
    expect(link.getAttribute('href')).toContain('/setup/repo')
  })
})

describe('GitHub on this machine (desktop app)', () => {
  const account = (signIn: HostAccount['signIn']): HostAccount => ({
    server: '',
    person: null,
    auth: () => new MockAuth(undefined, 1),
    signIn,
  })

  it('signs in with GitHub without a sync server, asking for repository access', async () => {
    const signIn = vi.fn<(server: string, s: Session) => Promise<void>>(async () => {})
    const github = new MockGithub(null, new MockAuth(undefined, 1))
    const client = new MockClient({ seed: starterSeed, latency: 0 })
    render(
      <AstraApp
        client={client}
        platform="mac"
        history={{ memory: '/' }}
        host={{ account: account(signIn), github }}
      />,
    )
    const user = userEvent.setup()
    await user.click(await screen.findByRole('link', { name: 'Continue with GitHub →' }))
    expect(await screen.findByLabelText('Your code')).toBeTruthy()
    expect(screen.getByText(/list and clone your repositories/)).toBeTruthy()
    await vi.waitFor(() => expect(signIn).toHaveBeenCalledOnce(), { timeout: 3000 })
    expect(signIn.mock.calls[0]?.[0]).toBe('')
  })

  it('lists your repositories and clones one into the workspace in one click', async () => {
    const github = new MockGithub('octocat')
    const client = new MockClient({ seed: starterSeed, latency: 0 })
    render(
      <AstraApp
        client={client}
        platform="mac"
        history={{ memory: '/setup/repo?source=github' }}
        host={{ github, pickFolder: async () => '/Users/you/Code' }}
      />,
    )
    const user = userEvent.setup()
    const list = await screen.findByRole('list', { name: 'GitHub repositories' })
    expect(within(list).getByText('octo-org/payments')).toBeTruthy()
    expect(within(list).getByText('PRIVATE')).toBeTruthy()
    expect(screen.getByText(/@octocat/)).toBeTruthy()

    await user.type(screen.getByRole('searchbox', { name: 'Search repositories' }), 'pay')
    expect(within(list).queryByText('octocat/hello-world')).toBeNull()

    await user.click(screen.getByRole('button', { name: 'Change' }))
    await user.click(screen.getByRole('button', { name: 'Clone octo-org/payments' }))
    expect(await screen.findByText('Cloned octo-org/payments into ~/Code/payments')).toBeTruthy()
    expect(github.cloned).toEqual(['octo-org/payments'])
    const repo = client.getState().repositories['r-gh-octo-org-payments']
    expect(repo).toMatchObject({
      name: 'octo-org/payments',
      source: 'github',
      defaultBranch: 'trunk',
      path: '/Users/you/Code/payments',
      remoteUrl: 'https://github.com/octo-org/payments.git',
    })
    // Added and selected: on to connecting agents.
    expect(screen.getByRole('button', { name: 'Selected' })).toBeTruthy()
    expect(screen.getByRole('button', { name: 'Continue →' })).toHaveProperty('disabled', false)
  })

  it('offers to sign in when GitHub is not signed in on this machine', async () => {
    const client = new MockClient({ seed: starterSeed, latency: 0 })
    render(
      <AstraApp
        client={client}
        platform="mac"
        history={{ memory: '/setup/repo?source=github' }}
        host={{ github: new MockGithub(null) }}
      />,
    )
    const link = await screen.findByRole('link', { name: 'Sign in with GitHub →' })
    expect(link.getAttribute('href')).toContain('/signin')
  })
})
