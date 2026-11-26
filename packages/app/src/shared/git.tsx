import { merge, type RaceResult, resolveConflicts } from '@astra/client'
import { Button } from '@astra/ui'
import { useNavigate } from '@tanstack/react-router'
import { useState } from 'react'
import { useAstraState, useClient } from '../context'

/** Wait for the runner before claiming success; a timeout leaves the review open. */
export function useResultMerge(result: RaceResult | undefined) {
  const client = useClient()
  const s = useAstraState()
  const navigate = useNavigate()
  const [notice, setNotice] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)
  const doMerge = async () => {
    if (!result || busy) return
    setBusy(true)
    setNotice(null)
    try {
      const outcome = await merge(client, result.id)
      if (!outcome?.ok) {
        setNotice(outcome?.error ?? 'The runner has not confirmed the merge. Check its connection and retry.')
        return
      }
      await navigate({ to: '/missions/$missionId/merged', params: { missionId: result.missionId } })
    } catch (e) {
      setNotice(e instanceof Error ? e.message : String(e))
    } finally {
      setBusy(false)
    }
  }
  const resolve = async () => {
    const mission = result && s.missions[result.missionId]
    if (!result || !mission || busy) return
    setBusy(true)
    try {
      const id = await resolveConflicts(client, {
        repoId: mission.repoId,
        branch: result.branch,
        into: mission.baseBranch,
        files: result.diff.map((f) => f.path),
      })
      await navigate({ to: '/missions/$missionId', params: { missionId: id } })
    } catch (e) {
      setNotice(e instanceof Error ? e.message : String(e))
    } finally {
      setBusy(false)
    }
  }
  return { doMerge, busy, notice, resolve, conflicts: /conflict/i.test(notice ?? '') }
}
export function MergeNotice({ action }: { action: ReturnType<typeof useResultMerge> }) {
  return action.notice ? (
    <div role="alert" className="flex flex-wrap items-center gap-3 border border-line p-3 text-ink">
      <span>{action.notice}</span>
      {action.conflicts ? (
        <Button size="sm" variant="outline" disabled={action.busy} onClick={action.resolve}>
          AUTO-RESOLVE
        </Button>
      ) : null}
    </div>
  ) : null
}
