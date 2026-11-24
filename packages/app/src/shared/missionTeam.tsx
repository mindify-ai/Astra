import {
  addAgent,
  type CommandOutcome,
  type Id,
  type MissionAgent,
  removeAgent,
  switchAgent,
} from '@astra/client'
import { Button, Eyebrow, Field } from '@astra/ui'
import { useState } from 'react'
import { useAstraState, useClient } from '../context'
import { canSteer, missionAgentName } from '../select'
import { type AgentChoice, firstChoice, HarnessSelect, TypeSelect } from './agentPicker'

/** Whether people may change a mission's agents (steer access, and the mission isn't merged). */
export function useCanChangeTeam(missionId: Id): boolean {
  const s = useAstraState()
  const mission = s.missions[missionId]
  return !!mission && mission.status !== 'merged' && canSteer(s, s.me, missionId)
}

const failure = (o: CommandOutcome | null) =>
  o === null ? 'The runner node did not answer. Try again when it is online.' : o.ok ? null : o.error

/** Switch the agent (vendor or harness) that runs this role, or take it off the mission. */
export function AgentControls({ ma }: { ma: MissionAgent }) {
  const s = useAstraState()
  const client = useClient()
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [confirm, setConfirm] = useState(false)
  const name = missionAgentName(s, ma)
  const run = async (f: () => Promise<CommandOutcome | null>) => {
    setBusy(true)
    setError(null)
    const e = failure(await f())
    setBusy(false)
    setError(e)
    return e === null
  }
  return (
    <div className="flex flex-col gap-3">
      <Field label="Runs on">
        {(id) => (
          <HarnessSelect
            s={s}
            id={id}
            value={ma.agentId}
            onChange={(agentId) => void run(() => switchAgent(client, ma.id, agentId))}
          />
        )}
      </Field>
      <div className="flex flex-wrap items-center gap-2">
        {confirm ? (
          <>
            <Button
              variant="outline"
              size="sm"
              disabled={busy}
              onClick={async () => {
                if (await run(() => removeAgent(client, ma.id))) setConfirm(false)
              }}
            >
              Remove {name}
            </Button>
            <Button variant="text" size="sm" onClick={() => setConfirm(false)}>
              Keep
            </Button>
          </>
        ) : (
          <Button variant="text" size="sm" disabled={busy} onClick={() => setConfirm(true)}>
            Remove from mission
          </Button>
        )}
      </div>
      {error ? (
        <p role="alert" className="m-0 text-[13px] text-danger">
          {error}
        </p>
      ) : null}
    </div>
  )
}

/** Add a role or custom agent type to a mission, run by a connected agent. */
export function AddAgentForm({
  missionId,
  onAdded,
}: {
  missionId: Id
  onAdded?: (missionAgentId: Id) => void
}) {
  const s = useAstraState()
  const client = useClient()
  const [choice, setChoice] = useState<AgentChoice | null>(null)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const start = firstChoice(s)
  if (!start) return null
  if (!choice)
    return (
      <div>
        <Button variant="outline" size="sm" onClick={() => setChoice(start)}>
          + Add agent
        </Button>
      </div>
    )
  return (
    <form
      aria-label="Add an agent"
      className="flex flex-col gap-3 border border-line bg-surface-raised p-4"
      onSubmit={async (e) => {
        e.preventDefault()
        setBusy(true)
        setError(null)
        const { missionAgentId, outcome } = await addAgent(
          client,
          missionId,
          choice.role,
          choice.agentId,
          choice.agentTypeId,
        )
        setBusy(false)
        const err = failure(outcome)
        setError(err)
        if (!err) {
          setChoice(null)
          onAdded?.(missionAgentId)
        }
      }}
    >
      <Eyebrow>Add an agent</Eyebrow>
      <Field label="Type">{(id) => <TypeSelect s={s} id={id} value={choice} onChange={setChoice} />}</Field>
      <Field label="Runs on">
        {(id) => (
          <HarnessSelect
            s={s}
            id={id}
            value={choice.agentId}
            onChange={(agentId) => setChoice({ ...choice, agentId })}
          />
        )}
      </Field>
      {error ? (
        <p role="alert" className="m-0 text-[13px] text-danger">
          {error}
        </p>
      ) : null}
      <div className="flex gap-3">
        <Button type="submit" size="sm" disabled={busy}>
          Add {missionAgentName(s, choice)}
        </Button>
        <Button variant="text" size="sm" onClick={() => setChoice(null)}>
          Cancel
        </Button>
      </div>
    </form>
  )
}
