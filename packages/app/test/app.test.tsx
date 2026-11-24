import { demoSeed, MockClient, starterSeed } from '@astra/client'
import { render, screen, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { describe, expect, it } from 'vitest'
import { AstraApp } from '../src'
import type { Platform } from '../src/context'

function mount(path: string, platform: Platform = 'mac') {
  const client = new MockClient({ seed: demoSeed, latency: 0 })
  const utils = render(<AstraApp client={client} platform={platform} history={{ memory: path }} />)
  return { client, ...utils }
}

describe('desktop', () => {
  it('shows a terminal dock only when the host can run shells', async () => {
    const opened: (string | null)[] = []
    const terminal = {
      open: async (o: { cwd: string | null }) => {
        opened.push(o.cwd)
        return { write() {}, resize() {}, close() {} }
      },
    }
    const client = new MockClient({ seed: demoSeed, latency: 0 })
    const { unmount } = render(
      <AstraApp client={client} platform="mac" history={{ memory: '/' }} host={{ terminal }} />,
    )
    const user = userEvent.setup()
    await user.click(await screen.findByRole('button', { name: 'TERMINAL' }))
    expect(await screen.findByRole('region', { name: 'Terminal' })).toBeTruthy()
    expect(screen.getByRole('tab', { selected: true })).toBeTruthy()
    await user.click(screen.getByRole('button', { name: 'HIDE' }))
    expect(screen.queryByRole('region', { name: 'Terminal' })).toBeNull()
    unmount()

    mount('/')
    await screen.findByRole('heading', { name: 'Start a mission.' })
    expect(screen.queryByRole('button', { name: 'TERMINAL' })).toBeNull()
  })

  it('resizes the terminal dock from its top edge with the keyboard', async () => {
    window.localStorage.removeItem('astra.size.terminal')
    const terminal = { open: async () => ({ write() {}, resize() {}, close() {} }) }
    const client = new MockClient({ seed: demoSeed, latency: 0 })
    render(<AstraApp client={client} platform="mac" history={{ memory: '/' }} host={{ terminal }} />)
    const user = userEvent.setup()
    await user.click(await screen.findByRole('button', { name: 'TERMINAL' }))
    const dock = await screen.findByRole('region', { name: 'Terminal' })
    const handle = within(dock).getByRole('separator', { name: 'Resize terminal' })
    expect(handle.getAttribute('aria-orientation')).toBe('horizontal')
    const before = Number(handle.getAttribute('aria-valuenow'))
    expect(dock.style.height).toBe(`${before}px`)

    handle.focus()
    await user.keyboard('{ArrowUp}')
    expect(Number(handle.getAttribute('aria-valuenow'))).toBe(before + 16)
    expect(dock.style.height).toBe(`${before + 16}px`)
    expect(window.localStorage.getItem('astra.size.terminal')).toBe(String(before + 16))

    await user.keyboard('{Home}')
    expect(dock.style.height).toBe('120px')
  })

  it('a clean workspace starts at welcome with every agent ready to connect', async () => {
    const client = new MockClient({ seed: starterSeed, latency: 0 })
    render(<AstraApp client={client} platform="mac" history={{ memory: '/' }} />)
    expect(await screen.findByRole('heading', { name: 'Every agent. One workspace.' })).toBeTruthy()
    const s = client.getState()
    expect(Object.keys(s.missions)).toHaveLength(0)
    expect(Object.keys(s.repositories)).toHaveLength(0)
    expect(Object.values(s.agents).every((a) => a.status === 'notConnected')).toBe(true)
  })

  it('home lists what needs you, missions and connected agents', async () => {
    mount('/')
    expect(await screen.findByRole('heading', { name: 'Start a mission.' })).toBeTruthy()
    expect(screen.getByText('Needs you · 2')).toBeTruthy()
    expect(screen.getByText('Approve migration 0042 on staging')).toBeTruthy()
    expect(screen.getByText('Agents · 4 connected')).toBeTruthy()
  })

  it('approves an agent command from the thread', async () => {
    const { client } = mount('/missions/m-014')
    const user = userEvent.setup()
    await user.click(await screen.findByRole('button', { name: 'Approve' }))
    expect(client.getState().approvals['ap-0042']?.state).toBe('approved')
    expect(await screen.findByText('Approved by Mark')).toBeTruthy()
  })

  it('starts a mission from home and lands on its thread', async () => {
    const { client } = mount('/')
    const user = userEvent.setup()
    await user.type(await screen.findByLabelText('Goal'), 'Add request tracing to the gateway.')
    await user.click(screen.getByRole('button', { name: /start mission/i }))
    const thread = await screen.findByRole('list', { name: 'Thread' })
    expect(within(thread).getByText('Add request tracing to the gateway.')).toBeTruthy()
    const created = Object.values(client.getState().missions).find((m) => m.number === 16)
    expect(created).toMatchObject({ title: 'Add request tracing to the gateway', status: 'working' })
  })

  it('merges the reviewer pick and shows the summary', async () => {
    mount('/missions/m-015/review')
    const user = userEvent.setup()
    await user.click(await screen.findByRole('button', { name: 'Merge A' }))
    expect(await screen.findByRole('heading', { name: 'Mission complete.' })).toBeTruthy()
    expect(screen.getByText('astra/pricing-cache-b')).toBeTruthy()
  })

  it('the command palette opens with ⌘K, filters, and runs the chosen item', async () => {
    mount('/')
    const user = userEvent.setup()
    await screen.findByRole('heading', { name: 'Start a mission.' })
    await user.keyboard('{Meta>}k{/Meta}')
    const search = await screen.findByRole('combobox', { name: /search missions/i })
    await user.type(search, 'board')
    const results = screen.getByRole('listbox', { name: 'Results' })
    expect(within(results).getAllByRole('option')[0]?.textContent).toContain('Board')
    await user.keyboard('{Enter}')
    expect(await screen.findByRole('heading', { name: /board/i })).toBeTruthy()
    expect(screen.queryByRole('combobox', { name: /search missions/i })).toBeNull()

    await user.click(screen.getByRole('button', { name: 'Command palette' }))
    await user.type(await screen.findByRole('combobox', { name: /search missions/i }), 'Add dark mode')
    await user.click(
      within(screen.getByRole('listbox', { name: 'Results' })).getByRole('option', {
        name: /Start a mission: Add dark mode/,
      }),
    )
    expect(((await screen.findByLabelText('Goal')) as HTMLTextAreaElement).value).toBe('Add dark mode')
  })

  it('opens the people panel from SHARE and changes access', async () => {
    const { client } = mount('/missions/m-014')
    const user = userEvent.setup()
    await user.click(await screen.findByRole('button', { name: 'SHARE' }))
    await user.selectOptions(await screen.findByLabelText('Access for Jo'), 'steer')
    expect(client.getState().memberships['m-014:p-jo']?.access).toBe('steer')
  })

  it.each(['mac', 'windows', 'linux'] as const)('draws %s window controls', async (platform) => {
    mount('/board', platform)
    expect(await screen.findByLabelText('Close window')).toBeTruthy()
  })

  it('shows the offline badge when the client loses its connection', async () => {
    const { client } = mount('/')
    await screen.findByText('Needs you · 2')
    client.setOnline(false)
    expect(await screen.findByText('OFFLINE')).toBeTruthy()
  })
})

describe('phones', () => {
  it('iOS puts approve before deny; Android puts deny first', async () => {
    mount('/missions/m-014', 'ios')
    let buttons = (await screen.findAllByRole('button'))
      .map((b) => b.textContent)
      .filter((t) => t === 'Approve' || t === 'Deny')
    expect(buttons).toEqual(['Approve', 'Deny'])
    document.body.innerHTML = ''
    mount('/missions/m-014', 'android')
    buttons = (await screen.findAllByRole('button'))
      .map((b) => b.textContent)
      .filter((t) => t === 'Approve' || t === 'Deny')
    expect(buttons).toEqual(['Deny', 'Approve'])
  })

  it('switches results with the segmented control and merges', async () => {
    const { client } = mount('/missions/m-015/review', 'ios')
    const user = userEvent.setup()
    await user.click(await screen.findByRole('button', { name: 'Result B' }))
    await user.click(screen.getByRole('button', { name: 'Merge B' }))
    expect(client.getState().merges['m-015']?.resultId).toBe('res-015-b')
  })
})
