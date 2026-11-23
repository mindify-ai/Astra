# Astra protocol

The contract shared by the Rust core (`crates/astra-protocol`) and the TypeScript client
(`packages/client`). JSON field names are **camelCase**, enum values are **camelCase strings**, and tagged
unions use a `type` field (entities use `kind`). Optional values are `null`, never omitted.

## Entities

Every entity has `kind` and `id`. Clients hold a normalized `State` with one map per kind.

| kind           | fields                                                                                                                                                                                                                                                                                       |
| -------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `person`       | `id, name, initials`                                                                                                                                                                                                                                                                         |
| `repository`   | `id, name, source: github\|gitlab\|local, defaultBranch, path: string\|null, remoteUrl: string\|null`                                                                                                                                                                                        |
| `agent`        | `id, name, vendor, category: codingAgent\|harness\|protocol\|local, transport, adapter: Adapter\|null, runsOn: local\|server, nodeId: string\|null, commandPolicy: askMe\|askSteerers\|auto, status: connected\|settingUp\|notConnected\|offline, requiresNetwork: bool`                     |
| `mission`      | `id, number, title, shortName, goal, mode: split\|race, repoId, baseBranch, branch, status: draft\|working\|needsYou\|done\|merged, approvalPolicy: owner\|steerers, ownerId, nodeId: string\|null, stats: DiffStats\|null, createdAt`                                                       |
| `missionAgent` | `id, missionId, agentId, role: planner\|builder\|tester\|reviewer\|writer, enabled: bool, status: idle\|queued\|working\|needsYou\|done\|offline, statusDetail: string\|null, steeredBy: string\|null, task: string\|null, startedAt: string\|null, branch: string\|null, latest: LogLine[]` |
| `task`         | `id, missionId, title, role, missionAgentId: string\|null, column: queued\|working\|needsYou\|done, note: string\|null, mergedBy: string\|null, approvalId: string\|null, order: number`                                                                                                     |
| `message`      | `id, missionId, author: Author, steering: string\|null, to: string\|null, body, plan: PlanStep[], log: LogLine[], approvalId: string\|null, at`                                                                                                                                              |
| `approval`     | `id, missionId, missionAgentId, prompt, command: string\|null, state: pending\|approved\|denied, decidedBy: string\|null, decidedAt: string\|null`                                                                                                                                           |
| `result`       | `id, missionId, label, missionAgentId, branch, stats: DiffStats, checks: passed\|failed\|pending\|none, reviewerPick: bool, diff: DiffFile[]`                                                                                                                                                |
| `merge`        | `id, missionId, resultId: string\|null, into, stats: DiffStats, checks, approvedBy, note: string\|null, keptBranches: string[], at`                                                                                                                                                          |
| `membership`   | `id` (= `"<missionId>:<personId>"`), `missionId, personId, access: owner\|steer\|watch`                                                                                                                                                                                                      |

Value types:

- `Adapter` =
  `{type:"acp", command, args: string[]}` |
  `{type:"langgraph", endpoint, graph}` |
  `{type:"command", command, args: string[]}` |
  `{type:"builtin", provider: auto|cloud|local, model: string|null}`
- `Author` = `{type:"person", personId}` | `{type:"agent", missionAgentId}`
- `LogLine` = `{verb, target}`; `PlanStep` = `{text, state: done|active|pending}`
- `DiffStats` = `{files, additions, deletions}`
- `DiffFile` = `{path, lines: {kind: ctx|add|del, text}[]}`
- Timestamps are RFC 3339 UTC strings.

`State` = `{ workspaceId, me, people, repositories, agents, missions, missionAgents, tasks, messages,
approvals, results, merges, memberships, presence }`. Each entity map is `Record<id, Entity>`.
`presence` is `Presence[]`, where `Presence` = `{personId, missionId: string|null, activity, at}`. Presence is
ephemeral: it's relayed but never stored in the op log.

## Ops

All changes are ops. An op is `{id, hlc, actor, body}`:

- `id`: a ULID.
- `hlc`: a hybrid logical clock string `"<ms 16 digits>:<counter 6 digits>:<nodeId>"`, so string order is
  causal order.
- `actor`: the personId or nodeId that made the op.

| body `type`     | fields                               | semantics                                                                                               |
| --------------- | ------------------------------------ | ------------------------------------------------------------------------------------------------------- |
| `put`           | `entity`                             | Create or replace. Last writer wins by `hlc` per entity.                                                |
| `patch`         | `entityKind, id, fields`             | Last writer wins **per field** by `hlc`. Ignored if the entity is unknown.                              |
| `delete`        | `entityKind, id`                     | Tombstone.                                                                                              |
| `command`       | `node, command`                      | A side effect addressed to runner node `node`. Executed **exactly once** by that node (dedup by op id). |
| `commandResult` | `commandOp, ok, error: string\|null` | Appended by the node after it executes a command.                                                       |

`Command` (tagged by `type`):

- `startMission{missionId}`
- `approve{approvalId}`, `deny{approvalId}`
- `sendMessage{missionId, to: string|null, body}`
- `takeSteering{missionAgentId}`, `releaseSteering{missionAgentId}`
- `pause{missionAgentId}`, `resume{missionAgentId}`
- `merge{resultId}`
- `connectAgent{agentId}`, `testAgent{agentId}`

