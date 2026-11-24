import {
  type Adapter,
  type AgentConnection,
  type CommandOutcome,
  type CommandPolicy,
  connectAgent,
  testAgent,
} from '@astra/client'
import {
  Button,
  buttonClass,
  cx,
  Eyebrow,
  Field,
  Fieldset,
  Radio,
  Select,
  Spec,
  Status,
  TextInput,
} from '@astra/ui'
import { Link } from '@tanstack/react-router'
import { useState } from 'react'
import { useAstraState, useClient } from '../context'
import { AppHeader, Page } from '../shell/AppHeader'
import { AgentTypesSection } from './AgentTypes'

const sections: { title: string; category: AgentConnection['category'][] }[] = [
  { title: 'Coding agents', category: ['codingAgent'] },
  { title: 'Harnesses and frameworks', category: ['harness'] },
  { title: 'Local models', category: ['local'] },
  { title: 'Open protocols and anything else', category: ['protocol'] },
]

const defaultAdapter = (a: AgentConnection): Adapter => {
  if (a.adapter) return a.adapter
  if (a.category === 'local') return { type: 'builtin', provider: 'auto', model: null }
  if (a.transport.toLowerCase().includes('endpoint') || a.transport.toLowerCase().includes('url'))
    return { type: 'langgraph', endpoint: 'http://localhost:2024', graph: '' }
  return { type: 'command', command: '', args: [] }
}

function RowStatus({
  agent,
  selected,
  onConnect,
}: {
  agent: AgentConnection
  selected: boolean
  onConnect: () => void
}) {
  if (agent.status === 'connected') return <Status tone="success">Connected</Status>
  if (agent.status === 'offline') return <Status tone="idle">Offline</Status>
  if (agent.status === 'settingUp') return <Status tone="working">Setting up</Status>
  if (selected) return <Status tone="idle">Not connected</Status>
  return (
    <button
      type="button"
      className={buttonClass('outline', 'sm', 'px-3')}
      onClick={onConnect}
      aria-label={`Connect ${agent.name}`}
    >
      CONNECT
    </button>
  )
}

type Note = { tone: 'working' | 'success' | 'danger'; text: string }

/** Keyed by agent id, so a status update from the node does not reset what you are typing. */
function SetupForm({ agent }: { agent: AgentConnection }) {
  const client = useClient()
  const [name, setName] = useState(agent.name)
  const [adapter, setAdapter] = useState<Adapter>(() => defaultAdapter(agent))
  const [runsOn, setRunsOn] = useState(agent.runsOn)
  const [policy, setPolicy] = useState<CommandPolicy>(agent.commandPolicy)
  const [note, setNote] = useState<Note | null>(null)

  const report = (o: CommandOutcome | null, okText: string) =>
    setNote(
      o === null
        ? { tone: 'danger', text: 'This machine did not answer. Is Astra running here?' }
        : o.ok
          ? { tone: 'success', text: okText }
          : { tone: 'danger', text: o.error ?? 'Could not connect.' },
    )
  const save = async () => {
    setNote({ tone: 'working', text: 'Connecting' })
    report(
      await connectAgent(client, agent.id, { name, adapter, runsOn, commandPolicy: policy }),
      'Connected',
    )
  }
  const test = async () => {
    setNote({ tone: 'working', text: 'Testing' })
    report(await testAgent(client, agent.id), 'Reachable from this machine')
  }

  return (
    <form
      className="flex flex-col gap-5"
      onSubmit={(e) => {
        e.preventDefault()
        void save()
      }}
    >
      <div className="flex flex-col gap-2">
        <Eyebrow>Set up</Eyebrow>
        <h2 className="m-0 text-title">{agent.name}</h2>
        <Spec>
          {agent.vendor} ·{' '}
          {agent.category === 'codingAgent'
            ? 'CODING AGENT'
            : agent.category === 'local'
              ? 'BUILT IN'
              : agent.category.toUpperCase()}
        </Spec>
      </div>
      <Field label="Name in Astra">
        {(id) => <TextInput id={id} value={name} onChange={(e) => setName(e.target.value)} />}
      </Field>

      {adapter.type === 'langgraph' ? (
        <>
          <Field label="Endpoint">
            {(id) => (
              <TextInput
                id={id}
                mono
                value={adapter.endpoint}
                onChange={(e) => setAdapter({ ...adapter, endpoint: e.target.value })}
              />
            )}
          </Field>
          <Field label="Graph">
            {(id) => (
              <TextInput
                id={id}
                mono
                value={adapter.graph}
                onChange={(e) => setAdapter({ ...adapter, graph: e.target.value })}
              />
            )}
          </Field>
        </>
      ) : null}
      {adapter.type === 'acp' || adapter.type === 'command' ? (
        <Field label={adapter.type === 'acp' ? 'Command (Agent Client Protocol)' : 'Command'}>
          {(id) => (
            <TextInput
              id={id}
              mono
              value={[adapter.command, ...adapter.args].join(' ')}
              placeholder="my-agent --stdio"
              onChange={(e) => {
                const [command = '', ...args] = e.target.value.split(' ')
                setAdapter({ ...adapter, command, args })
              }}
            />
          )}
        </Field>
      ) : null}
      {adapter.type === 'builtin' ? (
        <>
          <Field label="Model">
            {(id) => (
              <Select
                id={id}
                value={adapter.provider}
                onChange={(e) =>
                  setAdapter({ ...adapter, provider: e.target.value as 'auto' | 'cloud' | 'local' })
                }
              >
                <option value="auto">Claude online, local model offline</option>
                <option value="cloud">Claude only</option>
                <option value="local">Local model only</option>
              </Select>
            )}
          </Field>
          <Field label="Local model (Ollama, LM Studio or llama.cpp)">
            {(id) => (
              <TextInput
                id={id}
                mono
                placeholder="Detected automatically"
                value={adapter.model ?? ''}
                onChange={(e) => setAdapter({ ...adapter, model: e.target.value || null })}
              />
            )}
          </Field>
        </>
      ) : null}

      <Fieldset legend="Runs on">
        <Radio name="runs-on" checked={runsOn === 'local'} onChange={() => setRunsOn('local')}>
          This machine
        </Radio>
        <Radio name="runs-on" checked={runsOn === 'server'} onChange={() => setRunsOn('server')}>
          Your server
        </Radio>
      </Fieldset>
      <Field label="Before it runs a command">
        {(id) => (
          <Select id={id} value={policy} onChange={(e) => setPolicy(e.target.value as CommandPolicy)}>
            <option value="askMe">Ask me</option>
            <option value="askSteerers">Ask anyone who can steer</option>
            <option value="auto">Run without asking</option>
          </Select>
        )}
      </Field>
      <div className="flex flex-wrap gap-3">
        <Button type="submit">Connect agent</Button>
        <Button variant="outline" onClick={test}>
          Test
        </Button>
      </div>
      <div role="status" className="min-h-5 text-[13px]">
        {note ? <Status tone={note.tone}>{note.text}</Status> : null}
      </div>
      <Spec as="p" className="m-0 border-t border-line pt-4">
        Keys stay on this machine. Astra stores none.
      </Spec>
    </form>
  )
}

