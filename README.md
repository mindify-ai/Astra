# Astra

**Every agent. One workspace.** A multi-agent development environment by Mindify AI.

Connect coding agents from any vendor or harness (Claude Code, Codex, Gemini CLI, LangGraph, or your own command)
to a repository. Give them one goal, split the work or race them, steer them with your team, approve what they
run, and merge the result.

- **Platforms:** desktop (macOS, Windows, Linux), web, iOS and Android, from one React UI.
- **Runner nodes:** agents run on your machine or your server, and keys never leave it.
- **Local first:** works offline with local models (Ollama, LM Studio, llama.cpp), and syncs when connected.

See [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) for the design, [docs/PROTOCOL.md](docs/PROTOCOL.md) for the sync
contract and [docs/ROADMAP.md](docs/ROADMAP.md) for what's next.

## Quick start

Requirements: Node 22+, pnpm 10, Rust 1.88+. The desktop shell also needs the platform webview: on Linux,
`libwebkit2gtk-4.1-dev libgtk-3-dev librsvg2-dev`.

```bash
pnpm install
```

The web app runs on an in-browser demo workspace when no relay is configured:

```bash
pnpm dev
```

To run the desktop app (a local runner node with SQLite, agents and git):

```bash
pnpm --filter @astra/desktop app:dev
```

To run the tests:

```bash
pnpm test && cargo test --workspace
```

Append `?platform=mac|windows|linux|ios|android` to a web URL to preview another platform's chrome.

### Web sign-in

The web app signs in with GitHub (or GitLab) through the relay's OAuth device flow. It shows a code, you approve
it on GitHub, and no password or GitHub token ever reaches the browser.

| Variable (build time)   | Effect                                                                                             |
| ----------------------- | -------------------------------------------------------------------------------------------------- |
| `VITE_ASTRA_RELAY_URL`  | Relay base URL, e.g. `https://sync.example.com`. Unset: the in-browser demo workspace, no sign-in. |
| `VITE_ASTRA_DEV_AUTH=1` | Also shows a developer sign-in form (relay started with `ASTRA_DEV_AUTH=1`).                       |

- The relay needs `GITHUB_CLIENT_ID` from a GitHub OAuth app with **device flow enabled**. With Terraform this is
  `github_oauth_client_id`.
- Add `?signin` to the demo URL to try the sign-in screen without a relay.

## Layout

| Path                                        | What                                                                                           |
| ------------------------------------------- | ---------------------------------------------------------------------------------------------- |
| `apps/web`                                  | Web SaaS (Vite, offline app shell)                                                             |
| `apps/desktop`                              | Tauri 2 app: desktop, iOS and Android                                                          |
| `packages/tokens` · `ui` · `client` · `app` | Design tokens, UI primitives, sync client, screens                                             |
| `crates/*`                                  | Rust core: protocol, store, sync, runtime, LLM providers, node, `astrad`, `astra-server`       |
| `infra/gcp`                                 | Terraform for Google Cloud: Cloud Run relay, Cloud SQL, CDN-hosted web app, keyless CI deploys |
| `fixtures/`                                 | Demo workspace and reducer conformance cases shared by Rust and TypeScript                     |

## Phones

The Tauri project generates native iOS and Android projects (needs Xcode or the Android SDK/NDK):

```bash
pnpm --filter @astra/desktop ios:init
```

```bash
pnpm --filter @astra/desktop android:init
```

Release builds for every platform run in `.github/workflows/release.yml`.
