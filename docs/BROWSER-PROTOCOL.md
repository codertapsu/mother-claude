# Browser extension ⇄ desktop protocol, v1

The wire contract between the Mother Claude browser extension (`extension/`)
and the desktop app (`src-tauri/src/extension/`), fixed before either was
built (docs/PLAN-BROWSER-AND-TEAMS.md §5). Both sides implement exactly this.
A field either side does not know is ignored; a frame `type` either side does
not know is answered with an `error` frame (`code: "unknown_frame"`) and
otherwise ignored, so either side can be newer.

## Transport

- `ws://127.0.0.1:<port>/ws/browser`, the desktop's main port (default 6725;
  the options page lets the user change it). Loopback only: the server refuses
  any other peer, any request through a proxy or tunnel, and any `Origin` that
  is not `chrome-extension://<id>` (`moz-extension://<id>` later). After
  pairing, the extension id is pinned: another id needs pairing of its own.
- Text frames, one JSON object each, UTF-8. At most 8 MiB per frame (a
  screenshot); anything bigger is refused with `error` `frame_too_large`.
- The extension sends `ping` every 20 s (keeps the MV3 service worker alive:
  Chrome 116+ resets the idle timer on WebSocket activity); the server
  answers `pong`. The server drops a connection silent for 60 s.
- Reconnect: exponential backoff 1 s → 30 s, jitter, forever while paired.

## Frames

Every frame has `type`. Requests that expect a reply carry `id` (a string the
sender chooses, unique per connection); the reply carries the same `id`.

### Pairing and authentication

| Direction | Frame | Meaning |
|---|---|---|
| ext → app | `{"type":"pair","id":"…","code":"ABCD-EFGH","name":"Chrome on MacBook","browser":{"name":"Chrome","version":"156.0"},"extensionVersion":"0.1.0"}` | Pair with the 8-character code the desktop shows (Settings › Browser › Pair a browser). Allowed before `hello`. |
| app → ext | `{"type":"paired","id":"…","token":"mcx_…","browserId":"brw_…"}` | The token to keep in `chrome.storage.local`; never shown again. |
| ext → app | `{"type":"hello","id":"…","token":"mcx_…","extensionVersion":"0.1.0","browser":{"name":"Chrome","version":"156.0"},"capabilities":{"debugger":true,"sidePanel":true,"tabGroups":true}}` | Authenticate a connection. Must be the first frame other than `pair`; anything else first closes the connection with `error` `unauthenticated`. |
| app → ext | `{"type":"welcome","id":"…","browserId":"brw_…","protocol":1,"appVersion":"0.8.0","siteModes":{…},"settings":{…}}` | Authenticated. `siteModes` and `settings` as in `state` below. |
| app → ext | `{"type":"error","id":"…","code":"unauthenticated"\|"revoked"\|"bad_code"\|"expired_code"\|"rate_limited"\|…,"message":"…"}` | A refusal. `revoked`: the user removed this browser on the desktop; the extension forgets its token. |

### State the desktop owns

| Direction | Frame | Meaning |
|---|---|---|
| app → ext | `{"type":"state","siteModes":{"https://github.com":"act","*":"off"},"settings":{"screenshots":true,"defaultMode":"off"},"leases":[…],"pending":[…]}` | Sent after `welcome` and whenever any of it changes. Site modes are per origin (`scheme://host[:port]`), `"*"` is the default. Modes: `off` (nothing), `read` (observe only), `act` (observe and act). |
| ext → app | `{"type":"setSiteMode","id":"…","origin":"https://github.com","mode":"read"}` | The user changed a site's mode in the side panel. The desktop validates, records an audit line, answers `ok`, and broadcasts `state`. Raising a sensitive site (banking, payment, identity providers) to `act` is refused here (`code: "needs_desktop"`): that is done on the desktop. |
| app → ext | `{"type":"ok","id":"…"}` | A request without other reply data succeeded. |

### Leases: which session acts in which tabs

