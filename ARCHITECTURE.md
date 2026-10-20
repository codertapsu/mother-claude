# Architecture

Authoritative invariants also live in [CLAUDE.md](CLAUDE.md). The security model
is in [SECURITY.md](SECURITY.md), the phone and remote-access guide in
[docs/REMOTE.md](docs/REMOTE.md), the HTTP bridge in [docs/BRIDGE.md](docs/BRIDGE.md).

## Layers

```
 ┌──────────────────────────┐   HTTP + WebSocket v2    ┌──────────────────────────┐
 │ Angular SPA (src/)        │ ───────────────────────▶ │ axum server              │
 │  desktop webview + phone  │ ◀─────────────────────── │ (src-tauri/src/server)   │
 └──────────────────────────┘    one shared path        └────────────┬─────────────┘
                                                                     │ AppState, broadcast bus,
                                                                     │ pending store
                  ┌──────────────────────────────┬───────────────────┼──────────────────────┐
                  ▼                              ▼                   ▼                      ▼
     ┌────────────────────────┐   ┌──────────────────────┐ ┌──────────────────┐ ┌──────────────────┐
     │ claude/ adapter         │   │ owned control         │ │ foreign control   │ │ HTTP bridge       │
     │ ~/.claude, pidfiles,    │   │ claude -p stream-json │ │ hooks plugin,     │ │ bridge/ + Node    │
     │ transcripts, agents     │   │ (Path B)              │ │ tmux, PTY (bg)    │ │ bridge-host       │
     └────────────────────────┘   └──────────────────────┘ └──────────────────┘ └──────────────────┘
```

1. **Rust core** (`src-tauri/src/claude/`) — the *only* code that knows the
   `~/.claude` layout and formats:
   - `home` (paths, encoded cwd), `schema` (tolerant serde), `ids` (validators
     for every id that reaches a path or argv);
   - `live` — the live roster from `sessions/<pid>.json` and
     `jobs/<short>/state.json`, liveness by `kill(pid, 0)` + process start time;
   - `index` — the incremental transcript index (one byte-offset cursor per main
     and subagent file, aggregates for usage, model, mode, todos, queue,
     background tasks; checkpointed to `index-cache.json`);
   - `agents` — subagents, workflow runs and background shells; `activity` —
     "what is it doing right now"; `pricing` — cost and context window;
     `uievents` — transcript records normalised into `UiEvent`s, read backwards
     in pages; `summaries` — the `SessionSummary` / `SessionDetail` contract;
   - `control` + `owned` — owned sessions (Path B, and Path A behind a flag);
     `caps` — what each session supports (`ControlCaps`); `hooks` — the
     foreign-control plugin's files and `claude plugin` install/uninstall;
     `tmux` — typing into panes; `experimental` — PTY `claude attach` for
     background jobs;
   - `git` (libgit2 diffs, log, worktrees), `settings`, `auth` (Claude's own
     sign-in), `watcher` (debounced `notify`);
   - `accounts` — the Claude accounts sessions run under (the Default, directory
     and token accounts; `accounts.json`), the child environment each one
     composes, per-account sign-in, the `setup-token` capture and the accounts
     view; `secrets` — tokens and the backup passphrase in the Keychain;
     `transfer` — moving and copying sessions and projects between accounts'
     directories, with an undo; `backup` — encrypted per-session backups to a
     folder, restore and pull ([docs/ACCOUNTS.md](docs/ACCOUNTS.md),
     [docs/BACKUP.md](docs/BACKUP.md)). The roster and the index cover every
     account's directory at once.
