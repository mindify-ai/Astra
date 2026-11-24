import { demoSeed, MockClient, sendMessage } from '@astra/client'
import { render, screen, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { beforeEach, describe, expect, it } from 'vitest'
import { AstraApp } from '../src'

const PATH = '/missions/m-015/review/res-015-a'
const FILE = 'src/pricing/handler.ts'

function mount(path = PATH, client = new MockClient({ seed: demoSeed, latency: 0 })) {
  const utils = render(<AstraApp client={client} platform="mac" history={{ memory: path }} />)
  return { client, ...utils }
}

beforeEach(() => {
  try {
    window.localStorage.clear()
  } catch {
    // no storage in this environment
  }
})

describe('code review', () => {
  it('lists the files and renders a numbered diff, unified or split', async () => {
    mount()
    expect(await screen.findByRole('heading', { name: 'Result A' })).toBeTruthy()
    expect(screen.getByText('astra/pricing-cache-a')).toBeTruthy()
    expect(screen.getByText('Tests passed')).toBeTruthy()

    const files = screen.getByRole('complementary', { name: 'Files' })
    expect(within(files).getByRole('button', { name: FILE })).toBeTruthy()
    expect(within(files).getByText('+5 −1')).toBeTruthy()

    const diff = screen.getByRole('table', { name: `Diff of ${FILE}` })
    expect(within(diff).getByText('if (hit) return hit')).toBeTruthy()
    expect(within(diff).getByRole('button', { name: `Comment on ${FILE} removed line 2` })).toBeTruthy()
    // The closing brace is old line 3, new line 7.
    expect(within(diff).getByRole('button', { name: `Comment on ${FILE} line 7` }).textContent).toBe('37')

    const user = userEvent.setup()
    await user.click(screen.getByRole('button', { name: 'split' }))
    expect(screen.getByRole('button', { name: 'split' }).getAttribute('aria-pressed')).toBe('true')
    const split = screen.getByRole('table', { name: `Diff of ${FILE}` })
    // Removal and first addition share a row.
    const row = within(split).getByText('return db.pricing.all()').closest('tr')
    expect(row?.textContent).toContain("const hit = await cache.get('pricing')")
  })

  it('posts a line comment to the thread and shows it inline under the line', async () => {
    const client = new MockClient({ seed: demoSeed, latency: 0 })
    await sendMessage(client, 'm-015', null, `Result B · \`${FILE}:3\` — Only for B.`)
    mount(PATH, client)
    const user = userEvent.setup()
    await user.click(await screen.findByRole('button', { name: `Comment on ${FILE} line 3` }))
    await user.type(
      screen.getByRole('textbox', { name: `Comment on ${FILE} line 3` }),
      'Version the cache key.',
    )
    await user.click(screen.getByRole('button', { name: 'Comment' }))

    const thread = await screen.findByRole('list', { name: `Comments on ${FILE} line 3` })
    expect(within(thread).getByText('Version the cache key.')).toBeTruthy()
    expect(within(thread).getByText('Mark')).toBeTruthy()
    expect(within(thread).queryByText('Only for B.')).toBeNull()
    expect(screen.queryByRole('textbox', { name: `Comment on ${FILE} line 3` })).toBeNull()
    const bodies = Object.values(client.getState().messages).map((m) => m.body)
    expect(bodies).toContain(`Result A · \`${FILE}:3\` — Version the cache key.`)
  })

  it('marks a file viewed, which collapses it', async () => {
    mount()
    const user = userEvent.setup()
    await user.click(await screen.findByRole('checkbox', { name: `Viewed ${FILE}` }))
    expect(screen.getByText('1 of 1 viewed')).toBeTruthy()
    expect(screen.queryByRole('table', { name: `Diff of ${FILE}` })).toBeNull()
    const header = screen.getByRole('button', { name: new RegExp(`^${FILE}`), expanded: false })
    await user.click(header)
    expect(screen.getByRole('table', { name: `Diff of ${FILE}` })).toBeTruthy()
  })

  it('requests changes through the thread', async () => {
    const { client } = mount()
    const user = userEvent.setup()
    await user.click(await screen.findByRole('button', { name: 'Request changes' }))
    await user.type(screen.getByLabelText('What should change in result A?'), 'Add a cache test.')
    await user.click(screen.getByRole('button', { name: 'Send request' }))
    expect(await screen.findByText('Changes requested')).toBeTruthy()
    const sent = Object.values(client.getState().messages).find(
      (m) => m.body === 'Result A · Changes requested — Add a cache test.',
    )
    expect(sent?.to).toBe('ma-015-builder-a')
  })

  it('opens from the results page and merges with APPROVE & MERGE', async () => {
    mount('/missions/m-015/review')
    const user = userEvent.setup()
    const links = await screen.findAllByRole('link', { name: 'Review code →' })
    await user.click(links[0]!)
    await user.click(await screen.findByRole('button', { name: 'Approve & merge' }))
    expect(await screen.findByRole('heading', { name: 'Mission complete.' })).toBeTruthy()
  })
})
