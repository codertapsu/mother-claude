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
`MOTHER_CLAUDE_BRIDGE_AUTOSTART=1` to start it at launch with plain defaults.

### Controlling it over HTTP

The same controls are on the dashboard API, which needs the Mother Claude token
because it is reachable from your LAN:

| Method and path | Result |
|---|---|
| `GET /api/bridge` | Status, defaults, current conversation, running conversations |
| `POST /api/bridge/start` | `{"defaults": { … }}` — start (or restart) with these defaults |
| `POST /api/bridge/stop` | Unbind the port |
| `POST /api/bridge/active-thread` | `{"threadId": "…"\|null}` — change which conversation messages join |

---

## Which conversation a message joins

A client that only ever sends `{"message": "…"}` should be having *a
conversation*, not accumulating a new one per message. So:

1. **`thread_id` always wins.** Naming a conversation that is not currently
   running resumes it from disk — any past conversation, from this app or from
   your terminal, can be picked up. It does not change which conversation later
   unaddressed messages join.
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

---

## Where it listens

| Address | Auth | Use it for |
|---|---|---|
| `http://127.0.0.1:5612/…` | **None by default** | Local scripts, Postman, Swagger, a web app you are building. |
| `http://127.0.0.1:5612/v1/…` | Same | Identical routes under a version prefix. |
| `http://127.0.0.1:6725/v1/…` | Token, **always** | The main app port. Reachable from your LAN over TLS. |

The dedicated port binds loopback only, always — the bridge can drive an agent
with filesystem write access, so it is never exposed to the network. LAN clients
use the `/v1` mount on the main server, which already has TLS and token auth.

Configuration lives in the environment (see `.env.example`):

| Variable | Default | Effect |
|---|---|---|
| `MOTHER_CLAUDE_BRIDGE` | on | `0` disables the bridge entirely |
| `MOTHER_CLAUDE_BRIDGE_PORT` | `5612` | `0` serves only `/v1` on the main port |
| `MOTHER_CLAUDE_BRIDGE_REQUIRE_TOKEN` | **off** | `1` requires a bearer token **on the loopback port** too |
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
restricted to local clients. `claude auth login` in a terminal does the same
thing.

**The bridge token** is off by default on `127.0.0.1:5612`, which is what makes
the examples in this document work as written. Turn it on with
`MOTHER_CLAUDE_BRIDGE_REQUIRE_TOKEN=1` and present it as
`Authorization: Bearer <token>`, `?token=<token>` (for `EventSource`, which
cannot set headers), or an `mc_token` cookie. The token is printed at startup
and shown under **Settings → Pair a phone**.

The `/v1` mount on the main app port always requires it, because that listener
is reachable from your LAN.

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

Three things stay locked regardless:

- the port is **loopback-only** and never binds an external interface;
- `permission_mode: bypassPermissions`, transcript deletion, `POST /auth/login`
  and approvals of dangerous actions are **local-client only**;
- the LAN mount on port 6725 **always** requires the token.

To close it down: `MOTHER_CLAUDE_BRIDGE_REQUIRE_TOKEN=1`, plus
`MOTHER_CLAUDE_BRIDGE_ORIGINS` to name the pages you trust — or
`MOTHER_CLAUDE_BRIDGE_PORT=0` to drop the dedicated port entirely and use the
token-required `/v1` mount only.

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
| `resume` | Continue an existing conversation in place. |
| `fork` | With `resume`, branch to a new id instead. |
| `system_prompt_append` | Extra instructions appended to Claude Code's own prompt. |
| `allowed_tools` / `disallowed_tools` | Restrict the tool surface. |
| `additional_directories` | Extra readable/writable roots. |
| `setting_sources` | Which of `user`, `project`, `local` settings to load. `[]` isolates the conversation — it will not read `CLAUDE.md`. |
| `mcp_servers` | MCP servers for this conversation. |
| `output_schema` | JSON Schema; the result arrives as `structured_output`. |
| `max_turns`, `max_budget_usd` | Hard stops. |
| `skills`, `agents` | Skills and programmatic subagents. |

`bypassPermissions` skips every approval prompt, so it is restricted to local
clients unless `MOTHER_CLAUDE_ALLOW_REMOTE_DANGEROUS=1`.

### The rest of the conversation surface

| Method and path | Result |
|---|---|
| `GET /auth` | Which Anthropic account Claude is signed in as; `?refresh=true` re-checks |
| `POST /auth/login` | Start the browser sign-in flow (local clients only) |
| `GET /threads` | Live conversations plus what is on disk |
| `GET /threads/{id}` | One conversation; `?include_messages=true` inlines its transcript |
| `GET /threads/{id}/messages` | The stored transcript |
| `GET /threads/{id}/context` | Context-window breakdown: what is using the window |
| `POST /threads/{id}/name` | Set the stored title |
| `POST /threads/{id}/fork` | Branch into a new conversation id |
| `POST /threads/{id}/model` | Switch model mid-conversation |
| `POST /threads/{id}/permission-mode` | Change approval behaviour live |
| `DELETE /threads/{id}` | End it. `?purge=true` also deletes the transcript (local clients only) |
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

`EventSource` works if you pass `?token=`; the bundled JS client uses `fetch`
instead so the token stays in a header.

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

Unanswered requests are denied after 10 minutes. Approving something flagged `dangerous` is restricted to local clients — and so
is *any* approval that also grants a standing privilege, such as attaching a
`setMode: bypassPermissions` update or writing a rule to `userSettings`. The
severity of an answer is not the severity of the question it answers.

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
| 403 | `dangerous_blocked`, `file_id_not_accepted` | Do it from a local client, or don't |
| 403 | `origin_not_allowed` | Add the page's origin to `MOTHER_CLAUDE_BRIDGE_ORIGINS` |
| 404 | `thread_not_found`, `turn_not_found`, `operation_not_found`, `request_not_found`, `not_found` | Check the id; operations do not survive a restart |
| 405 | `method_not_allowed` | Check the method against the reference |
| 409 | `busy` | A turn is already running (steer it) or 16 operations are live |
| 409 | `already_answered`, `invalid_state` | Someone else got there first |
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