2. **Embedded axum server** (`src-tauri/src/server/`) — REST under `/api`,
   WebSocket v2 at `/ws`, the hook endpoints and the `/v1` bridge mount:
   - `context` (who is asking: ingress, `machine-local` / `local`, principal) and
     `auth` (credentials, guard extractors), `security` (headers, CORS, route
     policy, audit), `ratelimit`, `error`;
   - `routes/` — `data` (sessions, transcripts, agents, diffs, inbox),
     `control` (the dispatchers that route each action to the owned / hooks /
     tmux / PTY / bridge channel), `foreign`, `pairing`, `remote`, `push`,
     `uploads`, `accounts`, `transfers` (data moves and data jobs), `backup`;
   - `monitor` (the refresh pipeline), `ws` (the hub, per-client queues and
     subscriptions), `hooks` (the plugin's dispatcher), `devices`, `pairing`,
     `remote` (tunnel listener and detection), `listeners` + `tls` (LAN TLS),
     `push` (Web Push).
   **All** dashboard data flows through here so desktop and mobile share one
   client. Tauri `invoke` is only for desktop OS concerns (`permissions.rs`,
   `server_info`).
3. **Angular SPA** (`src/`) — standalone components and signals. `core/`
   (`ApiService` = fetch, `RealtimeService` = WS v2 with a polling fallback,
   `TranscriptStore`, `PushService`) and `pages/` (sessions, inbox, agents,
   session detail, new session, remote, pair, settings, API).
4. **Claude HTTP bridge** (`src-tauri/src/bridge/` + `sidecar/src/bridge-host.ts`)
   — see below.

Shared pieces: `state.rs` (`AppState`, one field per subsystem), `pending.rs`
(the pending-decision store every channel uses) and `persist.rs` (atomic 0600
writes to the app data dir, never under `~/.claude`).

## Data flow

```
~/.claude/sessions, jobs, projects  ──▶ watcher (debounced) ─┐
                                     2 s tick, 60 s sweep ──┼──▶ monitor ──▶ live roster + index + agents
hook events, owned stream, pending ─────────────────────────┘                  │
                                                                       summaries (content-hash diff)
                                                                                 ▼
                                               ws hub: sessions / transcript / agents / fleet / pending
                                                                                 ▼
                                                                  desktop webview, phones
```

- The monitor never re-reads an unchanged file (inode, size, mtime) and never
  spawns `claude agents --json`. Historical transcripts are indexed lazily in the
  background, most recent first.
- Clients get a full `sessions` frame on connect and then only the sessions whose
  content changed. Transcript, agents and fleet frames go only to clients that
  subscribed; a subscriber that falls behind gets `resync` (or
  `transcriptReset`) and refetches over REST.
- Owned sessions also emit `stream` frames (text and thinking deltas), so they
  update word by word; foreign sessions update when Claude Code flushes each
  completed message.

### Why a session's state is derived

No single source says what a session is doing. `activity` combines, in order: a
pending decision; the pidfile `status` / `waitingFor`; the owned control
stream; the latest hook event (a `PreToolUse` without its `PostToolUse` means that
tool is running); and the transcript tail (a tool call without a result, a
prompt waiting for an answer, an API error).

## Networking model

- `127.0.0.1:<port>` (default 6725), plain HTTP: the desktop webview, local
  scripts, the hooks plugin and the Path A sidecar.
- Each private LAN address at the same port, TLS with a certificate from a local
  CA name-constrained to private ranges: phones at home.
- `127.0.0.1:6726`, only while Internet access is on: the port a tunnel forwards
  to. It serves a reduced router (no `/v1`, no `/hooks/*`) and requires
  `X-Forwarded-Proto: https`.
- `127.0.0.1:5612`, only once the bridge is started: the tokenless local API.

Every listener's router carries its `Ingress`; the context middleware builds a
`ClientContext` per request, and authorization is decided by principal (desktop,
local script, device, hook, sidecar), not by address. See SECURITY.md.

## Owned vs foreign control

- **Owned** (spawned or continued by Mother Claude): the installed `claude -p`
  speaking the stream-json control protocol (`--permission-prompt-tool stdio`),
  the way VS Code drives it. Approvals, questions, plans and elicitations arrive
  as control requests and become pending items; interrupt, model, mode, effort,
  rename, stop-task and queue-cancel go out as control requests. Path A (the
  Agent SDK sidecar) is kept behind `MOTHER_CLAUDE_SIDECAR=1`.
- **Foreign** (started in a terminal, VS Code, JetBrains, Claude Desktop or as a
  background job): monitored from pidfiles and transcripts, and controlled only
  through documented channels — the hooks plugin (held `PermissionRequest`,
  relays at tool boundaries and turn ends, a wake listener for idle sessions,
  interrupts at the next tool call), tmux panes, and experimental PTY attach for
  background jobs. **Continue here** turns any of them into an owned session
  (a fork when it is still running elsewhere).
- `caps::compute` decides per session what is possible; the dispatchers in
  `routes/control.rs` route each action to the matching channel, and the UI
  shows the same capabilities.

## Pending decisions

`crate::pending` is the one store for every decision a human must make,
whatever raised it (owned stream, hook, bridge host, sidecar). An item is
published to the session's subscribers and the inbox, triggers push, and can be
answered from any client through `POST /api/sessions/{id}/respond` (or the
bridge's `/requests`). The first answer wins; later ones get `409
already_decided`, and every client learns who answered (`pendingResolved`).
Whether an answer is dangerous is decided server-side
(`pending::classify_dangerous`).

## The Claude HTTP bridge

`src-tauri/src/bridge/` adds a programmatic HTTP surface — conversations, turns,
SSE event streams, steering, interruption, tool approvals, image input — modelled
on the Codex HTTP bridge so a client can be repointed between the two.

```
 HTTP client ──▶ bridge routes (axum) ──▶ RuntimeHost ──NDJSON──▶ bridge-host.js
                      │                        │                   (N Agent SDK
                      │                        │                    conversations)
                      ├─ OperationRegistry ────┤
                      │   (per-turn EventLog)  │
                      └─ /requests ────────────┴──▶ crate::pending (source: bridge)
                                                    (the dashboard's own cards)
```

Modules: `error` (one `{"error":{code,message,…}}` envelope), `events` (bounded
replay logs — 2048 events / 8 MiB, with the 410-expired / 422-beyond-head cursor
contract), `ops` (background operations, caps, retention), `host` (the Node
supervisor), `inputs` (content-block normalization plus image validation),
`messages` (the direct `api.anthropic.com` client), `openapi`, `assets`, `routes`,
`control` (start/stop from the app's API screen).

- **One approval, two surfaces.** A blocked tool call is a pending item with
  source `bridge` *and* a record in the bridge's `/requests` table. The dashboard
  card and the HTTP endpoint race; whichever answers first wins.
- **Mounted twice.** `/v1` on the main server (always token-required, TLS on the
  LAN, never through a tunnel) and a dedicated loopback port that is tokenless
  for requests from this machine only. The Node host starts lazily on first use.
