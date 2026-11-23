# Astra architecture

Astra is a multi-agent development environment by Mindify AI. You connect coding agents from any vendor or
harness to a repository, give them one goal (a *mission*), split the work or race them, and steer and review
them with your team. It runs on:

- the desktop (macOS, Windows, Linux),
- the web (SaaS),
- iOS and Android.

It works remotely or fully locally, with or without internet.

## Principles

1. **Local first.** Every client reads from a local replica, so the UI never waits on the network. Changes are
   ops in an append-only log, and they sync when a connection exists.
2. **Agents run where you put them.** Only *runner nodes* execute agents: the desktop app, or `astrad` on your own
   server. API keys live in that machine's keychain. The SaaS backend never holds keys and never runs agents.
3. **One UI, every platform.** One React app is mounted by the Tauri shell (desktop, iOS, Android) and by the
   web build. Layouts adapt per platform (title bars, phone chrome), but screens are shared.
4. **One protocol.** The same op format and sync frames are used between every pair of nodes: phone ↔ relay,
   desktop ↔ relay, phone ↔ desktop on the LAN. See [PROTOCOL.md](PROTOCOL.md).

## System

```
 iOS / Android (Tauri)  ──┐                         ┌── Web SaaS (PWA)
   Rust core: store+sync   │   WebSocket · MsgPack   │     RemoteClient + IndexedDB
                           ├────►  astra-server  ◄───┤
 LAN (mDNS + QR pairing) ──┤    relay · identity    │     Postgres op log · NATS fan-out
                           ▼    presence · routing  ▼
 Desktop (Tauri)  /  astrad  =  RUNNER NODE
   store + sync + agent runtime + git worktrees + OS keychain
   ├─ ACP agents (Claude Code, Codex, Gemini CLI, …) over stdio JSON-RPC
   ├─ LangGraph (HTTP runs/stream) · custom command · MCP tools
   └─ Astra agent: Claude when online ⇄ Ollama / LM Studio / llama.cpp when offline
```

| Mode | What runs where | Internet needed |
|---|---|---|
| **Local** | The desktop alone: agents, git, data. No account. | No (with local models) |
| **Connected** | Desktop runner + relay, so phone, web and teammates join. | For remote clients |
| **Your server** | `astrad` on a box you own runs agents; any client steers. | Between clients and the server |
| **LAN** | Phone pairs directly with the desktop by QR code. | No |

## Repository

| Path | What |
|---|---|
| `packages/tokens` | `tokens.json` (Mindify AI Frontier) → CSS variables, Tailwind v4 theme, TS constants. Self-hosted fonts. |
| `packages/ui` | Design-system primitives: buttons, status, avatars, log well, diff view, window controls, phone chrome. |
| `packages/client` | Protocol types, HLC/ULID, per-field LWW reducer, and the `AstraClient` implementations (Mock, Tauri, Remote). |
| `packages/app` | Routes and screens: every artboard, desktop and phone. Depends only on `AstraClient`. |
| `apps/web` | SaaS build: Vite, service-worker app shell, Playwright screenshot script. |
| `apps/desktop` | Tauri 2 app. The same project builds desktop, iOS and Android; `src-tauri` embeds `astra-node`. |
| `crates/astra-protocol` | Rust protocol types, reducer and HLC; exports TS types to `packages/client/src/generated`. |
| `crates/astra-store` | SQLite (WAL) op log and projections. |
| `crates/astra-sync` | Transport-agnostic sync engine: client state machine and server hub. |
| `crates/astra-llm` | Model providers (Anthropic Messages API, OpenAI-compatible local servers) and the tool loop. |
| `crates/astra-runtime` | Agent supervisor, adapters, worktrees, split/race orchestration, approvals and policies. |
| `crates/astra-node` | Embeddable node used by the Tauri shell and `astrad`. |
| `crates/astrad` | Headless runner and LAN host. |
| `crates/astra-server` | Axum SaaS backend: sync relay, auth, presence, node routing. |
| `fixtures/` | Seed workspace and the reducer conformance cases shared by Rust and TS. |

## Data and sync

- **Ops.** Each op is `{id (ULID), hlc, actor, body}`. Bodies:
  - `put`, `patch` and `delete` change entities;
  - `command` asks a runner node to do something;
  - `commandResult` records what happened.
- **Merging.** Scalar fields are last-writer-wins *per field* by hybrid logical clock. Threads and logs are
  append-only entities, so there are no merge conflicts. Rust and TypeScript implement the same reducer, and
  `fixtures/ops-conformance.json` keeps them identical.
- **Commands run exactly once.** Side effects (approve, steer, merge, start) are ops addressed to the owning node.
  The node deduplicates by op id and appends a `commandResult`. A command for an offline node waits in the log.
- **Offline.**
  - Native clients keep the op log in SQLite.
  - The web keeps it in IndexedDB, and the service worker caches the app shell.
  - Ops made offline are queued and pushed after reconnecting, and the cursor-based catch-up replays what was
    missed.
- **Presence** ("Lin is reading result B") is relayed but never stored.

## Agent runtime

- **Adapters.**
  - **ACP** (Agent Client Protocol: JSON-RPC over the agent's stdio) covers Claude Code, Codex and Gemini CLI. Its
    permission requests become Astra approvals, and its file and terminal calls are confined to the agent's
    worktree.
  - **LangGraph** over HTTP.
  - **Custom command**: prompt on stdin, text on stdout.
  - **Built-in Astra agent.**
- **Isolation.**
  - Each mission agent works in its own git worktree on `astra/<slug>`.
  - Race entrants get `-a`, `-b`, and so on.
  - Losing branches are kept after a merge.
- **Orchestration.**
  - *Split*: the planner writes a plan, then tasks go to builders, testers and reviewers.
  - *Race*: every builder attempts the goal in parallel, a reviewer compares the results, and a person merges one.
- **Policies.**
  - Per agent: *Ask me*, *Ask anyone who can steer*, or *Run without asking*.
  - Per mission: who may approve.
  - Commands outside the worktree always ask.
  - One person steers an agent at a time.
- **Built-in agent models.**
  - Online, it calls the Anthropic Messages API over raw HTTP, because there is no official Rust SDK. It uses
    `claude-opus-5-5` with adaptive thinking, streaming, prompt caching and server-side fallbacks.
  - Offline, it detects Ollama, LM Studio or llama.cpp.
  - Routing is *Auto*, *Cloud only* or *Local only*.
  - Agents that need the network show *Offline*, and their roles can be handed to the local agent.

## Performance

- UI reads come from a local in-memory replica. The reducer copies only touched collections, and long threads
  use `content-visibility`.
- The Rust core uses tokio and SQLite in WAL mode. Agent output is coalesced before it is emitted to the UI.
- Sync frames are MessagePack, and catch-up is incremental by cursor.
- Tauri bundles are small: the system webview, with no bundled Chromium.
- The web bundle is about 125 KB gzipped (app plus vendor), and fonts load per unicode range.

## Security

- **Keys.** Keys stay in the OS keychain on runner nodes, and nothing secret is synced.
- **Agent sandboxing.** Agent file access is confined to its worktree, and commands follow the approval policy.
- **Tauri.** The webview's capabilities are limited to window controls, the OS info read, the folder picker and
  clipboard writes. Files, processes and the network go through the Rust core.
- **Relay.** The relay authenticates with JWT (GitHub/GitLab OAuth device flow) and enforces workspace
  membership. It stores ops (messages, diffs, statuses), not repositories.
- **LAN pairing.** It uses a one-time code from a QR scan.
