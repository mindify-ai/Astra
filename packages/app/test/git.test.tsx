import { demoSeed, MockClient, memoryGit, type Op } from '@astra/client'
import { render, screen, waitFor, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { describe, expect, it, vi } from 'vitest'
import { AstraApp } from '../src'

function mount(platform: 'mac' | 'ios' = 'mac') {
  localStorage.removeItem('astra.repo')
  const client = new MockClient({ seed: demoSeed, latency: 0 })
  render(<AstraApp client={client} platform={platform} history={{ memory: '/git' }} />)
  return client
}
describe('Git screen', () => {
  it('places GIT immediately after BOARD, with a working commit form', async () => {
    mount()
    const git = await screen.findByRole('link', { name: 'GIT' })
    expect(git.getAttribute('aria-current')).toBe('page')
    const links = within(screen.getByRole('navigation', { name: 'Views' })).getAllByRole('link')
    expect(links.map((l) => l.textContent).slice(-2)).toEqual(['BOARD', 'GIT'])
    const commit = screen.getByRole('button', { name: 'COMMIT' }) as HTMLButtonElement
    expect(commit.disabled).toBe(true)
    const user = userEvent.setup()
    await user.type(screen.getByRole('textbox', { name: 'Commit message' }), 'Update readme')
    expect(commit.disabled).toBe(false)
    await user.click(commit)
    expect(await screen.findByText('Update readme')).toBeTruthy()
    expect(commit.disabled).toBe(true)
  })
  it('starts a conflict repair and navigates to its mission thread', async () => {
    const client = mount()
    expect(await screen.findByText(/CONFLICTS · \d+ FILES/)).toBeTruthy()
    await userEvent.setup().click(screen.getByRole('button', { name: 'AUTO-RESOLVE' }))
    await waitFor(() =>
      expect(
        Object.values(client.getState().missions).some((m) => m.title.startsWith('Resolve conflicts:')),
      ).toBe(true),
    )
    expect(await screen.findByRole('heading', { name: /Resolve conflicts:/ })).toBeTruthy()
  })
  it('shows a version suggestion and creates a local tag', async () => {
    mount()
    const user = userEvent.setup()
    await user.click(await screen.findByRole('button', { name: 'NEW VERSION' }))
    await user.selectOptions(screen.getByRole('combobox', { name: 'Version increment' }), 'minor')
    await user.click(screen.getByRole('button', { name: 'CREATE v0.3.0' }))
    expect(await screen.findByText('v0.3.0')).toBeTruthy()
  })
  it('renders the phone view and lets the user resolve conflicts without a commit or tag form', async () => {
    mount('ios')
    expect(await screen.findByRole('button', { name: 'AUTO-RESOLVE' })).toBeTruthy()
    expect(screen.queryByRole('textbox', { name: 'Commit message' })).toBeNull()
    expect(screen.queryByRole('button', { name: 'NEW VERSION' })).toBeNull()
  })
  it('disables network actions while offline', async () => {
    const client = new MockClient({ seed: demoSeed, latency: 0 })
    client.setOnline(false)
    render(<AstraApp client={client} platform="mac" history={{ memory: '/git' }} />)
    for (const name of ['PULL', 'PUSH', 'OPEN PR']) {
      const buttons = await screen.findAllByRole('button', { name })
      expect(buttons.every((b) => (b as HTMLButtonElement).disabled)).toBe(true)
    }
    expect(screen.getAllByText(/OFFLINE/).length).toBeGreaterThan(0)
  })
  it('uses the local host for repositories with a folder and shows GitHub errors inline', async () => {
    const seed = structuredClone(demoSeed)
    for (const e of seed.entities) if (e.kind === 'repository' && e.id === 'r-gateway') e.path = '/repo'
    const client = new MockClient({ seed, latency: 0 })
    const git = memoryGit(client.getState())
    const status = vi.spyOn(git, 'status')
    vi.spyOn(git, 'pullRequests').mockRejectedValue('signedOut')
    render(<AstraApp client={client} platform="mac" history={{ memory: '/git' }} host={{ git }} />)
    expect(await screen.findByRole('alert')).toHaveProperty('textContent', 'Sign in to GitHub to continue.')
    expect(status).toHaveBeenCalledWith('/repo')
    expect(await screen.findByText('README.md')).toBeTruthy()
  })
})

class ConflictClient extends MockClient {
  protected override async send(ops: Op[]) {
    const failed = ops.filter((op) => op.body.type === 'command' && op.body.command.type === 'merge')
    if (failed.length) {
      this.receive(
        this.stamp(
          failed.map((op) => ({
            type: 'commandResult' as const,
            commandOp: op.id,
            ok: false,
            error: 'Merge conflict in src/cache.ts',
          })),
          this.nodeId,
        ),
      )
    }
    await super.send(ops.filter((op) => !failed.includes(op)))
  }
}
for (const [path, button] of [
  ['/missions/m-015/review', 'Merge A'],
  ['/missions/m-015/review/res-015-a', 'Approve & merge'],
]) {
  it(`stays in review after a conflict at ${path}`, async () => {
    const client = new ConflictClient({ seed: demoSeed, latency: 0 })
    render(<AstraApp client={client} platform="mac" history={{ memory: path! }} />)
    const user = userEvent.setup()
    await user.click(await screen.findByRole('button', { name: button! }))
    expect(await screen.findByRole('alert')).toHaveProperty(
      'textContent',
      expect.stringContaining('Merge conflict'),
    )
    expect(client.getState().missions['m-015']?.status).not.toBe('merged')
    await user.click(screen.getByRole('button', { name: 'AUTO-RESOLVE' }))
    expect(await screen.findByRole('heading', { name: /Resolve conflicts:/ })).toBeTruthy()
  })
}
