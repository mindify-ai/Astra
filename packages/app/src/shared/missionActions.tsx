import { archiveMission, type CommandOutcome, deleteMission, type Id, mayManage } from '@astra/client'
import { Button, Status } from '@astra/ui'
import { useNavigate } from '@tanstack/react-router'
import { useState } from 'react'
import { useAstraState, useClient } from '../context'
import { missionNo } from '../select'

const failure = (o: CommandOutcome | null) =>
  o === null ? 'The runner node did not answer. Try again when it is online.' : o.ok ? null : o.error

function useMissionCommand() {
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const run = async (f: () => Promise<CommandOutcome | null>) => {
    setBusy(true)
    setError(null)
    const e = failure(await f())
    setBusy(false)
    setError(e)
    return e === null
  }
  return { busy, error, run }
}

/** ARCHIVE / UNARCHIVE and DELETE for a mission, for its owner. Deleting asks first. */
export function MissionActions({ missionId, compact }: { missionId: Id; compact?: boolean }) {
  const s = useAstraState()
  const client = useClient()
  const navigate = useNavigate()
  const { busy, error, run } = useMissionCommand()
  const [confirm, setConfirm] = useState(false)
  const mission = s.missions[missionId]
  if (!mission || !mayManage(s, missionId, s.me)) return null
  const no = missionNo(mission)
  return (
    <span className="flex flex-wrap items-center gap-2">
      {confirm ? (
        <>
          <span className="text-[13px] text-ink-muted">Delete mission {no} and its history?</span>
          <Button
            variant="outline"
            size="sm"
            disabled={busy}
            onClick={async () => {
              if (await run(() => deleteMission(client, missionId))) {
                setConfirm(false)
                if (!compact) void navigate({ to: '/' })
              }
            }}
          >
            Delete mission {no}
          </Button>
          <Button variant="text" size="sm" onClick={() => setConfirm(false)}>
            Keep
          </Button>
        </>
      ) : (
        <>
          <Button
            variant={compact ? 'text' : 'outline'}
            size="sm"
            disabled={busy}
            onClick={() => void run(() => archiveMission(client, missionId, !mission.archivedAt))}
          >
            {mission.archivedAt ? 'Unarchive' : 'Archive'}
          </Button>
          <Button variant="text" size="sm" disabled={busy} onClick={() => setConfirm(true)}>
            Delete
          </Button>
        </>
      )}
      {error ? (
        <span role="alert" className="text-[13px] text-danger">
          {error}
        </span>
      ) : null}
    </span>
  )
}

/** Shown on an archived mission's screens. */
export function ArchivedBanner({ missionId }: { missionId: Id }) {
  const s = useAstraState()
  const mission = s.missions[missionId]
  if (!mission?.archivedAt) return null
  return (
    <div className="flex flex-wrap items-center gap-x-4 gap-y-2 border-b border-line bg-surface-alt px-8 py-3">
      <Status tone="idle">Archived</Status>
      <span className="text-[13px] text-ink-muted">
        Its agents are stopped and it is hidden from your missions. Unarchive it to work on it again.
      </span>
    </div>
  )
}
