import { demoSeed, MockClient } from '@astra/client'
import { render, screen, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { describe, expect, it } from 'vitest'
import { AstraApp } from '../src'

function mount(path: string) {
  const client = new MockClient({ seed: demoSeed, latency: 0 })
  render(<AstraApp client={client} platform="mac" history={{ memory: path }} />)
  return { client, user: userEvent.setup() }
}

describe("a mission's agents", () => {
  it('starts a mission where one harness plays two roles, one of them a custom agent type', async () => {
    const { client, user } = mount('/missions/new')
    await user.type(await screen.findByLabelText('Goal'), 'Harden the login endpoint.')
    await user.click(screen.getByRole('button', { name: '+ Add agent' }))
    await user.selectOptions(
      screen.getByRole('combobox', { name: 'Type for Builder on Claude Code' }),
      'type:at-security-reviewer',
    )
    // Picking the type also picked its agent: Codex now builds and reviews for security.
    expect(screen.getByRole('combobox', { name: 'Agent for Security reviewer' })).toHaveProperty('value', 'a-codex')
    await user.click(screen.getByRole('button', { name: 'Remove Planner on Claude Code' }))
    await user.click(screen.getByRole('button', { name: /start mission/i }))
    await screen.findByRole('list', { name: 'Thread' })

    const s = client.getState()
    const mission = Object.values(s.missions).find((m) => m.goal === 'Harden the login endpoint.')
    const team = Object.values(s.missionAgents).filter((m) => m.missionId === mission?.id)
    expect(team.map((m) => m.role).sort()).toEqual(['builder', 'reviewer', 'reviewer', 'tester'])
    const onCodex = team.filter((m) => m.agentId === 'a-codex')
    expect(onCodex.map((m) => [m.role, m.agentTypeId]).sort()).toEqual([
      ['builder', null],
      ['reviewer', 'at-security-reviewer'],
    ])
  })

  it('switches the harness of a working agent from the map', async () => {
    const { client, user } = mount('/missions/m-014/map')
    await screen.findByRole('heading', { name: 'Builder' })
    await user.selectOptions(screen.getByRole('combobox', { name: 'Runs on' }), 'a-claude-code')
    expect(client.getState().missionAgents['ma-014-builder']?.agentId).toBe('a-claude-code')
    expect(client.getState().missionAgents['ma-014-builder']?.statusDetail).toBe('switched from Codex')
    expect(await screen.findByText(/^Claude Code · Anthropic/i)).toBeTruthy()
  })

  it('adds a second builder on the same harness, then removes the reviewer', async () => {
    const { client, user } = mount('/missions/m-014/map')
    await screen.findByRole('heading', { name: 'Builder' })
    await user.click(screen.getByRole('button', { name: '+ Add agent' }))
    const form = screen.getByRole('form', { name: 'Add an agent' })
    await user.selectOptions(within(form).getByLabelText('Runs on'), 'a-codex')
    await user.click(within(form).getByRole('button', { name: 'Add Builder' }))
    // Two boxes, numbered so they can be told apart.
    expect(await screen.findByRole('button', { name: /^Builder 2/ })).toBeTruthy()
    expect(screen.getByRole('button', { name: /^Builder(?! 2)/ })).toBeTruthy()
    const builders = Object.values(client.getState().missionAgents).filter(
      (m) => m.missionId === 'm-014' && m.role === 'builder' && m.agentId === 'a-codex',
    )
    expect(builders).toHaveLength(2)

    await user.click(screen.getByRole('button', { name: /^Reviewer/ }))
    await user.click(screen.getByRole('button', { name: 'Remove from mission' }))
    await user.click(screen.getByRole('button', { name: 'Remove Reviewer' }))
    expect(client.getState().missionAgents['ma-014-reviewer']?.enabled).toBe(false)
    expect(screen.queryByRole('button', { name: /^Reviewer/ })).toBeNull()
  })

  it('creates and deletes a custom agent type on the agents screen', async () => {
    const { client, user } = mount('/agents')
    await user.click(await screen.findByRole('button', { name: '+ New agent type' }))
    const form = screen.getByRole('form', { name: 'New agent type' })
    await user.type(within(form).getByLabelText('Name'), 'Migration planner')
    await user.selectOptions(within(form).getByLabelText('Role'), 'planner')
    await user.type(within(form).getByLabelText('Instructions'), 'Plan schema changes in reversible steps.')
    await user.click(within(form).getByRole('button', { name: 'Save agent type' }))
    const created = Object.values(client.getState().agentTypes).find((t) => t.name === 'Migration planner')
    expect(created).toMatchObject({ role: 'planner', instructions: 'Plan schema changes in reversible steps.' })

    await user.click(screen.getByRole('button', { name: 'Delete Migration planner' }))
    await user.click(screen.getByRole('button', { name: 'Delete Migration planner' }))
    expect(client.getState().agentTypes[created?.id ?? '']).toBeUndefined()
  })
})
