# Astra architecture

Astra is a multi-agent development environment by Mindify AI. You connect coding agents from any vendor or
harness to a repository, give them one goal (a _mission_), split the work or race them, and steer and review
them with your team. It runs on:

- the desktop (macOS, Windows, Linux),
- the web (SaaS),
- iOS and Android.

It works remotely or fully locally, with or without internet.

## Principles

1. **Local first.** Every client reads from a local replica, so the UI never waits on the network. Changes are
   ops in an append-only log, and they sync when a connection exists.
2. **Agents run where you put them.** Only _runner nodes_ execute agents: the desktop app, or `astrad` on your own
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

| Mode            | What runs where                                           | Internet needed                |
| --------------- | --------------------------------------------------------- | ------------------------------ |
| **Local**       | The desktop alone: agents, git, data. No account.         | No (with local models)         |
| **Connected**   | Desktop runner + relay, so phone, web and teammates join. | For remote clients             |
| **Your server** | `astrad` on a box you own runs agents; any client steers. | Between clients and the server |
| **LAN**         | Phone pairs directly with the desktop by QR code.         | No                             |

**Attaching a runner node.** Every runner node puts a `node` entity when it starts. On the Connect screen, _Runs
on_ lists them (this machine first), and picking one attaches the agent there: its commands go to that node, which
connects the agent with its own keys. A mission runs on the node of its first enabled agent. To add a server, run
`astrad` there signed in to the workspace's relay (see the README).

**Desktop sign-in.** _Continue with GitHub_ on Welcome (or _Sign in to sync_) has two routes:

- **No sync server** (the default): the app runs GitHub's device flow itself, with its built-in OAuth client id
  (public; no client secret is used or shipped, which rules out GitHub's redirect flow here). It opens GitHub with
  the code copied and brings the window back to the front once you approve. You take your GitHub name in the local workspace, and GitHub's
  token goes to the OS keychain so the Repo screen can list your repositories and clone one into `~/Astra` in one
  click. The token is passed to `git` through environment config only, never written to the clone or synced.
- **A sync server address**: the relay's redirect flow. GitHub opens in the default browser; the relay sends the
  browser back to a loopback port the Rust shell listens on (RFC 8252), which hands the one-time code to the
  webview and brings the window to the front. The webview trades it, with its PKCE verifier, for a session. The
  node then restarts on that workspace in its own store. A relay without a client secret falls back to its device
  flow.

## Repository

| Path                    | What                                                                                                          |
| ----------------------- | ------------------------------------------------------------------------------------------------------------- |
| `packages/tokens`       | `tokens.json` (Mindify AI Frontier) → CSS variables, Tailwind v4 theme, TS constants. Self-hosted fonts.      |
| `packages/ui`           | Design-system primitives: buttons, status, avatars, log well, diff view, window controls, phone chrome.       |
| `packages/client`       | Protocol types, HLC/ULID, per-field LWW reducer, and the `AstraClient` implementations (Mock, Tauri, Remote). |
| `packages/app`          | Routes and screens: every artboard, desktop and phone. Depends only on `AstraClient`.                         |
| `apps/web`              | SaaS build: Vite, service-worker app shell, Playwright screenshot script.                                     |
| `apps/desktop`          | Tauri 2 app. The same project builds desktop, iOS and Android; `src-tauri` embeds `astra-node`.               |
| `crates/astra-protocol` | Rust protocol types, reducer and HLC; exports TS types to `packages/client/src/generated`.                    |
| `crates/astra-store`    | SQLite (WAL) op log and projections.                                                                          |
| `crates/astra-sync`     | Transport-agnostic sync engine: client state machine and server hub.                                          |
| `crates/astra-llm`      | Model providers (Anthropic Messages API, OpenAI-compatible local servers) and the tool loop.                  |
| `crates/astra-runtime`  | Agent supervisor, adapters, worktrees, split/race orchestration, approvals and policies.                      |
| `crates/astra-node`     | Embeddable node used by the Tauri shell and `astrad`.                                                         |
| `crates/astrad`         | Headless runner and LAN host.                                                                                 |
| `crates/astra-server`   | Axum SaaS backend: sync relay, auth, presence, node routing.                                                  |
| `fixtures/`             | Seed workspace and the reducer conformance cases shared by Rust and TS.                                       |

## Data and sync

- **Ops.** Each op is `{id (ULID), hlc, actor, body}`. Bodies:
  - `put`, `patch` and `delete` change entities;
  - `command` asks a runner node to do something;
  - `commandResult` records what happened.
- **Merging.** Scalar fields are last-writer-wins _per field_ by hybrid logical clock. Threads and logs are
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
  - _Split_: the planner writes a plan, then tasks go to builders, testers and reviewers.
  - _Race_: every builder attempts the goal in parallel, a reviewer compares the results, and a person merges one.
- **Policies.**
  - Per agent: _Ask me_, _Ask anyone who can steer_, or _Run without asking_.
  - Per mission: who may approve.
  - Commands outside the worktree always ask.
  - One person steers an agent at a time.
- **Built-in agent models.**
  - Online, it calls the Anthropic Messages API over raw HTTP, because there is no official Rust SDK. It uses
    `claude-opus-5-5` with adaptive thinking, streaming, prompt caching and server-side fallbacks.
  - Offline, it detects Ollama, LM Studio or llama.cpp.
  - Routing is _Auto_, _Cloud only_ or _Local only_.
  - Agents that need the network show _Offline_, and their roles can be handed to the local agent.

## Performance

- UI reads come from a local in-memory replica. The reducer copies only touched collections, and long threads
  use `content-visibility`.
- The Rust core uses tokio and SQLite in WAL mode. Agent output is coalesced before it is emitted to the UI.
- Sync frames are MessagePack, and catch-up is incremental by cursor.
- Tauri bundles are small: the system webview, with no bundled Chromium.
- The web bundle is about 125 KB gzipped (app plus vendor), and fonts load per unicode range.

## Security

- **Keys.** Keys stay in the OS keychain on runner nodes, and nothing secret is synced. That includes the desktop
  app's GitHub token (scopes `repo read:user`), used only to list and clone your repositories.
- **Agent sandboxing.** Agent file access is confined to its worktree, and commands follow the approval policy.
- **Tauri.** The webview's capabilities are limited to window controls, the OS info read, the folder picker,
  clipboard writes, bringing its window to the front, and opening `https://github.com/*` pages in the default browser (for sign-in). Files, processes and the network go through the Rust core.
- **In-app browser.** The workbench's browser pane and the floating mini browser load pages in a sandboxed
  iframe that can never navigate or read the app. A page from the app's own origin never keeps that origin, so it
  can't reach the app's storage or session. The **isolated sandbox** (the `ISOLATED` toggle, kept per device)
  takes the origin away from every page: an opaque origin with no cookies, storage or service workers, and no
  pop-ups, dialogs or downloads. On Chromium engines the frame is also `credentialless`, so the page loads without
  the cookies its site already has. Pages that need their own origin (most sign-ins, dev servers that check
  `Origin`) need it off.
- **Relay.** The relay authenticates with JWT (GitHub OAuth redirect flow, or the device flow without a client secret, from the web or the desktop app; the relay also speaks GitLab's device flow, which the apps don't offer for now). It keeps the OAuth client secret, sends browsers back only to allowed origins or loopback, and hands out a one-time code that only the app holding the PKCE verifier can trade in
  and enforces workspace membership. It stores ops (messages, diffs, statuses), not repositories.
- **LAN pairing.** It uses a one-time code from a QR scan.
