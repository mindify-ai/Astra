import { type AgentType, deleteAgentType, type Role, saveAgentType } from '@astra/client'
import { Button, Eyebrow, Field, Select, Spec, TextArea, TextInput } from '@astra/ui'
import { useState } from 'react'
import { useAstraState, useClient } from '../context'
import { agentTypes, connectedAgents, roleName, roles } from '../select'
import { HarnessSelect } from '../shared/agentPicker'

interface Draft {
  id?: string
  name: string
  role: Role
  agentId: string
  instructions: string
}

/** Custom agent types: a base role on a default agent, with its own instructions ("Security reviewer" on Codex). */
export function AgentTypesSection() {
  const s = useAstraState()
  const client = useClient()
  const types = agentTypes(s)
  const connected = connectedAgents(s)
  const [draft, setDraft] = useState<Draft | null>(null)
  const [confirming, setConfirming] = useState<string | null>(null)

  const blank = (): Draft => ({
    name: '',
    role: 'builder',
    agentId: connected[0]?.id ?? '',
    instructions: '',
  })
  const edit = (t: AgentType) =>
    setDraft({ id: t.id, name: t.name, role: t.role, agentId: t.agentId, instructions: t.instructions })
  const save = async () => {
    if (!draft || !draft.name.trim() || !draft.agentId) return
    await saveAgentType(client, draft)
    setDraft(null)
  }

  return (
    <section aria-labelledby="agent-types" className="flex flex-col gap-2">
      <div className="flex items-center justify-between px-3">
        <Eyebrow as="h2" id="agent-types">
          Agent types
        </Eyebrow>
        {draft ? null : (
          <Button
            variant="text"
            size="sm"
            disabled={connected.length === 0}
            onClick={() => setDraft(blank())}
          >
            + New agent type
          </Button>
        )}
      </div>
      <p className="m-0 px-3 text-ink-muted">
        A role on an agent you choose, with its own instructions. Use them when you start a mission or add an
        agent to one.
      </p>
      <div className="border-t border-line">
        {types.map((t) => (
          <div
            key={t.id}
            className="flex min-h-12 flex-wrap items-center gap-x-4 gap-y-1 border-b border-line-subtle px-3 py-2"
          >
            <div className="flex min-w-0 flex-[1_1_280px] flex-col gap-1">
              <span className="flex flex-wrap items-baseline gap-x-3">
                <b className="font-medium">{t.name}</b>
                <Spec>
                  {roleName[t.role]} · {s.agents[t.agentId]?.name ?? 'Unknown agent'}
                </Spec>
              </span>
              {t.instructions ? (
                <span className="line-clamp-2 text-[13px] text-ink-muted">{t.instructions}</span>
              ) : null}
            </div>
            <div className="flex gap-1">
              <Button variant="text" size="sm" aria-label={`Edit ${t.name}`} onClick={() => edit(t)}>
                Edit
              </Button>
              {confirming === t.id ? (
                <>
                  <Button
                    variant="outline"
                    size="sm"
                    onClick={() => {
                      setConfirming(null)
                      void deleteAgentType(client, t.id)
                    }}
                  >
                    Delete {t.name}
                  </Button>
                  <Button variant="text" size="sm" onClick={() => setConfirming(null)}>
                    Keep
                  </Button>
                </>
              ) : (
                <Button
                  variant="text"
                  size="sm"
                  aria-label={`Delete ${t.name}`}
                  onClick={() => setConfirming(t.id)}
                >
                  Delete
                </Button>
              )}
            </div>
          </div>
        ))}
        {types.length === 0 && !draft ? (
          <p className="m-0 border-b border-line-subtle px-3 py-3 text-ink-muted">No agent types yet.</p>
        ) : null}
      </div>
      {draft ? (
        <form
          aria-label={draft.id ? 'Edit agent type' : 'New agent type'}
          className="flex flex-col gap-3 border border-line bg-surface-raised p-4"
          onSubmit={(e) => {
            e.preventDefault()
            void save()
          }}
        >
          <div className="flex flex-wrap gap-3">
            <Field label="Name" className="flex-[1_1_220px]">
              {(id) => (
                <TextInput
                  id={id}
                  value={draft.name}
                  placeholder="Security reviewer"
                  onChange={(e) => setDraft({ ...draft, name: e.target.value })}
                />
              )}
            </Field>
            <Field label="Role" className="flex-[1_1_160px]">
              {(id) => (
                <Select
                  id={id}
                  value={draft.role}
                  onChange={(e) => setDraft({ ...draft, role: e.target.value as Role })}
                >
                  {roles.map((r) => (
                    <option key={r} value={r}>
                      {roleName[r]}
                    </option>
                  ))}
                </Select>
              )}
            </Field>
            <Field label="Runs on" className="flex-[1_1_160px]">
              {(id) => (
                <HarnessSelect
                  s={s}
                  id={id}
                  value={draft.agentId}
                  onChange={(agentId) => setDraft({ ...draft, agentId })}
                />
              )}
            </Field>
          </div>
          <Field label="Instructions">
            {(id) => (
              <TextArea
                id={id}
                rows={4}
                value={draft.instructions}
                placeholder="What this agent should focus on, and how it should report back."
                onChange={(e) => setDraft({ ...draft, instructions: e.target.value })}
              />
            )}
          </Field>
          <div className="flex gap-3">
            <Button type="submit" disabled={!draft.name.trim() || !draft.agentId}>
              Save agent type
            </Button>
            <Button variant="outline" onClick={() => setDraft(null)}>
              Cancel
            </Button>
          </div>
        </form>
      ) : null}
    </section>
  )
}
