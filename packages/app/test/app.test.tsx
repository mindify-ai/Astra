import { demoSeed, MockClient } from '@astra/client'
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
