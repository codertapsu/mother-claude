# Claude HTTP bridge

Drive Claude on this machine over HTTP. Conversations, turns, live event
streams, steering, interruption, tool-approval callbacks, image and document
input — plus a direct Messages API path for the model-level features the coding
agent does not expose.

This is the Claude-side counterpart to a Codex HTTP bridge: same shapes, same
error envelope, same `thread → turn → operation` model, so a client written for
one can be repointed at the other by changing a base URL.

```bash
curl -sS http://127.0.0.1:5612/chat -H 'Content-Type: application/json' \
  -d '{"message":"What does this project do?","cwd":"/path/to/repo"}'
```

No token. One step first: open Mother Claude, go to **API**, and press **Start**.

---

## What is actually behind the endpoints

**Two backends, one surface.** Pick per request:

| Backend | Routes | What it gives you | Needs |
|---|---|---|---|
| **Claude Agent SDK** | `/threads`, `/chat`, `/turns`, `/run`, `/requests`, `/operations`, `/events` | A real Claude Code conversation: file access, bash, MCP servers, skills, subagents, and a human-in-the-loop approval loop | Your existing Claude Code sign-in |
| **Messages API** | `/messages`, `/files` | `api.anthropic.com` directly: the Files API, programmatic tool calling, server tools, coordinate-safe vision, arbitrary `tool_choice` | `ANTHROPIC_API_KEY` |

The Agent SDK backend runs in a long-lived Node process that Mother Claude
starts on first use. It is **not** the Claude Desktop app — that application
exposes no local API, and nothing can drive it. What the bridge drives is the
same runtime the dashboard already uses for the sessions it launches itself.

**A bridge conversation is a dashboard session.** Creating one through the API
makes it appear in the Mother Claude UI, and its tool-approval prompts show up
as cards there and on your phone. Either surface can answer; whichever answers
first unblocks the turn.

---

## Starting it

The API is **off when the app opens**. Starting it publishes a port that can
drive Claude with your account, so it is something you do rather than something
that happens.

Open Mother Claude → **API**. Choose:

- **Working directory** — where conversations run, and which files Claude reaches.
- **Model**, **Effort**, **Thinking** — leave any on *Default* to inherit your
  own Claude settings, exactly as VS Code or the CLI would.
