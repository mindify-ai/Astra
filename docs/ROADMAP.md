# Roadmap

## Now (this first pass)

- All 18 design artboards implemented: desktop journey, Map, Board, People, per-OS title bars, and iOS/Android
  Home, Thread and Review.
- Local-first data: shared protocol, LWW reducer (Rust and TS, one conformance suite), SQLite store, sync engine.
- Runner node: worktrees, ACP / LangGraph / command / built-in adapters, split and race orchestration,
  approvals, steering and merge.
- Built-in agent: Claude online, local OpenAI-compatible model servers offline.
- Relay server: sync, presence, command routing, Postgres or in-memory storage, dev and device-flow auth.
- Web SaaS with an offline app shell. Tauri shell for desktop and mobile. CI plus a release matrix.

## Next

1. **Auth and onboarding.** Web sign-in through the GitHub/GitLab device flow is built (docs/PROTOCOL.md "Auth").
   Next: list repositories from the provider APIs instead of the seed, and sign the desktop app in to a relay.
2. **Push and PRs.** Push mission branches and open pull requests from Merge (GitHub/GitLab APIs) alongside the
   local merge.
3. **Mobile notifications.** APNs/FCM push for _Needs you_, routed by the relay.
4. **Relay scale-out.** NATS fan-out between relay instances, per-workspace partitions, and op-log compaction
   into snapshots.
5. **End-to-end encryption (optional).** Encrypt op bodies per workspace so the relay only routes ciphertext;
   server-side search then moves to clients.
6. **More adapters.** A2A agent cards, CrewAI, OpenAI Agents SDK, Cursor CLI, OpenCode.
7. **Bundled local models (optional).** A llama.cpp sidecar with in-app model download, for machines with no
   local server.
8. **Thread virtualization.** Windowed rendering for missions with thousands of events.
9. **Settings UI.** Relay URL, keys (stored in the keychain), local model choice, theme (light tokens already exist).
10. **Localisation.** The type stack already falls back to Noto Sans TC / PingFang TC; strings move to catalogs.

## Store readiness

| Platform | Needs                                                                |
| -------- | -------------------------------------------------------------------- |
| macOS    | Developer ID signing + notarization (secrets wired in `release.yml`) |
| Windows  | Authenticode certificate                                             |
| Linux    | Done (deb, rpm, AppImage)                                            |
| iOS      | Apple team id, provisioning, App Store review                        |
| Android  | Release keystore, Play listing                                       |
