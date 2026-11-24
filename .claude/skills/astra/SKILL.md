---

name: astra
description: How to work in the Astra monorepo (Mindify AI's multi-agent development environment) — the React UI, design system, sync client, Rust core, Tauri shell and GCP Terraform. Use for any change in this repository: building or editing screens, changing the protocol or reducer, adding agent adapters or relay endpoints, touching tokens or styling, Tauri/mobile work, deployment, or deciding which tests to run before committing.
---

# Working on Astra

Astra lets people run coding agents from any vendor (Claude Code, Codex, Gemini CLI, LangGraph, custom
commands) on one repository, split or race them on a _mission_, steer and approve them with teammates, and
merge the result. Desktop, web, iOS and Android all run **one React UI**. **Runner nodes** (the desktop app, or
`astrad`) execute agents and keep keys; the **relay** (`astra-server`) only syncs ops. Everything is
local-first: clients reduce an op log into state, and sync it when they're connected.

Read before non-trivial work: `docs/ARCHITECTURE.md` (system), `docs/PROTOCOL.md` (the contract), and
`reference/design-system.md` (UI rules).

## Map

| Path                                            | What                                                                                            | Notes                                                             |
| ----------------------------------------------- | ----------------------------------------------------------------------------------------------- | ----------------------------------------------------------------- |
| `packages/tokens`                               | `tokens.json` → `src/generated/{theme.css,tokens.ts}`                                           | Never edit generated files; run `pnpm tokens`                     |
| `packages/ui`                                   | Design-system primitives (`@astra/ui`)                                                          | Presentational only: no client or router imports                  |
| `packages/client`                               | Protocol types, HLC/ULID, LWW reducer, `MockClient`/`TauriClient`/`RemoteClient`, actions, auth | `src/generated/protocol.ts` comes from Rust                       |
| `packages/app`                                  | Routes (`src/router.tsx`), screens, phone screens, context                                      | Talks only to `AstraClient`, never to a transport                 |
| `apps/web`                                      | SaaS entry, service worker, `scripts/screenshots.mjs`                                           | Demo workspace when `VITE_ASTRA_RELAY_URL` is unset               |
| `apps/desktop`                                  | Tauri 2 app (desktop + iOS + Android); `src-tauri` embeds `astra-node`                          | `src-tauri` is **outside** the Cargo workspace (needs webkit2gtk) |
| `crates/astra-protocol`                         | Rust protocol, reducer, HLC; `export-ts` bin                                                    | Mirrors `packages/client/src/types.ts`                            |
| `crates/astra-store` · `astra-sync`             | SQLite op log; sync client/hub                                                                  |                                                                   |
| `crates/astra-runtime` · `astra-llm`            | Agent adapters, worktrees, orchestration, policies; model providers                             |                                                                   |
| `crates/astra-node` · `astrad` · `astra-server` | Embeddable node; headless runner/LAN host; Axum relay                                           |                                                                   |
| `infra/gcp`                                     | Terraform: Cloud Run relay, Cloud SQL, CDN web, WIF deploys                                     | Has offline `terraform test`                                      |
| `fixtures/`                                     | `seed.json` (demo workspace), `ops-conformance.json`                                            | Shared by the Rust and TS tests                                   |

## Commands

```bash
pnpm install                      # once
pnpm dev                          # web on :5173, demo workspace
pnpm typecheck && pnpm test       # all TS packages (turbo)
pnpm lint                         # biome (format + lint); `pnpm format` / `npx biome check --write .` fixes
pnpm tokens                       # regenerate token CSS/TS after editing tokens.json
cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
cargo run -p astra-protocol --bin export-ts   # after changing Rust protocol types
pnpm e2e                          # whole stack, no mocks: browser → relay → astrad → git worktrees → merge
pnpm --filter @astra/desktop app:dev          # desktop app (needs the platform webview)
```

Screenshots: build and preview the web app, then run `node apps/web/scripts/screenshots.mjs <outDir>`. It
captures every route at 1440×960 (`?platform=mac`) and the phone screens at 390×844 (`?platform=ios|android`).
`?instant` makes the mock node respond with no delay. Use this to check UI changes against the design.

Terraform (in `infra/gcp`): `terraform fmt -check -recursive && terraform init -backend=false && terraform validate && terraform test`.

## Rules that are easy to break

1. **The protocol has two implementations.** A change to entities, ops, commands or reducer rules touches
   _all_ of these, in the same change:
   - `docs/PROTOCOL.md`;
   - `crates/astra-protocol`, then regenerate `packages/client/src/generated/protocol.ts`;
   - `packages/client/src/types.ts` and `reducer.ts`;
   - a case in `fixtures/ops-conformance.json`, which both test suites run.
     JSON is camelCase, enums are camelCase strings, optional values are explicit `null` (never omitted), and
     tagged unions use `type` (entities use `kind`).
2. **Who writes what** (`docs/PROTOCOL.md`):
   - The _client_ writes human messages and puts the mission, its agents and memberships.
   - The _runner node_ writes everything that results from a `command`: approval state, statuses, results,
     merges, agent messages.
   - `MockClient` (`packages/client/src/mock.ts`) simulates the node and must follow the same rules. The
     authorization rules live once in `packages/client/src/policy.ts` (`mayDecide`, `maySteer`). They mirror
     `crates/astra-runtime/src/policy.rs`, and the UI uses them through `select.ts`. Change all of these together.
   - The relay rejects ops whose `actor` isn't the signed-in person, or the connection's own node for non-command
     ops. Never stamp ops with another person's id.
3. **The UI goes through actions, not raw ops.** Use `packages/client/src/actions.ts` (`createMission`,
   `approve`, `sendMessage`, `merge`, …). Add an action there rather than calling `client.submit` with
   hand-built ops from a screen.
4. **State must be snapshot-stable.** Read state with `useAstraState()` and derive values with plain functions
   in `packages/app/src/select.ts`. Never pass a selector that builds new arrays to `useSyncExternalStore`; that
   causes an infinite render loop. The reducer copies only the collections it touches; keep it that way.
5. **Design fidelity.** Use only `@astra/ui` primitives and token classes; see `reference/design-system.md`.
   - No raw hex colours, no rounded corners except `rounded-sm`/`rounded-full`, no shadows except overlays.
   - Status is always a dot and a word.
   - Labels and buttons use the uppercase capitals voice; body copy is sentence case.
6. **Every screen has a desktop and a phone form.**
   - Register routes in `src/router.tsx`, using `<Adaptive desktop mobile/>` when a phone layout exists.
   - Desktop screens use `Page` + `AppHeader` with the right variant: `nav`, `repoSelect`, `step`, `actions`.
   - Phone screens use `MobileFrame`, `BrandBar`/`MissionTopBar` and `MobileTabs`.
   - iOS and Android differ on purpose: back affordance, title alignment, approve/deny order, segmented
     control vs. tabs, tab-bar style. Follow the existing components.
7. **Platform chrome.** macOS draws red, amber and green window controls on the left (`window-*` tokens) and uses the system menu bar.
   Windows, Linux and the web open the app menu from the Astra mark. Window actions and native capabilities
   (`pickFolder`, `copy`, `signOut`) arrive through `AstraProvider` props (`windowActions`, `host`). Check
   `useHost()` for the capability; never import Tauri APIs in `packages/app`.
8. **Offline is a feature.**
   - Fonts are self-hosted (no CDN links).
   - New network calls belong in `packages/client`, with failure handled.
   - The UI shows `OFFLINE` from `useConnectivity()`.
9. **Keys never leave runner nodes.** Never send secrets to the relay or the web app. Never store provider
   OAuth tokens: the relay uses them once to read the profile, and the browser only ever holds Astra's JWT.
10. **Relay limits.** Fan-out is in-process, so `relay_max_instances` stays at 1 until NATS lands. The relay
    must keep the runtime contract the Terraform expects:
    - `PORT`, `DATABASE_URL`, `ASTRA_JWT_SECRET`, `ASTRA_ALLOWED_ORIGINS`, `ASTRA_LOG_FORMAT=json`;
    - `/healthz` and `/readyz`;
    - a WebSocket ping every 25 s, and a graceful `SIGTERM`.
11. **Never put model identifiers in commits, PR text or code comments.** A config default in code is fine.

## Recipes

**New screen.**

1. Build it in `packages/app/src/screens/X.tsx` from `@astra/ui` primitives.
2. Add the route in `router.tsx`, plus a phone variant in `mobile/` if needed.
3. Add a test in `packages/app/test/` that mounts `<AstraApp client={new MockClient({ seed: demoSeed, latency: 0 })}
   platform=… history={{ memory: '/path' }} />`.
4. Screenshot it.

**New command.**

1. Add it to the protocol (rule 1).
2. Handle it in `MockClient` and in the `astra-runtime` orchestrator. Effects are ops plus a `commandResult`,
   and it runs exactly once (deduplicated by op id).
3. Add an action in `actions.ts`, and tests on both sides.

**New agent adapter.** Implement the runtime's adapter trait in `crates/astra-runtime/src/adapters/`. Map
permission requests to approvals, confine file and terminal access to the worktree, and emit log lines as
`{verb, target}`. Then add a seed or connect row (an `agent` entity with `category` and `transport`) so it shows
on the Connect screen.

**New relay endpoint.** Document it in `docs/PROTOCOL.md`. Implement it in `crates/astra-server` with a test
against the in-memory backend. If the web app calls it, add the client in `packages/client` behind an interface
that has a mock.

**Infra change.** Edit `infra/gcp/*.tf`. Add or adjust an assertion in `tests/plan.tftest.hcl`, and run the
Terraform checks.

## Before committing

Run `pnpm lint && pnpm typecheck && pnpm test` for TS changes, and the cargo trio for Rust. Run `pnpm e2e` for
changes that touch sync, the relay, the runtime or the hosted web flow. For UI changes, take
screenshots and look at them. Commit on the working branch with a descriptive message. Don't edit
`src/generated/**` by hand.