- **Permissions** — from *Ask me* through to *Don't ask*.
- **Account** — which Claude account conversations run under, when there is
  more than one ([ACCOUNTS.md](ACCOUNTS.md)). *Default* follows the project's
  account, then the accounts' bridge default. *Only this account* makes the
  API run as that account alone ([below](#several-api-servers)).
- **Conversation** — which conversation messages join (see below).

Press **Start**. The screen shows the URL, links to the reference and console,
and the exact `curl` for your configuration. **Stop** unbinds the port;
conversations already running stay running.

Everything chosen here is a **default**. Any request can override any of it:

```bash
curl -sS http://127.0.0.1:5612/chat -H 'Content-Type: application/json' -d '{
  "message": "Review the diff.",
  "model": "opus",
  "effort": "xhigh",
  "cwd": "/somewhere/else",
  "permission_mode": "plan"
}'
```

Until it is started, the routes that drive Claude answer
`503 bridge_not_started`. Discovery (`/health`, `/capabilities`, `/auth`), this
document and the console stay available, so a client can always tell "not
started" from "not there".

For headless and CI runs, where there is nobody to press a button, set
`MOTHER_CLAUDE_BRIDGE_AUTOSTART=1` to start it at launch with plain defaults —
or `all` to start every saved [API server](#several-api-servers) too, each
closed to tunnels. `MOTHER_CLAUDE_BRIDGE_ACCOUNT` (an account's id or name)
chooses the autostarted API's account, and `MOTHER_CLAUDE_BRIDGE_ACCOUNT_ONLY=1`
makes it run as that account only; an account that does not exist starts
nothing rather than the wrong one.

### Controlling it over HTTP

The same controls are on the dashboard API on the main port, which always needs
a Mother Claude token (the desktop's, the local-script `script-token` file from this
Mac, or a paired device's). Starting with defaults that skip approvals
(`bypassPermissions`) is possible only from the desktop app:

| Method and path | Result |
|---|---|
| `GET /api/bridge` | Status, defaults, current conversation, running conversations |
| `POST /api/bridge/start` | `{"defaults": { … }, "openToTunnels": true\|false}` — start (or restart) with these defaults; `openToTunnels` is the API screen's switch (below), and only the desktop app may set it to `true` |
| `POST /api/bridge/stop` | Unbind the port |
| `POST /api/bridge/active-thread` | `{"threadId": "…"\|null}` — change which conversation messages join |

These drive the main API server; the [others](#several-api-servers) have
routes of their own.

---

## Which conversation a message joins

A client that only ever sends `{"message": "…"}` should be having *a
conversation*, not accumulating a new one per message. So:

1. **`thread_id` always wins.** Naming a conversation that is not currently
   running resumes it from disk — any past conversation, from this app or from
   your terminal, can be picked up. It does not change which conversation later
   unaddressed messages join. A conversation still open in another window is
   not taken over: that answers `409 thread_live_elsewhere`; fork it instead.
2. **`createNewChat: true`** starts a fresh conversation *and* makes it the one
   later messages join. It is `false` by default, deliberately.
3. Otherwise the message joins the **current conversation** — the one nominated
   on the API screen when you started, or the one an earlier message created.
4. With no current conversation, one is created and becomes current.

Worked through:

```bash
# Nothing yet: this creates a conversation and makes it current.
curl -sS $B/chat -d '{"message":"Remember the word ORCHARD."}' -H 'Content-Type: application/json'

# Joins it — no thread_id needed.
curl -sS $B/chat -d '{"message":"What word did I ask you to remember?"}' -H 'Content-Type: application/json'
# → "ORCHARD"

# Branch off a new one, which becomes current.
curl -sS $B/chat -d '{"message":"Fresh start.","createNewChat":true}' -H 'Content-Type: application/json'

# Reach back into the first one for a single message, without switching.
curl -sS $B/chat -d '{"message":"And the word?","thread_id":"<first id>"}' -H 'Content-Type: application/json'
```

`create_new_chat` is accepted as well as `createNewChat`, so either convention
works.

Selecting a conversation on the API screen — at start, or with **Switch
conversation** while running — is the same as step 3 above. `GET /api/bridge`
reports it as `activeThread`.

`/threads/{id}/turns` and `/run` are unaffected: they name their conversation
explicitly and always have.

With `account` in a `/chat` body, only a conversation of that account takes
the message: the active conversation is joined when it runs under it, and a
new one is opened when it does not. Naming a `thread_id` that runs under
another account answers `409 account_mismatch` (continue it on this account
with `POST /threads {"resume": …, "account": …}`, which forks it).

---

## Several API servers

Everything above is the **main** API server, the one there always was. More
can be added on the API screen (*Add API server*), each with a name, a
loopback port of its own (the first free one from 5613 unless you pick one),
its defaults and, above all, its own Claude account. Each is started and
stopped on its own, keeps its own current conversation and token policy, and
answers `503 bridge_not_started` until it is started. Added servers are saved
with their defaults in `api-servers.json` in the app data directory (0600);
whether one is open to tunnels is chosen at each start and never saved. The
main server's defaults are never saved, as before. One Node host serves every
server, and the conversations are one store.

**One account only.** A server's *Account* is the default for new
conversations, and a request may name another — the main server's behaviour.
With *Only this account* (`"accountOnly": true` in its defaults) the server
runs **only** as that account:

- new conversations run under it, and a request that names another account
  answers `403 account_not_served`;
- a conversation, operation, event or tool request of another account does not
  exist through it: reading, resuming, continuing, forking, renaming,
  steering, interrupting, streaming or answering one answers `404`
  (`thread_not_found`, `operation_not_found`, `request_not_found`), and
  `GET /threads`, `GET /requests` and `GET /events` leave them out;
- `GET /auth` reports that account's sign-in (with `account: {id, name}`), and
  `POST /auth/login` signs that account in (a token account gets a new token
  on the desktop instead).

That is what lets you put one account's API behind a tunnel without opening
the others. (`/messages` and `/files` use `ANTHROPIC_API_KEY` on every server;
they are not a Claude account's.)

**Reaching them.** Each server answers on its own port, at `/` and `/v1`, and
through the main ones:

| Address | Auth |
|---|---|
| `http://127.0.0.1:<its port>/…`, `/v1/…` | its own token policy |
| `http://127.0.0.1:5612/servers/<id>/…`, `/servers/<id>/v1/…` | its own token policy — one tunnel to 5612 reaches every server |
| `https://<LAN address>:6725/servers/<id>/v1/…` | Token, **always**, as on `/v1` there |
| The Internet-access port (`6726`) | — not served |

A request through a `/servers/<id>` prefix gets the target server's rules — its
token policy, its account, whether it is started — while the port it arrived
on decides whether a tunnel points at it. An id that names no server answers
`404 server_not_found`.

**Tunnels per server.** Tunnel detection tracks every server's port. A tunnel
to one switches off the tokenless mode of that port only, as it does for 5612,
and the API screen and the Remote page name the server it points at.

`/capabilities` and `/metadata` say which server answered —
`server: {id, name, account?: {id, name}, accountOnly}` — once there is more
than the main server or more than the Default account. A conversation names
the server that made it (`server`, left out for the main one).

**Controlling them over HTTP.** On the dashboard API, like the main server's
routes:

| Method and path | Result |
|---|---|
| `POST /api/bridge/servers` | `{"name", "port"?, "defaults"?}` → `201` its status, not started. Its id is a slug of the name (`[a-z0-9-]`, up to 32, made unique; `main` is taken) and never changes |
| `PATCH /api/bridge/servers/{id}` | `{"name"?, "port"?, "defaults"?}` — a running server restarts with them |
| `DELETE /api/bridge/servers/{id}` | Stop and remove it (`204`); its conversations go on as the main server's |
| `POST /api/bridge/servers/{id}/start` | `{"defaults"?, "openToTunnels"?}` — the defaults given are saved, without them the saved ones apply; left out, `openToTunnels` is off |
| `POST /api/bridge/servers/{id}/stop` | Unbind its port |
| `POST /api/bridge/servers/{id}/active-thread` | `{"threadId": "…"\|null}` — the conversation its messages join |

Each answers the server's status: `id`, `name`, `running`, `port`, `url`,
`prefix` (`/servers/<id>`, `""` for the main one), `startedAt`,
`requireToken`, `tokenPolicy`, `exposedByTunnel`, `tunnelUrl`, `defaults`,
`activeThread`, `threads`, `docsUrl`, `consoleUrl`, and `accountMissing: true`
when its account was removed. `GET /api/bridge` adds `servers` — every
server's status, the main one first — once a server besides the main one
exists. `main` answers on these routes too, but keeps its port and cannot be
changed or removed (`409 main_server_fixed`).

Anyone signed in to the dashboard may add, start, stop, change and remove
servers; the dangerous parts keep their rules — defaults that skip approvals
and *Open to tunnels* are the desktop app's alone, a default folder Claude Code
never opened needs the dangerous right. A name is 1–40 characters and unique
(`409 server_name_taken`); a port is 1024–65535 and nobody else's — not another
server's, the main API port, the app's or the tunnel port
(`409 port_unavailable`). Removing an account a running server uses is refused
(`409 account_used_by_bridge`, with `server` and `serverName` unless it is the
main one); a stopped server whose
account was removed shows `accountMissing` and answers `409 account_not_found`
until another account is chosen.

---

## Where it listens

| Address | Auth | Use it for |
|---|---|---|
| `http://127.0.0.1:5612/…` | **None for requests from this machine** (by default); a token for anything that came through a tunnel or proxy | Local scripts, Postman, Swagger, a web app you are building. |
| `http://127.0.0.1:5612/v1/…` | Same | Identical routes under a version prefix. |
| `http://127.0.0.1:6725/v1/…` | Token, **always** | The main app port, from this machine. |
| `https://<LAN address>:6725/v1/…` | Token, **always** (a paired device's) | The same mount from your LAN, over TLS. |
| `…/servers/<id>/…` on 5612, `…/servers/<id>/v1/…` on 6725 | As [above](#several-api-servers) | Another API server, through the main ones. |
| The Internet-access port (`6726`), or anything tunnelled to `6725` | — | Not served: `/v1` and `/servers/…` never answer through a tunnel. |

The dedicated port binds loopback only, always — the bridge can drive an agent
with filesystem write access, so it is never exposed to the network. LAN clients
use the `/v1` mount on the main server, which already has TLS and token auth.
From the LAN only a paired device's token works there; the desktop and
local-script tokens are accepted only from this machine.

Configuration lives in the environment (see `.env.example`):

| Variable | Default | Effect |
|---|---|---|
| `MOTHER_CLAUDE_BRIDGE` | on | `0` disables the bridge entirely |
| `MOTHER_CLAUDE_BRIDGE_AUTOSTART` | off | `1` starts the main API server at launch; `all` every saved one too, each closed to tunnels |
| `MOTHER_CLAUDE_BRIDGE_ACCOUNT` | unset | The autostarted main server's account: an id or a name |
| `MOTHER_CLAUDE_BRIDGE_ACCOUNT_ONLY` | off | `1`: the autostarted main server runs as that account only |
| `MOTHER_CLAUDE_BRIDGE_PORT` | `5612` | `0` serves only `/v1` on the main port |
| `MOTHER_CLAUDE_BRIDGE_REQUIRE_TOKEN` | **off** | `1` requires a bearer token **on the loopback port** too; `never` drops it for everyone who can reach that port, tunnels included (below) |
| `MOTHER_CLAUDE_BRIDGE_CWD` | your home directory | Default working directory for new conversations |
| `MOTHER_CLAUDE_BRIDGE_ORIGINS` | unset | Extra browser origins allowed to call the bridge |
| `ANTHROPIC_API_KEY` | unset | Enables `/messages` and `/files` |
| `ANTHROPIC_BASE_URL` | `https://api.anthropic.com` | Point the Messages API path elsewhere |

### Two different sign-ins

These are unrelated and it is worth keeping them apart:

| | What it is | Where it lives |
|---|---|---|
| **Claude's account** | The Anthropic account the model runs as | Handled by `claude` itself; checked when Mother Claude opens |
| **The bridge token** | Mother Claude's own API token | Off by default on the loopback port |

**Claude's account** is the one that matters. Mother Claude checks it at startup
and prints the result:

```
  Claude: you@example.com (max)
```

If it is missing, every turn would otherwise come back as the string
`Not logged in · Please run /login` — a per-request mystery. Instead you get one
line at startup, `GET /auth` any time, and `POST /auth/login` to start the
browser flow:

```bash
curl -sS http://127.0.0.1:5612/auth
curl -sS -X POST http://127.0.0.1:5612/auth/login -d '{}' -H 'Content-Type: application/json'
curl -sS 'http://127.0.0.1:5612/auth?refresh=true'   # poll until authenticated
```

Signing in opens a browser on the host machine, so `POST /auth/login` is
restricted to local clients (and devices granted dangerous actions).
`claude auth login` in a terminal does the same thing. Mother Claude runs it
in a pseudo-terminal and opens the sign-in page itself (only Claude's and
Anthropic's own https pages); one sign-in runs at a time across all accounts,
so while another runs it answers `409 sign_in_running` (docs/ACCOUNTS.md
§15.2–15.3).

**The bridge token** is off by default on `127.0.0.1:5612` for requests from
this machine, which is what makes the examples in this document work as written.
Turn it on with `MOTHER_CLAUDE_BRIDGE_REQUIRE_TOKEN=1` and present a token as
`Authorization: Bearer <token>` (or `?token=<token>` from this machine, for
`EventSource`, which cannot set headers). Scripts on this Mac use the
`script-token` file in the app data directory (see [REMOTE.md](REMOTE.md#for-scripts)); a paired
device uses its own token.

"From this machine" means a loopback connection with a loopback `Host` and no
proxy headers. A development server or reverse proxy on this Mac (Next.js
rewrites, `http-proxy` with `xfwd`, a local nginx or Caddy) counts too when the
only proxy headers are `X-Forwarded-*`, every `X-Forwarded-For` hop is a
loopback address and `X-Forwarded-Host` (if sent) names `localhost`, `127.x` or
`[::1]`. A request that came through a tunnel — a non-loopback hop, or headers
a tunnel adds (`cf-*`, `ngrok-*`, `tailscale-*`, `Via`, `Forwarded`, …) — needs
a paired device's token (`401 tunneled_request_requires_token`; the desktop and
`script-token` credentials are only accepted from this machine), and while
Mother Claude sees a tunnel pointing at this port the tokenless mode is off for
everyone (`401 bridge_exposed_requires_token`) until that tunnel stops.

The `/v1` mount on the main app port always requires a token, because that
listener is reachable from your LAN.

### Browser origins

Any origin may call the bridge by default, so a page you are building can
`fetch` it without ceremony. Naming origins in `MOTHER_CLAUDE_BRIDGE_ORIGINS`
switches to strict allowlisting, and anything else then gets
`403 origin_not_allowed`:

```bash
MOTHER_CLAUDE_BRIDGE_ORIGINS=http://localhost:4200,https://myapp.test
```

### What the open default actually means

On the default settings, **any process on this machine — and any web page you
have open — can drive Claude through `127.0.0.1:5612` with your permissions and
your account.** That includes reading and writing files under whatever `cwd` it
names. This is the same posture as a local Codex bridge, and it is the right
trade-off for a developer tool on your own machine; it is not one to make on a
shared or untrusted one.

These stay locked regardless:

- the port is **loopback-only** and never binds an external interface;
- **dangerous** requests need a local client on this port — no foreign
  `Origin`, no proxy headers, with or without the `script-token` file — or a token
  with the right to them (the desktop app's, or a paired device granted
  dangerous actions; on the `/v1` mount of the main port the `script-token` file does
  not grant them): `permission_mode: bypassPermissions`, transcript
  deletion, `POST /auth/login`, approvals of dangerous actions, thread options
  that run code nobody approves (`mcp_servers`, `allowed_tools`,
  `additional_directories`, a `cwd` Claude Code was never opened in, resuming a
  conversation whose approvals are off), and turns sent into a conversation
  whose approvals are off. A page on another website therefore keeps tokenless
  access to ordinary work, never to these;
- tunnelled requests need a paired device's token, and a detected tunnel to
  the port switches the tokenless mode off — each API server's port on its
  own;
- the LAN mount on port 6725 **always** requires the token.

**Open to tunnels too: the API screen's _Open to tunnels_ switch.** An
explicit opt-in for publishing the API through a tunnel you run (`ngrok http
5612`, cloudflared, …) without handing callers a token. Turn it on under
**Access** on the API screen and start (or restart) the API: no request to 5612
then needs a token, wherever it comes from, and a tunnel pointing at the port
no longer switches anything off. Only the desktop app can turn it on — a phone
or a script starting the API gets `desktop_only` — and it lasts until you
restart the API with it off, or quit the app. `MOTHER_CLAUDE_BRIDGE_REQUIRE_TOKEN=never`
sets its starting position, so every launch begins with it on. Anyone who finds the tunnel's URL can then start
conversations in your trusted folders and approve Claude's ordinary tool calls
— shell commands included — on this Mac, with your Claude account. What stays
closed:

- the dangerous actions listed above still need a caller on this Mac, and while
  a tunnel points at the port **no** tokenless request counts as one (a raw TCP
  tunnel adds no headers and lets its caller name a loopback `Host`), so then
  only a token holder has them;
- the LAN mount on port 6725 keeps requiring the token.

The app says so where you would look: a warning when the API starts, on the
API screen, and on the Remote page while a tunnel points at 5612. `/capabilities`
reports `auth.token_policy: "never"`. Turn the switch off and restart the API
to return to the default. Every [added API server](#several-api-servers) has
the same switch, chosen at each of its starts and never saved (an added server
starts closed to tunnels, even with `MOTHER_CLAUDE_BRIDGE_REQUIRE_TOKEN=never`).

To close it down: `MOTHER_CLAUDE_BRIDGE_REQUIRE_TOKEN=1`, plus
`MOTHER_CLAUDE_BRIDGE_ORIGINS` to name the pages you trust — or
`MOTHER_CLAUDE_BRIDGE_PORT=0` to drop the dedicated port entirely and use the
token-required `/v1` mount only. A page on that list, calling
`127.0.0.1:5612` from this Mac, sends the `script-token` file's value as
`Authorization: Bearer <token>` (or `?token=` for `EventSource`); the list never
makes it a desktop client, so dangerous requests stay refused.

---

## Try it

With the app running:

- **[API reference](http://127.0.0.1:5612/docs/)** — Swagger UI, vendored locally,
  no CDN. Its schema is generated per request, so it describes the listener you
  loaded it from, including whether that listener wants a token.
- **[Console](http://127.0.0.1:5612/example/)** — a runnable page: start a
  conversation, send text and images, watch the stream, approve tool calls.
- **[OpenAPI 3.1](http://127.0.0.1:5612/openapi.json)** — import into Postman or
  generate a client.
- **[client.mjs](http://127.0.0.1:5612/client.mjs)** — the JS client, importable
  straight from a page.

Swagger's **Try it out** works with no Authorize step on the default settings.

---

## Conversations and turns

A **thread** is a conversation. A **turn** is one exchange within it. Starting a
turn returns immediately with an **operation** you can stream, poll, or wait on.

### The one-call version

`POST /chat` creates a conversation (or continues one) and waits for the answer:

```bash
curl -sS http://127.0.0.1:5612/chat -H 'Content-Type: application/json' -d '{
  "message": "Summarize README.md in one sentence.",
  "cwd": "/path/to/repo",
  "model": "haiku"
}'
```

(Add `-H "Authorization: Bearer $TOKEN"` only if you turned the token on.)

```json
{
  "thread_id": "c713e1bb-…",
  "turn_id": "1e2ca10d-…",
  "operation_id": "1f951cbe-…",
  "status": "completed",
  "response": "…",
  "structured_output": null,
  "usage": { "input_tokens": 10, "output_tokens": 57, "…": "…" },
  "total_cost_usd": 0.0126314
}
```

Pass the returned `thread_id` back to continue:

```bash
curl -sS http://127.0.0.1:5612/chat -H 'Content-Type: application/json' \
  -d '{"thread_id":"c713e1bb-…","message":"Now list its dependencies."}'
```

Any conversation option (`cwd`, `model`, `effort`, `permission_mode`, …) may
ride along and applies when a new conversation is created.

### The explicit version

```bash
# 1. Start a conversation. You hold its id before any model work happens.
curl -sS http://127.0.0.1:5612/threads -H 'Content-Type: application/json' -d '{
  "cwd": "/path/to/repo",
  "model": "sonnet",
  "effort": "high",
  "permission_mode": "default"
}'
# 201 -> {"thread_id":"…","status":"created","cwd":"…"}

# 2. Start a turn. Returns at once.
curl -sS http://127.0.0.1:5612/threads/$THREAD/turns \
  -H 'Content-Type: application/json' -d '{"input":"Add a test for parse_config."}'
# 202 + X-Operation-Id -> {"operation_id":"…","turn_id":"…","status":"running",…}

# 3. Watch it.
curl -N http://127.0.0.1:5612/operations/$OPERATION/events

# …or block instead of streaming:
curl -sS http://127.0.0.1:5612/threads/$THREAD/run \
  -H 'Content-Type: application/json' -d '{"input":"…"}'
```

### Conversation options

`POST /threads` accepts, and rejects anything it does not recognise:

| Field | Notes |
|---|---|
| `cwd` | Working directory. Must exist on this machine. |
| `model` | `haiku`, `sonnet`, `opus`, `default`, or a full model id. `GET /models` lists them. |
| `effort` | `low` · `medium` · `high` · `xhigh` · `max` |
| `thinking` | `on` · `off`. Omit to inherit your own Claude settings. |
| `permission_mode` | `default` · `plan` · `acceptEdits` · `dontAsk` · `auto` · `bypassPermissions` |
| `thread_id` | Pre-assign the conversation id (must be a UUID). |
| `resume` | Continue an existing conversation in place (a fork when it is still running in another window; the answer then has `forked: true` and the new id). |
| `fork` | With `resume`, branch to a new id instead. |
| `system_prompt_append` | Extra instructions appended to Claude Code's own prompt. |
| `allowed_tools` / `disallowed_tools` | Restrict the tool surface. |
| `additional_directories` | Extra readable/writable roots. |
| `setting_sources` | Which of `user`, `project`, `local` settings to load. `[]` isolates the conversation — it will not read `CLAUDE.md`. |
| `mcp_servers` | MCP servers for this conversation. |
| `output_schema` | JSON Schema; the result arrives as `structured_output`. |
| `max_turns`, `max_budget_usd` | Hard stops. |
| `skills`, `agents` | Skills and programmatic subagents. |
| `account` | The Claude account to run under: `default` or an `acc_…` id from `GET /api/accounts`. Default: the start-time choice, then the project's account, then the accounts' bridge default. With `resume`, another account than the conversation's forks it; an account that keeps its own directory needs the conversation copied there first, which the desktop allows once per pair of accounts — until then `409 copy_consent_required`. A token account whose token is not on this machine answers `409 account_token_missing`. A conversation that ran with a token no account holds counts as the Default's. |

A conversation names its `account` (in `GET /threads` and the other thread
answers) only when it is not the Default's: **absent means `default`**, so
with only the Default every answer reads as it did before accounts. The same
goes for `account` in `GET /api/bridge`'s defaults: absent means none was
chosen at start.

`bypassPermissions` skips every approval prompt, so it is restricted to local
clients and to devices granted dangerous actions. So are `mcp_servers`, a
non-empty `allowed_tools`, `additional_directories`, resuming a conversation
whose approvals are off, and a `cwd` that Claude Code has never been opened in
on this Mac (`403 untrusted_cwd`) — under the conversation's account: each
account keeps its own trust, so a folder the Default opened is still new to a
directory account that never did.

### The rest of the conversation surface

| Method and path | Result |
|---|---|
| `GET /auth` | Which Anthropic account Claude is signed in as; `?refresh=true` re-checks |
| `POST /auth/login` | Start the browser sign-in flow (local clients, or a device granted dangerous actions) |
| `GET /threads` | Live conversations plus what is on disk |
| `GET /threads/{id}` | One conversation; `?include_messages=true` inlines its transcript |
| `GET /threads/{id}/messages` | The stored transcript |
| `GET /threads/{id}/context` | Context-window breakdown: what is using the window |
| `POST /threads/{id}/name` | Set the stored title |
| `POST /threads/{id}/fork` | Branch into a new conversation id |
| `POST /threads/{id}/model` | Switch model mid-conversation |
| `POST /threads/{id}/permission-mode` | Change approval behaviour live |
| `DELETE /threads/{id}` | End it. `?purge=true` also deletes the transcript (local clients, or a device granted dangerous actions) |
| `POST /threads/{id}/turns/{turn}/steer` | Add input to a turn already running |
| `POST /threads/{id}/turns/{turn}/interrupt` | Stop the turn, keep the conversation |

---

## Input: text, images and documents

`input` is a string, one item, or an array of them. Anthropic content blocks pass
through untouched, so anything the Messages API accepts works here.

```json
{
  "input": [
    { "type": "image", "url": "data:image/png;base64,…" },
    { "type": "localImage", "path": "/Users/you/Desktop/screenshot.png" },
    { "type": "localDocument", "path": "/Users/you/spec.pdf" },
    { "type": "text", "text": "Which button is misaligned? Give pixel coordinates." }
  ]
}
```

`localImage` and `localDocument` are bridge conveniences: the file is read and
base64-encoded for you. Everything is validated **before** the runtime is
touched, so a bad image is a typed `422`/`413` from the bridge rather than an
opaque sentence from the model:

| Limit | Value |
|---|---|
| Image formats | PNG, JPEG, GIF, WebP — and the bytes must match the declared `media_type` |
| Image size | 10 MiB decoded |
| Image dimensions | 8 – 8000 px per edge |
| Images per message | 100 |
| Message text | 32,000 characters |
| Request body | 32 MiB |

The 8000 px ceiling is deliberate: above it the API silently resizes, which
breaks any request for pixel coordinates. The 8 px floor catches images the
model rejects with an untyped error.

Files API `file_id` sources are **refused** from bridge clients. Uploads are
workspace-scoped, so accepting an id from a caller would let one client read
another's files. Upload through `POST /files` instead.

---

## Streaming

`GET /operations/{id}/events` streams one turn. `GET /events` streams everything
across every conversation.

```text
id: 1
data: {"method":"turn/started","params":{"thread_id":"…","turn_id":"…","text":"…"}}

id: 2
data: {"method":"message","params":{"thread_id":"…","turn_id":"…","message":{"type":"assistant",…}}}

id: 7
data: {"method":"bridge/completed","params":{"operation_id":"…","status":"completed","result":{…}}}
```

Frames you will see:

| `method` | Meaning |
|---|---|
| `turn/started` | The turn opened. Carries the text and image count you sent. |
| `turn/steered` | Input was added to a running turn. |
| `message` | One Agent SDK message: assistant text, thinking, tool use, partial deltas, status. |
| `bridge/request` | A tool approval or question is waiting on a human. |
| `bridge/completed` / `bridge/error` | Terminal. The log is closed; the stream ends. |

**Replay.** Each operation keeps its own log (2048 events / 8 MiB). An operation
stream replays from the beginning by default, so connecting after a turn has
finished still gives you the whole thing. Resume with `?after=<id>` or
`Last-Event-ID`. A cursor older than the retained window is a readable
`410 events_expired`, validated *before* the stream opens — never a stream that
dies without explanation.

A fresh `/events` subscriber starts at the oldest retained event. Heartbeat
comments arrive every 15s. **Disconnecting does not interrupt Claude** — use the
interrupt endpoint.

`EventSource` cannot set headers. When a token is needed, it works with
`?token=` only from this machine (a query token is refused from anywhere else);
the bundled JS client uses `fetch` instead so the token stays in a header.

---

## Tool approvals and questions

When Claude wants to run a tool that is not auto-approved, or asks you a
question, the turn blocks and the request appears here:

```bash
curl -sS http://127.0.0.1:5612/requests
```
```json
{ "data": [{
  "request_id": "…", "thread_id": "…", "turn_id": "…",
  "kind": "permission", "dangerous": false,
  "params": { "tool": "Bash", "prompt": "Run tests", "detail": "npm test", "input": {…} },
  "status": "pending", "created_at": 1789750000000
}] }
```

Answer it:

```bash
# Approve
curl -sS http://127.0.0.1:5612/requests/$ID/respond -d '{"decision":"allow"}' -H 'Content-Type: application/json'

# Approve, and stop asking for this tool
curl -sS http://127.0.0.1:5612/requests/$ID/respond -H 'Content-Type: application/json' -d '{
  "decision": "allow",
  "updated_permissions": [{ "type": "addRules", "behavior": "allow",
                            "rules": [{ "toolName": "Bash" }], "destination": "session" }]
}'

# Deny with a reason Claude can read
curl -sS http://127.0.0.1:5612/requests/$ID/respond -H 'Content-Type: application/json' \
  -d '{"decision":"deny","message":"Use the staging database instead."}'

# Answer a question
curl -sS http://127.0.0.1:5612/requests/$ID/respond -H 'Content-Type: application/json' \
  -d '{"answer":"postgres"}'
```

`{"result": {…}}` sends the raw callback result if you need full control.

Unanswered requests are denied after 10 minutes. Approving something flagged
`dangerous` is restricted to local clients and devices granted dangerous
actions — and so is *any* approval that also grants a standing privilege, such
as attaching a `setMode: bypassPermissions` update or writing a rule to
`userSettings`. The severity of an answer is not the severity of the question it
answers. Whether a request is `dangerous` is decided by the server (for example
an edit to `.claude/` or a shell startup file), not by the client.

---

## Direct Messages API

`POST /messages` is a faithful pass-through to `POST /v1/messages`, with three
conveniences: `localImage`/`localDocument` expansion inside `messages[].content`,
`betas` as a JSON field instead of a header, and `model`/`max_tokens` defaults.
Everything else — `tools`, `tool_choice`, `output_config`, `container`,
`transformations`, server tools — is forwarded untouched, so the platform
documentation is the reference and nothing here goes stale as the API grows.

**Vision with coordinates.** Opt out of silent resizing so returned pixel
coordinates refer to the image you sent:

```json
{
  "messages": [{ "role": "user", "content": [
    { "type": "localImage", "path": "/Users/you/ui.png" },
    { "type": "text", "text": "Give the bounding box of the Submit button in pixels." }
  ]}],
  "transformations": { "oversized_image": "error" }
}
```

**Programmatic tool calling.** Claude calls your tool from inside code execution:

```json
{
  "tools": [
    { "type": "code_execution_20260120", "name": "code_execution" },
    { "name": "query_database", "description": "…", "input_schema": {…},
      "allowed_callers": ["code_execution_20260120"] }
  ],
  "messages": [{ "role": "user", "content": "Which region grew fastest last quarter?" }]
}
```

When you answer a pending programmatic call, the user message must contain
**only** `tool_result` blocks, and you must pass back the `container` id from the
paused response along with the same `tools` array.

**Files.**

```bash
curl -sS http://127.0.0.1:5612/files -F 'file=@/path/to/report.pdf'
curl -sS http://127.0.0.1:5612/files
curl -sS http://127.0.0.1:5612/files/$FILE_ID/content -o out.pdf
curl -sS -X DELETE http://127.0.0.1:5612/files/$FILE_ID
```

Reference an uploaded file from a Messages API call with
`{"type":"document","source":{"type":"file","file_id":"…"}}`. The conversation
endpoints deliberately do not accept `file_id`s — inline the bytes instead.

Streaming works by setting `"stream": true`; the upstream SSE is proxied verbatim.

Without `ANTHROPIC_API_KEY`, every route in this section answers `503
messages_api_unconfigured` with instructions. Conversation endpoints are
unaffected — they use your Claude Code sign-in, which is not an API key.

---

## JavaScript client

```javascript
import { chat, createThread, startTurn, streamOperation, respondToRequest }
  from 'http://127.0.0.1:5612/client.mjs';

const conn = { baseUrl: 'http://127.0.0.1:5612', token: TOKEN };

const { thread_id } = await createThread(conn, { cwd: '/repo', model: 'sonnet' });
const op = await startTurn(conn, thread_id, 'Refactor parse_config and add tests.');

for await (const { data } of streamOperation(conn, op.operation_id)) {
  if (data.method === 'bridge/request') {
    await respondToRequest(conn, data.params.request_id, { decision: 'allow' });
  }
  if (data.method === 'bridge/completed') console.log(data.params.result.result);
}
```

An omitted `baseUrl` uses the current page's origin in a browser and
`http://127.0.0.1:5612` in Node. Omit `token` when the listener runs tokenless.
`runTurnStreaming` wraps start-watch-settle if that is all you need.

The full source, with every export documented, is at
[`/client.mjs`](http://127.0.0.1:5612/client.mjs); the console at
[`/example/`](http://127.0.0.1:5612/example/) is a worked example of it.

---

## Errors

Every failure is the same envelope, with context fields flattened alongside:

```json
{ "error": { "code": "busy", "message": "…", "thread_id": "…", "turn_id": "…" } }
```

| Status | Codes | What to do |
|---|---|---|
| 400 | `invalid_json`, `invalid_multipart` | Fix the body |
| 401 | `unauthorized` | Present the API token |
| 401 | `claude_login_required` | Claude itself is signed out — run `claude` and `/login` |
| 401 | `device_revoked` | The paired device was revoked or expired mid-request; its open streams end too. Pair again |
| 401 | `tunneled_request_requires_token`, `bridge_exposed_requires_token` | The request came through a tunnel (or a proxy with a non-loopback hop), or a tunnel points at the port: send a paired device's token |
| 403 | `dangerous_blocked`, `untrusted_cwd`, `file_id_not_accepted` | Do it from a local client (or a device granted dangerous actions), or don't |
| 403 | `origin_not_allowed` | Add the page's origin to `MOTHER_CLAUDE_BRIDGE_ORIGINS` |
| 403 | `account_not_served` | This API server runs as one account only: leave `account` out, or use a server of that account |
| 404 | `thread_not_found`, `turn_not_found`, `operation_not_found`, `request_not_found`, `not_found` | Check the id; operations do not survive a restart; through a server that runs as one account, another account's do not exist |
| 404 | `server_not_found` | No API server has that id: check `/servers/<id>` |
| 405 | `method_not_allowed` | Check the method against the reference |
| 409 | `busy` | A turn is already running (steer it) or 16 operations are live |
| 409 | `already_answered`, `invalid_state` | Someone else got there first |
| 409 | `thread_live_elsewhere` | The conversation is running in another window: fork it (`resume` on `POST /threads` does that for you) |
| 409 | `already_owned` | Mother Claude already drives that session: use its existing conversation |
| 409 | `switching_account` | The conversation (or session) is switching to another account on the dashboard (docs/ACCOUNTS.md §15.1): try again in a moment — it goes on under the same id |
| 410 | `events_expired` | Your cursor fell out of the replay window; read the operation result |
| 413 | `request_size`, `image_size`, `image_dimensions` | Shrink it |
| 415 | `content_type` | Send `application/json` |
| 422 | `invalid_field`, `unknown_field`, `invalid_input`, `invalid_cursor`, `invalid_id`, `image_format` | `error.field` names what was wrong |
| 502 | `claude_error`, `anthropic_error` | The runtime or the API failed; `error.upstream` has the detail |
| 503 | `bridge_not_started` | Start the API on the app's **API** screen |
| 503 | `runtime_unavailable`, `messages_api_unconfigured` | The host is not built/running, or no API key |
| 504 | `timeout` | Still running — poll the operation or stream its events |

---

## Limits and behaviour worth knowing

`GET /capabilities` reports all of this at runtime; read it instead of
hardcoding.

- **16 concurrent operations**; the 17th gets `409 busy`.
- **128 retained operations**, 30 minutes each. Only finished ones are evicted,
  oldest first.
- **One turn at a time per conversation.** A second `POST /turns` returns `409`
  with the running `turn_id` — steer it instead of racing it.
- **Conversations are process-local.** One created here is lost on restart until
  its first turn has written a transcript — after that it can be resumed by id,
  or nominated on the API screen. Operations and event logs never
  survive a restart.
- **Nothing is retried or replayed.** A disconnected client does not cancel a
  running turn, and there is no idempotency key. Keep your ids and check
  `GET /operations/{id}` before resubmitting.
- **`/run` waits up to 30 minutes**, then returns `504` with the operation id —
  the turn is still running.
- The Node host starts on first use, not at app startup, and is respawned after
  a crash. `GET /health` reports whether it is built and running.

---

## Troubleshooting

**`503 bridge_not_started`** — the API is off. Open Mother Claude → **API** →
**Start**, or set `MOTHER_CLAUDE_BRIDGE_AUTOSTART=1` for a headless run.

**`503 runtime_unavailable` / `host_built: false`** — the host is not built:

```bash
npm run sidecar:build
```

**`409 port_unavailable` when starting** — something else holds 5612. Change
`MOTHER_CLAUDE_BRIDGE_PORT`, or set it to `0` to serve only the `/v1` mount on
the main app port.

**Every turn says "Not logged in · Please run /login"** — Claude itself is signed
out, which the startup banner and `GET /auth` both report. Fix it with
`POST /auth/login` or `claude auth login`. Note that **exporting
`CLAUDE_CONFIG_DIR` breaks credential resolution even when it points at the
default `~/.claude`**; leave it unset unless you genuinely use a non-default
config directory.

**`409 busy` on every turn** — a previous turn never finished. Check
`GET /threads/{id}`, then interrupt the turn it names.

**`403 account_not_served`, or a conversation that is there on one port and
`404` on another** — the server you called runs as one account only, and that
conversation (or the account you named) is another's. Leave `account` out, or
call a server of that account.

**`404 server_not_found`** — no API server has the id in `/servers/<id>`. The
ids are on the API screen, and in `GET /api/bridge`'s `servers`.

**Turns hang with no events** — look at `GET /requests`: Claude is probably
blocked on an approval. The dashboard shows the same prompt.

**The console page can't connect** — leave its Base URL blank when it is served
by the bridge itself. Set it only when hosting the page elsewhere, in which case
the browser's local-network permission and CORS also apply.

---

## Verification

```bash
# Offline: auth, error envelope, assets, OpenAPI, validation. Safe for CI.
cargo test --manifest-path src-tauri/Cargo.toml --test bridge

# Live: real conversations, real tokens. Needs `claude` signed in.
npm run sidecar:build
cargo test --manifest-path src-tauri/Cargo.toml --test bridge -- --ignored --nocapture
```

The live suite covers a conversation round trip, `/chat` context recall across
turns, and a vision turn with an inline PNG. It spends a few cents.

---

## See also

- [ARCHITECTURE.md](../ARCHITECTURE.md) — how the bridge sits inside the app
- [SECURITY.md](../SECURITY.md) — exposure model
- [KNOWN_ISSUES.md](../KNOWN_ISSUES.md) — research-preview internals this leans on
- [Tool use](https://platform.claude.com/docs/en/agents-and-tools/tool-use/overview) ·
  [Programmatic tool calling](https://platform.claude.com/docs/en/agents-and-tools/tool-use/programmatic-tool-calling) ·
  [Files](https://platform.claude.com/docs/en/build-with-claude/files) ·
  [Vision](https://platform.claude.com/docs/en/build-with-claude/vision) ·
  [Vision coordinates](https://platform.claude.com/docs/en/build-with-claude/vision-coordinates)
