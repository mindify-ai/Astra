import { demoSeed, MockAuth, MockClient, type Session, starterSeed } from '@astra/client'
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

    await user.click(screen.getByRole('link', { name: 'Use this machine only' }))
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