/** 3 · Connect agents (setup step 2 of 2). */
export function Connect() {
  const state = useAstraState()
  const agents = Object.values(state.agents)
  const [selectedId, setSelectedId] = useState<string>(
    () => agents.find((a) => a.id === 'a-langgraph')?.id ?? agents[0]?.id ?? '',
  )
  const selected = state.agents[selectedId]

  return (
    <Page
      header={
        <AppHeader
          repoSelect
          nav={null}
          actions={
            <Link to="/" className={buttonClass('primary', 'sm')}>
              CONTINUE →
            </Link>
          }
        />
      }
    >
      <div className="flex grow flex-wrap">
        <main className="flex min-w-0 flex-[999_1_560px] flex-col gap-6 p-8">
          <div className="flex max-w-[640px] flex-col gap-2">
            <Eyebrow>Agents</Eyebrow>
            <h1 className="m-0 text-title">Connect a vendor or harness</h1>
            <p className="m-0 text-ink-muted">
              Astra works with any agent that runs in a terminal, behind an endpoint or over an open protocol.
              Each agent keeps its own keys and runs where you put it.
            </p>
          </div>
          {sections.map((sec) => {
            const rows = agents.filter((a) => sec.category.includes(a.category))
            if (rows.length === 0) return null
            return (
              <section key={sec.title} className="flex flex-col gap-2">
                <Eyebrow as="h2" className="px-3">
                  {sec.title}
                </Eyebrow>
                <ul className="m-0 list-none border-t border-line p-0">
                  {rows.map((a) => {
                    const isSel = a.id === selectedId
                    return (
                      <li
                        key={a.id}
                        aria-current={isSel ? 'true' : undefined}
                        className={cx(
                          'flex min-h-11 flex-wrap items-center gap-x-4 gap-y-1 border-b border-line-subtle px-3 py-1',
                          isSel && 'bg-signal-soft',
                        )}
                      >
                        <button
                          type="button"
                          onClick={() => setSelectedId(a.id)}
                          className="flex-[1_1_180px] border-0 bg-transparent p-0 text-left font-medium text-ink"
                        >
                          {a.name}
                        </button>
                        <Spec className="flex-[0_0_120px]">{a.vendor}</Spec>
                        <span className="flex-[1_1_180px] text-ink-muted">{a.transport}</span>
                        <span className="flex flex-[0_0_116px] justify-end">
                          <RowStatus agent={a} selected={isSel} onConnect={() => setSelectedId(a.id)} />
                        </span>
                      </li>
                    )
                  })}
                </ul>
              </section>
            )
          })}
          <AgentTypesSection />
        </main>
        <aside className="box-border flex-[1_1_360px] border-l border-line bg-surface-alt px-6 py-8">
          {selected ? <SetupForm key={selected.id} agent={selected} /> : null}
        </aside>
      </div>
    </Page>
  )
}
