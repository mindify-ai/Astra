import { demoSeed, MockClient } from '@astra/client'
import { render, screen, waitFor, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { beforeEach, describe, expect, it } from 'vitest'
import { AstraApp } from '../src'
import type { HostCapabilities } from '../src/context'
import { normalizeUrl, sandboxFor } from '../src/shared/browser'
import { activeToken, commandsFor, harnessOf, rank, sideQuestion } from '../src/shared/harness'
import { messagesOf, sideMessagesOf } from '../src/shared/thread'

const options = () => within(screen.getByRole('listbox')).getAllByRole('option')

function mount(path: string, host: HostCapabilities = {}) {
  const client = new MockClient({ seed: demoSeed, latency: 0 })
  render(<AstraApp client={client} platform="mac" history={{ memory: path }} host={host} />)
  return client
}

beforeEach(() => window.localStorage.clear())

describe('agent output in the thread', () => {
  it('is folded by default, opens one at a time, and can all be shown', async () => {
    mount('/missions/m-014')
    const thread = await screen.findByRole('list', { name: 'Thread' })
    const folds = thread.querySelectorAll('details')
    expect(folds.length).toBeGreaterThan(0)
    for (const d of folds) expect(d.open).toBe(false)
    const summary = folds[0]?.querySelector('summary')
    expect(summary?.textContent).toMatch(/Output · \d+ lines?/)

    const user = userEvent.setup()
    await user.click(summary as HTMLElement)
    expect(folds[0]?.open).toBe(true)

    await user.click(screen.getByRole('button', { name: 'SHOW OUTPUT' }))
    for (const d of screen.getByRole('list', { name: 'Thread' }).querySelectorAll('details'))
      expect(d.open).toBe(true)
    expect(window.localStorage.getItem('astra.thread.output')).toBe('open')
    await user.click(screen.getByRole('button', { name: 'FOLD OUTPUT' }))
    for (const d of screen.getByRole('list', { name: 'Thread' }).querySelectorAll('details'))
      expect(d.open).toBe(false)
  })
})

describe('slash commands and mentions', () => {
  it('offers the Claude Code and Codex commands of the agents being addressed', async () => {
    mount('/missions/m-014')
    const user = userEvent.setup()
    const input = await screen.findByRole('combobox', { name: 'Message' })
    await user.type(input, '/')
    const list = screen.getByRole('listbox', { name: 'Commands' })
    const text = list.textContent ?? ''
    expect(text).toContain('CLAUDE CODE')
    expect(text).toContain('CODEX')
    expect(text).toContain('/btw')

    await user.type(input, 'secur')
    expect(options().map((o) => o.textContent)).toEqual([expect.stringContaining('/security-review')])
    await user.keyboard('{Enter}')
    expect((input as HTMLTextAreaElement).value).toBe('/security-review ')
    expect(screen.queryByRole('listbox')).toBeNull()

    // Addressing the Codex builder leaves only Codex's commands (and Astra's own).
    await user.clear(input)
    await user.selectOptions(screen.getByLabelText('To'), 'ma-014-builder')
    await user.type(input, '/')
    const tags = options().map((o) => o.textContent ?? '')
    expect(tags.some((t) => t.includes('CLAUDE CODE'))).toBe(false)
    expect(tags.some((t) => t.endsWith('CODEX'))).toBe(true)
    await user.keyboard('{Escape}')
    expect(screen.queryByRole('listbox')).toBeNull()
  })

  it('completes @ with agents and files, and picking an agent addresses it', async () => {
    mount('/missions/m-014')
    const user = userEvent.setup()
    const input = await screen.findByRole('combobox', { name: 'Message' })
    await user.type(input, 'look at @')
    const options = within(screen.getByRole('listbox', { name: 'Mentions' })).getAllByRole('option')
    expect(options.some((o) => o.textContent?.includes('AGENT'))).toBe(true)
    expect(options.some((o) => o.textContent?.includes('src/limiter/bucket.ts'))).toBe(true)

    await user.type(input, 'bucket')
    await user.keyboard('{Enter}')
    expect((input as HTMLTextAreaElement).value).toBe('look at @src/limiter/bucket.ts ')

    await user.type(input, '@build')
    await user.keyboard('{Enter}')
    expect((input as HTMLTextAreaElement).value).toBe('look at @src/limiter/bucket.ts @builder ')
    expect((screen.getByLabelText('To') as HTMLSelectElement).value).toBe('ma-014-builder')
  })

  it('sends on Enter and breaks the line on Shift+Enter', async () => {
    const client = mount('/missions/m-014')
    const user = userEvent.setup()
    const input = await screen.findByRole('combobox', { name: 'Message' })
    await user.type(input, 'one{Shift>}{Enter}{/Shift}two')
    expect((input as HTMLTextAreaElement).value).toBe('one\ntwo')
    await user.keyboard('{Enter}')
    expect((input as HTMLTextAreaElement).value).toBe('')
    expect(messagesOf(client.getState(), 'm-014').some((m) => m.body === 'one\ntwo')).toBe(true)
  })
})

describe('side chat', () => {
  it('answers /btw beside the thread without interrupting the agents', async () => {
    const client = mount('/missions/m-014')
    const before = messagesOf(client.getState(), 'm-014').length
    const steering = client.getState().missionAgents['ma-014-builder']?.steeredBy
    const user = userEvent.setup()
    const input = await screen.findByRole('combobox', { name: 'Message' })
    await user.type(input, '/btw what is the builder doing?')
    await user.keyboard('{Escape}{Enter}')

    const panel = await screen.findByRole('region', { name: 'Side chat' })
    await waitFor(() =>
      expect(within(panel).getByRole('list', { name: 'Side thread' }).children).toHaveLength(2),
    )
    const sideList = within(panel).getByRole('list', { name: 'Side thread' })
    expect(within(sideList).getByText('what is the builder doing?')).toBeTruthy()
    expect((within(panel).getByRole('combobox', { name: 'Side thread' }) as HTMLSelectElement).value).toMatch(
      /^side-/,
    )

    const s = client.getState()
    expect(messagesOf(s, 'm-014')).toHaveLength(before)
    const side = Object.values(s.messages).find((m) => m.side)?.side as string
    const thread = sideMessagesOf(s, 'm-014', side)
    expect(thread.map((m) => m.author.type)).toEqual(['person', 'agent'])
    expect(s.missionAgents['ma-014-builder']?.steeredBy).toBe(steering)

    // A follow-up stays in the same side thread.
    await user.type(within(panel).getByRole('combobox', { name: 'Side question' }), 'and the tester?{Enter}')
    await waitFor(() => expect(sideMessagesOf(client.getState(), 'm-014', side)).toHaveLength(4))
    await user.click(within(panel).getByRole('button', { name: 'Close side chat' }))
    expect(screen.queryByRole('region', { name: 'Side chat' })).toBeNull()
  })
})

describe('workbench layout', () => {
  it('puts the chat beside the code and rearranges panes from the layout menu', async () => {
    mount('/code')
    const chat = await screen.findByRole('region', { name: 'Chat' })
    expect(within(chat).getByRole('combobox', { name: 'Message' })).toBeTruthy()

    const user = userEvent.setup()
    await user.click(screen.getByRole('button', { name: 'LAYOUT' }))
    await user.click(screen.getByRole('menuitem', { name: 'Code · browser' }))
    expect(screen.queryByRole('region', { name: 'Chat' })).toBeNull()
    const browser = screen.getByRole('region', { name: 'Browser' })
    expect(JSON.parse(window.localStorage.getItem('astra.layout') ?? '{}').browser).toEqual({
      open: true,
      side: 'right',
    })

    await user.type(within(browser).getByRole('textbox', { name: 'Address' }), 'localhost:3000{Enter}')
    const frame = within(browser).getByTitle('Browser: http://localhost:3000/') as HTMLIFrameElement
    expect(frame.getAttribute('src')).toBe('http://localhost:3000/')
    expect(frame.getAttribute('sandbox')).toContain('allow-scripts')

    await user.click(screen.getByRole('button', { name: 'LAYOUT' }))
    await user.click(screen.getByRole('menuitem', { name: 'Code · chat over browser' }))
    expect(screen.getByRole('region', { name: 'Chat' })).toBeTruthy()
    expect(screen.getByRole('separator', { name: 'Resize right split' })).toBeTruthy()

    await user.click(
      within(screen.getByRole('region', { name: 'Browser' })).getByRole('button', { name: 'Close browser' }),
    )
    expect(screen.queryByRole('region', { name: 'Browser' })).toBeNull()
  })

  it('docks the terminal on the right', async () => {
    const terminal = { open: async () => ({ write() {}, resize() {}, close() {} }) }
    mount('/code', { terminal })
    const user = userEvent.setup()
    await user.click(await screen.findByRole('button', { name: 'TERMINAL' }))
    const dock = await screen.findByRole('region', { name: 'Terminal' })
    await user.click(within(dock).getByRole('button', { name: 'DOCK RIGHT' }))
    const handle = within(dock).getByRole('separator', { name: 'Resize terminal' })
    expect(handle.getAttribute('aria-orientation')).toBe('vertical')
    expect(dock.style.width).toBe(`${handle.getAttribute('aria-valuenow')}px`)
    expect(JSON.parse(window.localStorage.getItem('astra.layout') ?? '{}').terminal).toBe('right')
  })
})

describe('helpers', () => {
  it('normalizes browser addresses', () => {
    expect(normalizeUrl('localhost:3000')).toBe('http://localhost:3000/')
    expect(normalizeUrl('3000')).toBe('http://localhost:3000/')
    expect(normalizeUrl('192.168.1.4:8080/app')).toBe('http://192.168.1.4:8080/app')
    expect(normalizeUrl('example.com')).toBe('https://example.com/')
    expect(normalizeUrl('https://docs.rs/tokio')).toBe('https://docs.rs/tokio')
    expect(normalizeUrl('javascript:alert(1)')).toBeNull()
    expect(normalizeUrl('file:///etc/passwd')).toBeNull()
    expect(normalizeUrl('not a url')).toBeNull()
  })

  it('never lets a same-origin page lift its sandbox', () => {
    expect(sandboxFor(`${window.location.origin}/`)).not.toContain('allow-same-origin')
    expect(sandboxFor('http://localhost:3999/')).toContain('allow-same-origin')
  })

  it('finds the token at the caret', () => {
    expect(activeToken('/comp', 5)).toEqual({ trigger: '/', query: 'comp', start: 0 })
    expect(activeToken('say /comp', 9)).toBeNull()
    expect(activeToken('see @src/a', 10)).toEqual({ trigger: '@', query: 'src/a', start: 4 })
    expect(activeToken('mail a@b', 8)).toBeNull()
    expect(sideQuestion('/btw why?')).toBe('why?')
    expect(sideQuestion('/btw')).toBe('')
    expect(sideQuestion('/btwx')).toBeNull()
  })

  it('maps agents to harnesses and ranks matches', () => {
    const s = new MockClient({ seed: demoSeed, latency: 0 }).getState()
    expect(harnessOf(s.agents['a-claude-code'])).toBe('claude')
    expect(harnessOf(s.agents['a-codex'])).toBe('codex')
    expect(harnessOf(s.agents['a-langgraph'])).toBeNull()
    expect(commandsFor(new Set()).map((c) => c.name)).toEqual(['btw'])
    expect(rank(['src/a/bucket.ts', 'bucket.ts', 'xbucket'], 'bucket', (x) => x)).toEqual([
      'bucket.ts',
      'src/a/bucket.ts',
      'xbucket',
    ])
  })
})