A **lease** is one Claude Code session's right to act in the browser. Each
lease owns one tab group (colored, titled after the session). A session acts
only in tabs of its lease; a tab the user adds to the group joins the lease.

| Direction | Frame | Meaning |
|---|---|---|
| app → ext | `{"type":"lease.open","id":"…","lease":"l_…","session":{"id":"<uuid>","title":"Frontend · my-app"},"color":"blue"}` | Create the tab group (empty until a command opens a tab). Reply `{"type":"lease.opened","id":"…","lease":"l_…","groupId":123}`. |
| app → ext | `{"type":"lease.close","id":"…","lease":"l_…","closeTabs":false}` | End it: ungroup (or close) its tabs, detach the debugger. Reply `ok`. |
| ext → app | `{"type":"event","event":"lease.revoked","lease":"l_…","by":"user"}` | The user pressed Stop or closed the group: the desktop fails in-flight commands with `stopped` and refuses new ones for 60 s unless the user resumes. |

### Commands

The desktop sends a command; the extension replies with exactly one `result`.

```json
{"type":"command","id":"c_17","lease":"l_3","tabId":412,"action":"click","params":{"ref":"e14"},"timeoutMs":30000}
{"type":"result","id":"c_17","ok":true,"data":{"snapshot":"…delta…","url":"https://…","title":"…"}}
{"type":"result","id":"c_17","ok":false,"error":{"code":"stale_ref","message":"e14 is gone: take a new snapshot"}}
```

`tabId` is optional: without it the command runs in the lease's active tab.
The extension enforces the site mode again before every action (defence in
depth: the desktop checks first). Error codes: `stale_ref`, `not_found`,
`timeout`, `site_off`, `site_read_only`, `no_tab`, `no_lease`, `stopped`,
`dialog_open`, `navigation`, `debugger_unavailable`, `permission_needed`
(the extension needs an optional permission the user has not granted; `data`
names it), `unsupported`, `internal`.

