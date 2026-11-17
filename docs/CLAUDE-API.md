# The Claude API, answered by Claude Code

Every API server of the HTTP API ([BRIDGE.md](BRIDGE.md)) also speaks the
**Claude API** (platform.claude.com/docs/en/api): the same endpoints, request
bodies, answers, streamed events and errors as `https://api.anthropic.com`. A
client written for the Claude API — Postman, the Anthropic SDKs, curl, your
own code — works here with **only its base URL changed**. Claude Code on this
computer answers, signed in as the server's account: no Anthropic API key is
needed, and a key a client sends is never used, kept or passed on.

Code: `src-tauri/src/bridge/claude_api/` (`wire.rs` the request and error
shapes, `routes.rs` `/v1/messages`, `stream.rs` the events, `convert.rs`
fingerprints and history, `store.rs` conversations and clients, `access.rs`
who may call and `network.rs` the listeners beyond this computer,
`extras.rs` count_tokens, models and `/v1/conversations`, `admin.rs` the
dashboard's routes, `sweep.rs`), the bare sessions in
`sidecar/src/bridge-host.ts` (`bareOptions`, `clientToolsServer`). Tests:
`src-tauri/tests/claude_api.rs` (offline, the fake host), the unit tests
beside each module, and `live_claude_api_messages` in `tests/bridge.rs`
(real Claude Code, `--ignored`).

---

## Use it

Start the API on **HTTP API** (or tick *Start when Mother Claude opens*,
[below](#start-when-mother-claude-opens)). Then point a client at it:

```bash
curl -sS http://127.0.0.1:5612/v1/messages \
  -H 'anthropic-version: 2023-06-01' \
  -H 'content-type: application/json' \
  -d '{"model":"claude-sonnet-5-5","max_tokens":1024,
       "messages":[{"role":"user","content":"Hello, Claude"}]}'
```

```python
import anthropic
client = anthropic.Anthropic(base_url="http://127.0.0.1:5612", api_key="unused")
print(client.messages.create(model="claude-sonnet-5-5", max_tokens=1024,
      messages=[{"role": "user", "content": "Hello, Claude"}]).content[0].text)
```

```typescript
import Anthropic from '@anthropic-ai/sdk';
const client = new Anthropic({ baseURL: 'http://127.0.0.1:5612', apiKey: 'unused' });
```

In Postman, keep the Claude API's collection and change the base URL. The
`anthropic-version` and `x-api-key` headers may stay: the key is ignored
unless the server was started with one ([Access](#access)) — then it is that
key.

| Endpoint | As in the Claude API | Notes |
|---|---|---|
| `POST /v1/messages` | yes — JSON or `stream: true` (SSE) | Claude Code answers ([what differs](#what-differs)) |
| `POST /v1/messages/count_tokens` | yes | an estimate here: `x-mother-claude-estimate: true` |
| `GET /v1/models` | yes, with an `anthropic-version` header (or `?format=anthropic`) | without it, the bridge's own list (`value`, `displayName`, …) |
| `GET /v1/models/{model_id}` | yes | `created_at` is not known here: the Unix epoch stands in |
| `GET /v1/conversations`, `GET`/`DELETE /v1/conversations/{id}` | Mother Claude's own | [Conversations](#conversations) |

The base URL is the server's own port (`http://127.0.0.1:5612` for the main
one unless another is chosen, its own for others), with the routes at
`/v1/…` and at `/…`. From another computer or through a tunnel, the same
port at the addresses its [Access](#access) opens (`http://<address>:<port>`,
or the tunnel's URL); or the app's TLS address with `/v1`
(`https://<lan-ip>:6725/v1/messages`, Mother Claude's local CA, and a paired
device's token or the server's key). An added server is also at
`/servers/<id>/v1/…` on the main ports ([BRIDGE.md](BRIDGE.md) "Several API
servers").

Every answer carries `request-id` and `x-conversation-id`; an error is the
API's envelope, `{"type":"error","error":{"type","message"},"request_id"}`,
with the API's types and statuses (`invalid_request_error` 400,
`authentication_error` 401, `permission_error` 403, `not_found_error` 404,
`conflict_error` 409, `rate_limit_error` 429, `api_error` 500/503,
`overloaded_error` 529) — plus Mother Claude's own `error.code` when it has
one (`bridge_not_started`, `api_key_required`, …), which an API client
ignores. A request that names the API (its `/messages` paths, or an
`anthropic-version` header) gets this envelope for the bridge's own refusals
too: not started, no key, an unknown path.

---

## Conversations

**A conversation is a Claude Code session**, kept going across requests: it
shows on the dashboard, its transcript is Claude Code's own, and it resumes
after Mother Claude restarts.

**Which one a request goes on in.** A client of the Claude API sends the
whole history every time. Each message gets a fingerprint that survives how
clients send an answer back (SDK objects add `citations: null`, `caller`,
cache marks; thinking blocks may be dropped), and a request whose history
before its last message is one of this client's conversations' continues
it. So an unchanged client keeps one conversation going without knowing it.

- **Named**: `x-conversation-id: <id>` (or `conversation_id` in the body or
  in `metadata`). Then the request may carry **only its next message** — the
  conversation keeps the rest. A named conversation that is answering another
  request is `409 conflict_error`; one the client does not have, `404`.
- **Other settings, same history** (a new system prompt, other tools,
  thinking, output format, effort or `max_tokens` — what a process starts
  with): a **new conversation forked from it** (`--resume --fork-session`),
  the history carried over as it is, the new settings in force. The answer's
  `x-conversation-id` is the fork's.
- **A history no conversation has** (a conversation begun elsewhere, an
  edited earlier message): a new conversation, its first message carrying
  that history, written out as labelled turns (images and documents where
  they were, tool calls and results described, thinking left out). The
  conversation is marked `seeded`.
- **A model change** goes on in the same session (`session.setModel`).
- A conversation idle for 10 minutes has its process closed; the next request
  resumes it from its transcript. At most 8 run at once: past that the one
  used longest ago is closed. A transcript that is gone (deleted, pruned) is
  replaced by a session of the same id with the history written in.

`GET /v1/conversations` lists the caller's (`limit`, `after_id`; the models
list's page shape), `GET /v1/conversations/{id}` one with its messages (the
bytes of images and documents left out), `DELETE /v1/conversations/{id}`
forgets one: its record goes and its process closes, while its transcript
stays on this computer until it is deleted in Mother Claude (an API client is
not the person at the dashboard: Mother Claude deletes transcripts only on
the user's own action, docs/ACCOUNTS.md §14.3).

**Nothing else writes into one.** A conversation of the Claude API is its
client's: the dashboard's composer, a session's `session_send` and anything
else that would add a turn are refused (`409 claude_api_conversation`;
`SessionSummary.control` says so, `claude_api`), since the model would then
hold an exchange the client never saw. It can be read (`session_read`), its
turn interrupted, and it can be stopped.

## Tools

**The client's own tools** (`tools[]` with `input_schema`) are the API's:
the model's call ends the answer with `stop_reason: "tool_use"` and a
`tool_use` block; the client runs the tool and sends a `tool_result` in its
next request, which goes to the very turn that waits for it. Under the hood
the tools are an in-process MCP server of the conversation's session
(`mcp__client__<name>`, the prefix taken off before the client sees a name);
a call becomes a `client_tool` request that never shows as a card. A result
the client never sends is given up after 30 minutes (the turn ends; results
sent later are read as a message).

`tool_choice: none` leaves the tools out; `any` and `tool` cannot force a
call in Claude Code, so the system prompt asks for it and
`x-mother-claude-ignored` names `tool_choice`. Server tools (`web_search_*`,
`code_execution_*`, `bash_*`, `text_editor_*`, computer use, memory, tool
search) run on Anthropic's servers and are refused with a 400 that says so.

A conversation has **no Claude Code tools** — no files, no shell, no MCP
servers but the client's, no settings, hooks, plugins or `CLAUDE.md` — and
the client's system prompt replaces Claude Code's (an empty one is "You are
Claude, an AI assistant made by Anthropic."). It runs in its client's folder
([Clients](#clients)), which it cannot reach.

## What differs

| | Here |
|---|---|
| Input tokens | Claude Code adds several hundred of its own (its environment note, the tools' framing); `usage` counts them. Answers count against the account's Claude plan, like any session. |
| `temperature`, `top_p`, `top_k`, `service_tier`, `speed`, `inference_geo`, `cache_control`, `context_management`, `diagnostics` | accepted, cannot be applied: named in `x-mother-claude-ignored` |
| `stop_sequences` | honoured here: the text is cut where one begins, `stop_reason: "stop_sequence"`, the turn interrupted. The model's own record of that answer may run past the stop. |
| `max_tokens` | the process's output cap (`CLAUDE_CODE_MAX_OUTPUT_TOKENS`); `0` answers an empty message (the API's cache warm-up) |
| `thinking` | `enabled` (`budget_tokens` ≥ 1024, below `max_tokens`), `adaptive` (with `display`), `disabled` (`between_tools` too). Left out, as in the API: adaptive on the Claude 5 models (and Claude Code's aliases, which name them), off on the models before them. Where a model cannot turn thinking off, Claude Code leaves it on rather than fail. |
| `output_config` | `effort` and `format` (`json_schema`) as in the API: a structured answer is a text block of the JSON, `end_turn`. Claude Code writes it through its StructuredOutput tool and checks it against the schema, trying again when it does not match (within the same message), so the JSON comes whole once checked rather than token by token, and words the model writes before it are left out. |
| A last `assistant` message (prefill) | refused (400): Claude Code takes user messages only |
| `container`, `mcp_servers`, Files API `file_id` sources | refused (400) |
| `count_tokens` | an estimate (four bytes of text a token; an image by its size) |
| `models` | what this account can select (`resolvedModel` as `id`); `created_at` the Unix epoch |
| `metadata.user_id` | names the client when the server groups by it |

**Backends.** A server with **Answered by: The Anthropic API**
(`messagesBackend: "anthropic"`), or a request with `x-mother-claude-backend:
anthropic`, sends `/v1/messages` and `count_tokens` to the Anthropic API with
the key Mother Claude was started with (`ANTHROPIC_API_KEY`; the server
refuses to start with it otherwise) — the bridge's pass-through, unchanged
([BRIDGE.md](BRIDGE.md) "The Anthropic API directly"). Before this, that was
what `/messages` did.

---

## Access

Who may call an API server is set when it starts, on the HTTP API page
(**Access**) or the start routes (`POST /api/bridge/start`, `POST
/api/bridge/servers/{id}/start`): from where, and with which key. Code:
`bridge::claude_api::access`, `bridge::claude_api::network`,
`require_bridge_auth` (`bridge/mod.rs`).

**Reachable from** (`access` in the server's defaults):

| | Listens on | Who reaches it |
|---|---|---|
| **This computer** (the default; unset) | `127.0.0.1:<port>` | programs on this computer; a tunnel to the port only with a key or *Open to tunnels* |
| **Local network** (`lan`) | also each private address, `<lan-ip>:<port>` | other computers on your network |
| **Anywhere** (`all`) | also every other address of this computer, public ones included | anyone who can reach one of them — a public address such as `171.254.176.94:5678`, a router's port forward — and tunnels such as ngrok or cloudflared pointed at the port |

Beyond this computer the listeners are **plain HTTP** (no certificate to
trust: a client changes its base URL and nothing else). They are bound per
address, re-read every 30 seconds, so a server started at login before the
network is up is reachable once it is; none while Mother Claude is kept to
loopback (`MOTHER_CLAUDE_HOST`). They serve the server's own routes — never
another server's `/servers/<id>/` — under the ingress `ApiNetwork`: never
local, so never the dangerous right, and no Mother Claude token is read
there (one sent over plain HTTP opens nothing). A client there is its TCP
peer's address. Opening a server beyond this computer is the desktop's
choice; a device may keep it open when it restarts it. `GET /api/bridge`
(and a server's status) lists the addresses in `networkUrls` while it is.

**Port**: an added server's is set under *Manage*; the main server's can be
chosen at its start (`port`, saved in `api-servers.json`) unless
`MOTHER_CLAUDE_BRIDGE_PORT` names one (`portFromEnv` in its status).

**Key** (`key` on the start routes): optional. Blank, or never given: no
authentication. Given: every request to that server must carry it —
`x-api-key: <key>` or `Authorization: Bearer <key>`, what the Claude API's
SDKs send — from this computer too (Mother Claude's own tokens, the
desktop's and paired devices', still work where they always did). Absent
on a later start keeps it; `""` takes it off (from a server open beyond this
computer, the desktop only). A key is 1–256 visible characters; the page
can generate one (`mck-` and 40 letters and digits). It is kept in the
secret store (the Keychain, Windows Credential Manager, a 0600 file
elsewhere), never in a file of ours: `api-servers.json` keeps its last four
characters (`keyHint`), which the status shows. A start whose key cannot be
read back fails (`409 key_unavailable`) — a server meant to take its key
never starts open. Requests without it answer `401 api_key_required`, with
another `401 invalid_api_key`; a caller past its failed-auth budget is
refused (`429`) on every path of an API port. The context middleware leaves
a bearer that is a server's key to the API servers rather than count it as a
failed Mother Claude credential.

**Without a key**, from beyond this computer — the network listeners, or
the loopback port through a proxy or tunnel when the server is reachable
from anywhere — the **Claude API answers** (`/messages`,
`/messages/count_tokens`, `/models`, at `/` and `/v1`): its conversations
have no tools, so nothing out there reaches this computer's files. The rest
of the HTTP API — conversations that run Claude Code with its tools here —
needs the key (`401`), or the desktop's **Open to tunnels**, which takes
every call without a key (dangerous actions still stay with callers on this
computer). A client's conversations (`/v1/conversations`) are read and
deleted from beyond this computer only with a key: without one nothing tells
two callers apart. The app port's `/v1` (TLS) always needs a Mother Claude
token or the server's key.

**ngrok** (or any tunnel): `ngrok http <port>`, the server reachable from
anywhere, and the tunnel's `https://…` URL is the base URL — HTTPS from the
client to ngrok, so a key never travels in the clear. Tunnel detection shows
it on the page (`tunnelUrl`). Kept to this computer, a tunneled request gets
`401 tunneled_request_requires_token` unless it carries the key.

**A public address**: the server reachable from anywhere listens on it as
on any other; give it a key — anyone can find an open port — and prefer a
tunnel or a reverse proxy with TLS in front of it, since requests, answers
and the key otherwise cross the internet unencrypted. On Windows the firewall
rule Windows offers for Mother Claude itself covers the port; one kept to
port 6725 would not. A router's port forward reaches the server once it is
reachable from the local network or anywhere.

With **Tell clients apart by: API key**, each key a client sends is its
client — on a server without a key any key is only a name — told apart by
the first 12 characters of its SHA-256, never kept.

## Clients

**Tell clients apart by** (`groupClientsBy`): `none` (the default — one
folder for every client), `ip` (the address a request comes from, through a
tunnel its `X-Forwarded-For`), `key`, `user` (`metadata.user_id`, else the
IP), `header` (`X-Client-Id`, else the IP).

Each client has a folder under the server's **Clients' folder**
(`apiDataDir`; empty: `<app data>/claude-api`), named after it (letters,
digits, `.-_`; IPv6 as `ip6-…`; Windows device names prefixed): its
`client.json` (label, first and last seen, requests) and
`conversations/<id>.json` (0600, atomic) — the conversation's settings
fingerprint, the history's fingerprints, its usage, and the exchange as the
client saw it. Conversations run with that folder as their working
directory, so their transcripts are grouped per client in Claude Code's
`projects` too. A client sees and continues only its own conversations.

A new folder is chosen from the desktop (or a device with the dangerous
right): it must be an absolute path, outside every Claude configuration
directory; it is made at the start.

## Managing clients

**HTTP API › Clients and conversations** (`admin.rs`): every folder an API
server keeps clients in, each client with its conversations, requests, usage
and state (answering, waiting for tool results, open, idle).

| Route | Who | Does |
|---|---|---|
| `GET /api/bridge/clients` | signed in | `{roots: [{path, default, servers, clients: […]}]}` |
| `GET /api/bridge/clients/{client}?root=` | signed in | its conversations |
| `GET /api/bridge/api-conversations/{id}` | signed in | one, with its messages (`messageList`) |
| `POST /api/bridge/api-conversations/delete` `{ids, transcripts}` | `ManageData` | forgets them (one answering a request is left, `busy`) |
| `DELETE /api/bridge/clients/{client}?root=&transcripts=` | `ManageData` | everything of a client: its conversations, and the files Mother Claude wrote in its folder (the folder goes when that empties it) |

`transcripts: true` also deletes their Claude Code transcripts through the
data delete — the dangerous right, an undo copy for seven days, an audit line
(docs/ACCOUNTS.md §14.3). A conversation can also be opened as a session.

## Start when Mother Claude opens

`POST /api/bridge/autostart {server, on}`: a server with it on starts at
every launch, with the defaults of its last start, closed to tunnels ("Open
to tunnels" is never saved) — its reach and key too. Turning it on is the
desktop's (the port opens unattended); anyone signed in turns it off. Saved
in `api-servers.json`: an added server's entry gets `autostart: true`; the
main server is written as `main: {autostart, defaults, port, keyHint}` with
what applies (its defaults only while it starts at launch). `GET
/api/bridge` and a server's status say `autostart: true` (left out while
off, so the answer reads as before).

---

## How an answer is made

1. The request is read field by field into what Claude Code can run
   (`wire::MessagesRequest::parse`), or refused in the API's words.
2. Its conversation is found or made (above); a new or reopened one is a
   **bare** session in the bridge host: `systemPrompt` the client's,
   `tools: []`, `settingSources: []`, `strictMcpConfig`, the client's tools as
   an SDK MCP server, `canUseTool` allowing those only, `thinking`,
   `effort`, `outputFormat`, `CLAUDE_CODE_MAX_OUTPUT_TOKENS`,
   `includePartialMessages`.
3. The last message becomes a turn (`begin_turn_blocks`: the API's blocks,
   no bridge text limit). Claude Code passes on the API's own stream events
   (`stream_event`): the message id, model, blocks and usage are the API's.
   `stream::Translator` passes them on — a client tool's prefix taken off,
   stop sequences cut, pings Mother Claude's own every 10 seconds — and
   builds the whole message for a request that does not stream.
4. The answer is recorded **before its last event goes out**
   (`message_stop`), so a client that sends its next request at once finds
   its conversation; that request waits (≤ 35 s) while the turn ends. A turn
   that ends on its own is let end; one cut short (a stop sequence,
   `max_tokens`, a refusal) is interrupted; one waiting on client tools
   waits.
5. A client that goes away before the answer is whole: the turn is
   interrupted and a conversation that never answered is forgotten. Errors
   Claude Code reports (signed out, a rate limit, billing) become the API's.

## Security notes

- The Claude API is a **second consumer of the bridge**, not a second server:
  the same listeners, `ClientContext`, token policies, started gate and
  `MountPolicy` (an `accountOnly` server answers `404` for another account's
  conversation, as everywhere).
- A server's key is a credential for that API server only (never a
  principal of the dashboard's routes, never the dangerous right), kept in
  the secret store; a key a client sends to a server without one — an
  Anthropic key, say — is ignored: never logged, stored or forwarded (only a
  hash prefix tells clients apart).
- Beyond this computer, without a key, only the tool-less Claude API
  answers; everything else needs the key or the desktop's *Open to
  tunnels*.
- Conversations have no Claude Code tools, so a client cannot reach this
  computer's files through one; dangerous actions stay where the bridge puts
  them.
- Nothing is written under a Claude configuration directory by our code:
  transcripts are Claude Code's own; our records live in the clients' folders
  (never inside a config directory).