A client creates a mission by issuing `put` ops for the mission, its missionAgents and its memberships, then a
`startMission` command.

### Rules for runner nodes

- **Authorization.** `approve`/`deny` are honoured only from the mission owner, or from members with `steer` access
  when `approvalPolicy = steerers`. `sendMessage` with `to` set requires the actor to hold that agent's
  steering lock, or to take it implicitly when it is free.
- **Agent ↔ approval mapping.** An agent permission request becomes an `approval` put plus a missionAgent patch
  `status: needsYou`. The decision resumes or cancels the agent.

### Who writes what

| command            | client writes                                                                             | node writes                                                                                                                                                                                                                                           |
| ------------------ | ----------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `sendMessage`      | The `message` put, `author.type = person`                                                 | Delivers `body` to the target agent, or to every enabled agent when `to` is null; writes no duplicate message. If `to` is set and free, patches `steeredBy = actor`; if someone else holds it, rejects.                                               |
| `approve` / `deny` | Only the command                                                                          | Patches the approval (`state, decidedBy, decidedAt`), the missionAgent status (`working`/`idle`) and the linked task's column, then resumes or cancels the agent.                                                                                     |
| `merge`            | Only the command                                                                          | Writes the `merge` entity (`id = missionId`), patches mission `status = merged`, and patches the linked `needsYou` task to `done` with `mergedBy`.                                                                                                    |
| `startMission`     | Mission put (`status = draft`, `number = max + 1`), missionAgents (`queued`), memberships | Patches mission `status = working`, then writes tasks/messages as agents run. Agent messages use `author.type = agent`; plans go in `plan`, edits and commands in `log` (`verb` = edit\|run\|read). `missionAgent.latest` keeps the last 3 log lines. |

## Sync wire protocol

WebSocket at `/v1/sync`. Frames are MessagePack by default; JSON with `?format=json`. Frames are tagged by `t`.

| direction       | frames                                                                                                                  |
| --------------- | ----------------------------------------------------------------------------------------------------------------------- |
| client → server | `{t:"hello", workspaceId, nodeId, cursor, token}`, `{t:"push", ops}`, `{t:"presence", presence}`                        |
| server → client | `{t:"welcome", cursor}`, `{t:"ops", ops, cursor}`, `{t:"ack", ids}`, `{t:"presence", presence}`, `{t:"error", message}` |

- `cursor` is the server's per-workspace sequence number.
- On `hello` the server replays every op after `cursor`.
- Pushed ops are deduplicated by `id`.
- A runner node receives `command` ops addressed to it; commands for a node that is offline stay in the log
  until it reconnects.

The same protocol runs over the LAN: a phone talks directly to a desktop node over a Noise-encrypted WebSocket
after QR pairing.

## Local API (Tauri and LAN)

| call                         | returns / does                                       |
| ---------------------------- | ---------------------------------------------------- |
| `snapshot()`                 | `{state, cursor}`                                    |
| `apply(ops)`                 | Appends ops locally and syncs them.                  |
| event `astra://ops`          | `{ops, cursor}`                                      |
| event `astra://presence`     | `Presence[]`                                         |
| event `astra://connectivity` | `{online, relay: connected\|disconnected\|disabled}` |

Clients reduce ops into `State` with the same rules in Rust and TypeScript. `fixtures/ops-conformance.json`
holds shared cases that both implementations must pass.

## Auth (relay HTTP API)

JSON, camelCase. The relay proxies the GitHub/GitLab OAuth **device flow**, because the providers' device
endpoints don't allow browser CORS. The provider's access token is used once to read the profile and then
discarded.

| Request                                                           | Response                                                                                                                                                                |
| ----------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `POST /v1/auth/device/start` `{provider: github\|gitlab}`         | `200 {deviceCode, userCode, verificationUri, verificationUriComplete: string\|null, interval, expiresIn}`; `503 {error: "notConfigured"}`; `400 {error: "badProvider"}` |
| `POST /v1/auth/device/poll` `{provider, deviceCode}`              | `200 {status: "pending"}` \| `{status: "slowDown", interval}` \| `{status: "expired"}` \| `{status: "denied"}` \| `{status: "complete", session}`                       |
| `POST /v1/auth/dev-token` `{name}` (only with `ASTRA_DEV_AUTH=1`) | `200 {session}`                                                                                                                                                         |
| `GET /v1/me` (bearer token)                                       | `200 {person, workspaceId}`; `401 {error: "unauthorized"}`                                                                                                              |

- `Session` = `{token, workspaceId, person: Person, expiresAt}`.
- `token` is an HS256 JWT (`sub` = personId, `ws` = workspaceId, 30-day expiry).
- On first sign-in the relay writes the person entity into the workspace op log.

**Sync authentication.** The sync `hello` frame carries the token. An invalid or expired token, or a
`workspaceId` that doesn't match the token's `ws` claim, gets `{t: "error", message: "unauthorized"}` and the
socket is closed with code 4401. Clients then sign in again.