| `action` | `params` | `data` on success |
|---|---|---|
| `tabs.list` | — | `{"tabs":[{"tabId","url","title","active","inLease"}]}` (lease tabs, plus the active tab when the user granted it) |
| `tabs.open` | `{"url"?,"active"?:true}` | `{"tabId","url","title"}` (opened in the lease's group) |
| `tabs.select` / `tabs.close` | `{"tabId"}` | `{}` |
| `navigate` | `{"url","waitUntil"?:"load"\|"domcontentloaded"\|"networkidle"}` | `{"url","title","status"?}` |
| `back` / `forward` / `reload` | — | `{"url","title"}` |
| `snapshot` | `{"scope"?:"viewport"\|"page","selector"?,"interactiveOnly"?:false,"maxNodes"?:1500}` | `{"snapshot":"<text tree>","url","title","truncated":false,"nodes":n}` |
| `read` | `{"mode":"text"\|"markdown"\|"html","selector"?,"maxChars"?:40000}` | `{"content","url","title","truncated"}` |
| `find` | `{"text"?,"role"?,"name"?,"selector"?,"limit"?:20}` | `{"matches":[{"ref","role","name","text"}]}` |
| `click` / `hover` / `dblclick` | `{"ref"\|"selector","button"?:"left","trusted"?:false}` | `{"snapshot"?}` (a fresh snapshot of what changed, when cheap) |
| `type` | `{"ref","text","clear"?:false,"submit"?:false}` | `{}` |
| `fill` | `{"fields":[{"ref","value"}],"submit"?:false}` | `{"filled":n}` |
| `select` | `{"ref","values":["…"]}` | `{}` |
| `check` | `{"ref","checked":true}` | `{}` |
| `press` | `{"key":"Enter","modifiers"?:["Shift"]}` | `{}` |
| `scroll` | `{"ref"?,"direction"?:"down","amount"?:600}` | `{}` |
| `wait` | `{"for":"selector"\|"text"\|"url"\|"networkidle"\|"time","value"?,"timeoutMs"?}` | `{"waitedMs"}` |
| `screenshot` | `{"scope"?:"viewport"\|"element","ref"?,"format"?:"png"}` | `{"data":"<base64>","mimeType":"image/png","width","height"}` (needs `debugger` or `tabs.captureVisibleTab`) |
| `eval` | `{"expression","awaitPromise"?:true}` | `{"value"}` (JSON-serializable; the desktop gates this as dangerous) |
| `console` | `{"since"?:<ms>,"level"?}` | `{"messages":[{"level","text","at"}]}` (needs `debugger`) |
| `dialog` | `{"accept":true,"text"?}` | `{}` |
| `upload` | `{"ref","files":[{"name","mimeType","data":"<base64>"}]}` | `{}` (the desktop reads the files; the extension never reads the disk) |
| `highlight` | `{"ref","ms"?:800}` | `{}` (shows the user what is about to be touched) |

### Snapshot format

A compact text tree of the page's accessibility roles, one node per line,
indented two spaces per level, interactive nodes carrying a `ref` that stays
valid until the next `snapshot` or navigation in that tab:

```text
- banner
  - link "Mother Claude" [ref=e1]
  - navigation "Main"
    - link "Issues" [ref=e2]
- main
  - heading "New issue" [level=1]
  - textbox "Title" [ref=e3] value=""
  - textbox "Password" [ref=e4] value=«redacted»
  - button "Submit new issue" [ref=e5] [disabled]
  - iframe "Captcha" (cross-origin, not read)
```

Rules: names are the accessible name (truncated to 120 characters); `value=`
only for form controls; the value of a password field, of a field whose
name, id or autocomplete says secret (`password`, `cc-number`, `cc-csc`,
`one-time-code`, `ssn`, `api key`, `token`, `secret`), and of anything in a
`[data-mc-redact]` subtree is `«redacted»`; shadow DOM is included;
same-origin iframes are included with their own refs (`f1e3`); cross-origin
iframes appear as one line; `[ref=…]` only on elements a user could act on.

### Events the extension reports

`{"type":"event","event":"…",…}` with no reply:

| `event` | Fields | Meaning |
|---|---|---|
| `tab.updated` | `tabId, lease?, url, title, status` | A lease tab navigated or finished loading |
| `tab.closed` | `tabId, lease?` | A lease tab closed |
| `dialog.opened` | `tabId, lease?, kind: alert\|confirm\|prompt\|beforeunload, message` | A JavaScript dialog blocks the tab until `dialog` |
| `download.started` | `tabId?, lease?, url, filename` | A download began in a lease tab |
| `lease.revoked` | `lease, by: user` | Stop pressed, or the group closed |
| `permission.changed` | `granted: ["debugger", "https://github.com/*", …]` | The user granted or removed optional permissions |

### Approvals in the side panel

Dangerous browser actions are pending items in the desktop's approval store
(`crate::pending`), answered on the desktop, on a phone with the grant, or in
the side panel — the first answer wins.

| Direction | Frame | Meaning |
|---|---|---|
| app → ext | (inside `state`) `"pending":[{"id":"…","lease":"l_…","session":"…","summary":"Submit the form on github.com","detail":"…","dangerous":true,"createdAt":…}]` | What waits |
| ext → app | `{"type":"answer","id":"…","pending":"<pending id>","allow":true}` | The user's answer from the side panel. The desktop answers `ok`, or `error` `already_answered`. |

## Limits and rules both sides keep

- One command at a time per tab; commands for different tabs may overlap.
  The extension queues per tab, in order.
- The desktop's `timeoutMs` is the deadline; the extension aborts the action
  and replies `timeout` itself 500 ms before it, so the desktop always gets
  a `result`.
- Nothing the extension sends contains cookies, storage, or the value of a
  redacted field. The desktop redacts again before anything reaches a model.
- The extension never acts outside a lease's tabs, except in the active tab
  of a window when the user granted it for that tab (`activeTab`).
