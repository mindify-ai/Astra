# Astra protocol

The contract shared by the Rust core (`crates/astra-protocol`) and the TypeScript client
(`packages/client`). JSON field names are **camelCase**, enum values are **camelCase strings**, and tagged
unions use a `type` field (entities use `kind`). Optional values are `null`, never omitted.

## Entities

Every entity has `kind` and `id`. Clients hold a normalized `State` with one map per kind.

| kind           | fields                                                                                                                                                                                                                                                                                                                                                                   |
| -------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `person`       | `id, name, initials`                                                                                                                                                                                                                                                                                                                                                     |
| `repository`   | `id, name, source: github\|gitlab\|local, defaultBranch, path: string\|null, remoteUrl: string\|null`                                                                                                                                                                                                                                                                    |
| `agent`        | `id, name, vendor, category: codingAgent\|harness\|protocol\|local, transport, adapter: Adapter\|null, runsOn: local\|server, nodeId: string\|null, commandPolicy: askMe\|askSteerers\|auto, status: connected\|settingUp\|notConnected\|offline, requiresNetwork: bool`                                                                                                 |
| `mission`      | `id, number, title, shortName, goal, mode: split\|race, repoId, baseBranch, branch, status: draft\|working\|needsYou\|done\|merged, approvalPolicy: owner\|steerers, ownerId, nodeId: string\|null, stats: DiffStats\|null, createdAt, archivedAt: string\|null`                                                                                                         |
| `missionAgent` | `id, missionId, agentId, role: planner\|builder\|tester\|reviewer\|writer, enabled: bool, status: idle\|queued\|working\|needsYou\|done\|offline, statusDetail: string\|null, steeredBy: string\|null, task: string\|null, startedAt: string\|null, branch: string\|null, latest: LogLine[], agentTypeId: string\|null` (the custom agent type it was added as)          |
| `task`         | `id, missionId, title, role, missionAgentId: string\|null, column: queued\|working\|needsYou\|done, note: string\|null, mergedBy: string\|null, approvalId: string\|null, order: number`                                                                                                                                                                                 |
| `message`      | `id, missionId, author: Author, steering: string\|null, to: string\|null, body, plan: PlanStep[], log: LogLine[], approvalId: string\|null, at`                                                                                                                                                                                                                          |
| `approval`     | `id, missionId, missionAgentId, prompt, command: string\|null, state: pending\|approved\|denied, decidedBy: string\|null, decidedAt: string\|null`                                                                                                                                                                                                                       |
| `result`       | `id, missionId, label, missionAgentId, branch, stats: DiffStats, checks: passed\|failed\|pending\|none, reviewerPick: bool, diff: DiffFile[]`                                                                                                                                                                                                                            |
| `merge`        | `id, missionId, resultId: string\|null, into, stats: DiffStats, checks, approvedBy, note: string\|null, keptBranches: string[], at`                                                                                                                                                                                                                                      |
| `membership`   | `id` (= `"<missionId>:<personId>"`), `missionId, personId, access: owner\|steer\|watch`                                                                                                                                                                                                                                                                                  |
| `agentType`    | `id, name, role, agentId, instructions`: a custom agent type, a base role on a default agent (vendor or harness) with its own instructions. Clients put and delete them; a runner node adds `instructions` to the brief of every mission agent whose `agentTypeId` names it.                                                                                             |
| `node`         | `id, name, ownerId, platform, startedAt`: a runner node (the desktop app or `astrad`). Each runner node puts its own entry, as its own actor, every time it starts with the runtime on. `name` is the host name unless the node was given one (`astrad --name`); `ownerId` is the person it runs as. Clients list these as the places an agent can run (`agent.nodeId`). |

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
approvals, results, merges, memberships, agentTypes, nodes, presence }`. Each entity map is `Record<id, Entity>`.
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
| `delete`        | `entityKind, id`                     | Tombstone (see visibility below).                                                                       |
| `command`       | `node, command`                      | A side effect addressed to runner node `node`. Executed **exactly once** by that node (dedup by op id). |
| `commandResult` | `commandOp, ok, error: string\|null` | Appended by the node after it executes a command.                                                       |

**Visibility after deletes.** A deleted entity keeps its last data and field clocks, hidden. Later ops still
update those hidden fields by the same last-writer-wins rule. An entity is visible when it has no tombstone, or
when its newest field clock is newer than the tombstone. So a write that is concurrent with a delete but newer
than it revives the entity, whatever order the two arrive in. A put older than a tombstone is recorded hidden. The
Rust reducer also ignores a write that would make the entity invalid. Both reducers are checked against every
causal arrival order of `fixtures/ops-conformance.json`.

`Command` (tagged by `type`):

- `startMission{missionId}`
- `approve{approvalId}`, `deny{approvalId}`
- `sendMessage{missionId, to: string|null, body}`
- `takeSteering{missionAgentId}`, `releaseSteering{missionAgentId}`
- `pause{missionAgentId}`, `resume{missionAgentId}`
- `merge{resultId}`
- `connectAgent{agentId}`, `testAgent{agentId}`
- `switchAgent{missionAgentId, agentId}`: run that role on another connected agent (vendor or harness)
- `addAgent{missionAgentId}`, `removeAgent{missionAgentId}`: add a role to a mission or take one off
- `archiveMission{missionId, archived: bool}`, `deleteMission{missionId}`: owner only

A client creates a mission by issuing `put` ops for the mission, its missionAgents and its memberships, then a
`startMission` command. A mission runs on one runner node (`mission.nodeId`): by default, the node its first
enabled agent runs on.

**Attaching an agent to a runner node.** `agent.nodeId` says which runner node runs an agent, and every command
about the agent (`connectAgent`, `testAgent`) is addressed to that node. To move an agent to another node (say
`astrad` on a server), a client puts the agent with the new `nodeId` and sends `connectAgent` to that node.
`runsOn` is `local` when the node is the client's own (the desktop app) and `server` otherwise.

### Rules for runner nodes

- **Authorization.**
  - Approvals and merges may be decided by the mission owner. Members with `steer` access may also decide them
    when the mission's `approvalPolicy` is `steerers`, or when the agent's `commandPolicy` is `askSteerers`.
  - `sendMessage`, `takeSteering`, `switchAgent`, `addAgent` and `removeAgent` need owner or steer access.
  - `sendMessage` with `to` set also needs the agent's steering lock; the node takes it implicitly when it is free.
  - The same rules live in `@astra/client` `policy.ts`, which the UI and the mock node use.
- **Identity.** The relay checks every pushed op against the signed-in person. A `command` must be authored by
  that person; any other op by that person or by the connection's own node id (so a runner node writes its
  `node` entity as itself). Presence is rewritten to that person. Runner nodes sign in to the relay with a person's session; there is no separate node identity yet.
- **IDs the node creates:**
  - approvals: `ap-<ulid>`
  - messages: `msg-<ulid>`
  - tasks: `t-<ulid>`
  - results: `res-<missionId>` (split) or `res-<missionId>-<a|b…>` (race)
  - merge: the merge id is the mission id
- **Failures.** A mission that stops on an error goes to `needsYou`, with an agent message
  `The mission stopped: …`.
- **Agent ↔ approval mapping.** An agent permission request becomes an `approval` put plus a missionAgent patch
  `status: needsYou`. The decision resumes or cancels the agent.

### Who writes what

| command            | client writes                                                                                                                                                                                                       | node writes                                                                                                                                                                                                                                                                                                                                                                                                                                     |
| ------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `sendMessage`      | The `message` put, `author.type = person`                                                                                                                                                                           | Delivers `body` to the target agent, or to every enabled agent when `to` is null; writes no duplicate message. If `to` is set and free, patches `steeredBy = actor`; if someone else holds it, rejects.                                                                                                                                                                                                                                         |
| `approve` / `deny` | Only the command                                                                                                                                                                                                    | Patches the approval (`state, decidedBy, decidedAt`) and the missionAgent status (`working`/`idle`). The linked task goes to `working` on approve, or to `done` with note `Denied by <actor>` on deny. Then the node resumes or cancels the agent.                                                                                                                                                                                              |
| `merge`            | Only the command                                                                                                                                                                                                    | Writes the `merge` entity (`id = missionId`), patches mission `status = merged`, and patches the linked `needsYou` task to `done` with `mergedBy`.                                                                                                                                                                                                                                                                                              |
| `startMission`     | Mission put (`status = draft`, `number = max + 1`), missionAgents (`queued`), memberships                                                                                                                           | Patches mission `status = working`, then writes tasks/messages as agents run. Agent messages use `author.type = agent`; plans go in `plan`, edits and commands in `log` (`verb` = edit\|run\|read). `missionAgent.latest` keeps the last 3 log lines.                                                                                                                                                                                           |
| `switchAgent`      | Only the command                                                                                                                                                                                                    | Rejects an unknown agent, one without an adapter, one that isn't `connected`, or a merged mission. Patches the missionAgent `agentId` and `statusDetail = switched from <old name>`. A running session is cancelled; if the agent was mid-task the task is handed to the new agent in the same worktree, so the mission carries on. An agent that has not started yet just starts on the new one.                                               |
| `addAgent`         | The missionAgent put (`status = queued`, `enabled = true`)                                                                                                                                                          | Nothing more for a `draft` mission: `startMission` picks it up. On a split mission under way, starts it in the mission worktree with its role's brief. Rejects races that have started, merged missions, and agents that aren't connected.                                                                                                                                                                                                      |
| `removeAgent`      | Only the command                                                                                                                                                                                                    | Patches the missionAgent `enabled = false, status = done, statusDetail = removed, steeredBy = null` and stops its session; a task it was on closes with note `Removed from the mission` and the mission carries on. The entity stays, so its messages keep their author. Rejects removing the last enabled agent, or one with a pending approval.                                                                                               |
| `archiveMission`   | Only the command                                                                                                                                                                                                    | Owner only. Archiving patches `archivedAt = now` (and `status = needsYou` if it was `working`), denies pending approvals, stops running agents (`status = idle, statusDetail = stopped: archived`) and stops the mission's run without a failure message. An archived mission rejects `startMission`, `sendMessage`, `switchAgent`, `addAgent` and `removeAgent`. `archived = false` patches `archivedAt = null`; agents restart when messaged. |
| `connectAgent`     | The whole agent as a `put` (its setup, `nodeId`, `status = settingUp`), not a patch: a catalog agent may exist only in this client's store, and a patch of an entity the relay never saw is ignored everywhere else | Checks it can reach the agent with its adapter, then patches `status` (`connected` or `notConnected`) and, when connected, `nodeId` = itself.                                                                                                                                                                                                                                                                                                   |
| `deleteMission`    | Only the command                                                                                                                                                                                                    | Owner only. Stops everything as `archiveMission` does, removes the mission's worktrees (its branches stay in the repository), then deletes the mission and its missionAgents, tasks, messages, approvals, results, merge and memberships.                                                                                                                                                                                                       |

## Sync wire protocol

WebSocket at `/v1/sync`. Frames are MessagePack by default; JSON with `?format=json`. Frames are tagged by `t`.

| direction       | frames                                                                                                                  |
| --------------- | ----------------------------------------------------------------------------------------------------------------------- |
| client → server | `{t:"hello", workspaceId, nodeId, cursor, token}`, `{t:"push", ops}`, `{t:"presence", presence}`                        |
| server → client | `{t:"welcome", cursor}`, `{t:"ops", ops, cursor}`, `{t:"ack", ids}`, `{t:"presence", presence}`, `{t:"error", message}` |

- `cursor` is the server's per-workspace sequence number.
- On `hello` the server sends `welcome{cursor: head}`, then replays every op after the client's cursor in
  `ops` frames. Each frame's `cursor` is the sequence number of its last op.
- Pushed ops are deduplicated by `id`, echoed to the pusher as `ops`, then acknowledged with `ack`.
- A runner node receives `command` ops addressed to it; commands for a node that is offline stay in the log
  until it reconnects.
- `presence` frames carry `Presence[]` both ways. Client → server replaces that connection's entries; server →
  client is the whole workspace.
- A node's local cursor (`snapshot().cursor`, `astra://ops`) is its local op-log sequence, not the relay's.
- **Scaling limit.** Live fan-out reaches only sockets on the same relay instance; sequence numbers stay correct
  across instances. Run one relay instance until NATS or Postgres `LISTEN/NOTIFY` fan-out lands.

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

**Desktop sign-in.** The desktop app runs the same device flow from its webview, so the relay always allows the
Tauri origins (`tauri://localhost`, `http://tauri.localhost`, `https://tauri.localhost`) besides
`ASTRA_ALLOWED_ORIGINS`. The shell then keeps the session (the Astra token only) in its local settings and
restarts its node on the session's workspace. That workspace gets its own store, under `workspaces/<workspaceId>`
in the app data directory, so the local-only workspace's unsynced ops are never pushed to the relay. Signing out
returns to the local-only store.

**Sync authentication.** The sync `hello` frame carries the token. An invalid or expired token, or a
`workspaceId` that doesn't match the token's `ws` claim, gets `{t: "error", message: "unauthorized"}` and the
socket is closed with code 4401. Clients then sign in again.
