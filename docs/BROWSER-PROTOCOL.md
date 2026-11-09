# Browser extension ⇄ desktop protocol, v1

The wire contract between the Mother Claude browser extension (`extension/`)
and the desktop app (`src-tauri/src/extension/`), fixed before either was
built (docs/PLAN-BROWSER-AND-TEAMS.md §5). Both sides implement exactly this.
A field either side does not know is ignored; a frame `type` either side does
not know is answered with an `error` frame (`code: "unknown_frame"`) and
otherwise ignored, so either side can be newer. Additions made since v1 was
fixed are all optional and degrade gracefully; they are marked *(added)* and
listed in [Changes since v1 was fixed](#changes-since-v1-was-fixed).

## Transport

- `ws://127.0.0.1:<port>/ws/browser`, the desktop's main port (default 6725;
  the options page lets the user change it). Loopback only: the server refuses
  any other peer, any request through a proxy or tunnel, and any `Origin` that
  is not `chrome-extension://<id>` (`moz-extension://<id>` later). After
  pairing, the extension id is pinned: another id needs pairing of its own.
- The upgrade itself is refused with HTTP 403 and `{"error":{"code","message"}}`
  *(added)*: `not_local` (not this machine, or a proxy/tunnel header),
  `origin_not_allowed` (no `Origin: chrome-extension://<32 letters a–p>`),
  `tunnel_exposed` (a raw TCP tunnel points at the port, so this machine
  cannot be told from the internet). The LAN and tunnel listeners do not serve
  `/ws/browser` at all.
- Text frames, one JSON object each, UTF-8. At most 8 MiB per frame (a
  screenshot); anything bigger is refused with `error` `frame_too_large`.
  *(added)* When the oversized frame answers a request (its `id` is found in
  its first 512 bytes), that request fails at once with `frame_too_large`;
  past 16 MiB the socket simply closes. A binary frame or text that is not one
  JSON object is answered `error` `bad_frame`.
- The extension sends `ping` every 20 s (keeps the MV3 service worker alive:
  Chrome 116+ resets the idle timer on WebSocket activity); the server
  answers `pong` (with the ping's `id`, if it had one). *(added)* `ping` is
  answered before `hello` too, so a connection can wait for the user to type
  a pairing code. The server drops a connection silent for 60 s (`error`
  `timeout`, then close).
- One connection per paired browser *(added)*: when a newer connection of the
  same browser says `hello`, the older one gets `error` `replaced` and is
  closed (don't reconnect from it while the newer one is open). At most 8
  connections may wait unauthenticated; beyond that the oldest gets `error`
  `too_many_connections`.
- Reconnect: exponential backoff 1 s → 30 s, jitter, forever while paired.

## Frames

Every frame has `type`. Requests that expect a reply carry `id` (a string the
sender chooses, unique per connection); the reply carries the same `id`.

### Pairing and authentication

| Direction | Frame | Meaning |
|---|---|---|
| ext → app | `{"type":"pair","id":"…","code":"ABCD-EFGH","name":"Chrome on MacBook","browser":{"name":"Chrome","version":"156.0"},"extensionVersion":"0.1.0"}` | Pair with the 8-character code the desktop shows (Browser › Pair a browser). Allowed before `hello`. |
| app → ext | `{"type":"paired","id":"…","token":"mcx_…","browserId":"brw_…"}` | The token to keep in `chrome.storage.local`; never shown again. |
| ext → app | `{"type":"hello","id":"…","token":"mcx_…","extensionVersion":"0.1.0","browser":{"name":"Chrome","version":"156.0"},"capabilities":{"debugger":true,"sidePanel":true,"tabGroups":true}}` | Authenticate a connection. Must be the first frame other than `pair`; anything else first closes the connection with `error` `unauthenticated`. |
| app → ext | `{"type":"welcome","id":"…","browserId":"brw_…","protocol":1,"appVersion":"0.8.0","siteModes":{…},"settings":{…}}` | Authenticated. `siteModes` and `settings` as in `state` below. |
| app → ext | `{"type":"error","id":"…","code":"unauthenticated"\|"revoked"\|"bad_code"\|"expired_code"\|"rate_limited"\|…,"message":"…"}` | A refusal. `revoked`: the user removed this browser on the desktop; the extension forgets its token. |

Pairing and `hello`, as the desktop implements them *(added detail)*:

- The code is 8 Crockford base32 characters, shown `ABCD-EFGH`; case, spaces
  and dashes don't matter, `O` reads as `0` and `I`/`L` as `1`. It lives 5
  minutes and works once. `bad_code`: not a code the desktop issued;
  `expired_code`: expired or already used; `rate_limited`: 5 wrong codes a
  minute block pairing for a minute (doubling, up to 15 minutes).
- The pairing pins the connection's `Origin`. After `paired` the connection
  is still unauthenticated: send `hello` with the new token on it (or on a
  new connection).
- `hello` refusals close the connection: `unauthenticated` (an unknown token,
  or a token paired by another extension id), `revoked`. Anything but
  `pair`, `ping` and `hello` before `hello` is answered `unauthenticated` and
  closes it. A second `hello` on an authenticated connection is answered
  `welcome` (then `state`) again when its token is the same browser's;
  another token closes it.

### State the desktop owns

| Direction | Frame | Meaning |
|---|---|---|
| app → ext | `{"type":"state","siteModes":{"https://github.com":"act","*":"off"},"settings":{"screenshots":true,"defaultMode":"off"},"leases":[…],"pending":[…]}` | Sent after `welcome` and whenever any of it changes. Site modes are per origin (`scheme://host[:port]`), `"*"` is the default. Modes: `off` (nothing), `read` (observe only), `act` (observe and act). |
| ext → app | `{"type":"setSiteMode","id":"…","origin":"https://github.com","mode":"read"}` | The user changed a site's mode in the side panel. The desktop validates, records an audit line, answers `ok`, and broadcasts `state`. Raising a sensitive site (banking, payment, identity providers) to `act` is refused here (`code: "needs_desktop"`): that is done on the desktop. |
| app → ext | `{"type":"ok","id":"…"}` | A request without other reply data succeeded. |

The shapes, as the desktop sends them:

- `leases`: `[{"lease":"l_…","session":{"id":"<uuid>","title":"…"},"color":"blue","groupId":123}]`
  — the leases in this browser (`groupId` as `lease.opened` reported it).
- `pending`: as in [Approvals in the side panel](#approvals-in-the-side-panel),
  the approvals waiting for this browser's leases.
- `settings` *(added)*: also `"sensitiveDomains":["paypal.com","chase.com",
  "accounts.google.com",…]`, the built-in sensitive list (domain suffixes,
  `paypal.com` covers `www.paypal.com`), so the side panel can mark those
  sites and not offer `act` for them.
- `siteModes` keys *(added)*: besides origins and `"*"`, a wildcard
  `scheme://*.domain[:port]` covers every subdomain of `domain` (not `domain`
  itself). A site's mode is its own entry, else the longest wildcard that
  covers it, else `"*"` — except a sensitive site, which is `off` unless it
  has its own entry (neither `"*"` nor a wildcard opens it). An extension that
  does not know wildcards falls back to `"*"` for those sites: stricter,
  never looser, and the desktop checks first anyway. Default ports are
  dropped (`https://github.com:443` is `https://github.com`); hosts are lower
  case, IDN in punycode.

`setSiteMode` details *(added)*: `origin` may be any URL (its origin is
taken), `"*"` or a wildcard; `"mode": null` removes the site's own entry (it
follows the others again). Refusals: `invalid` (a key or mode that does not
parse) and `needs_desktop` for `act` on a sensitive site, on `"*"` or on a
wildcard. Raising a site to `act` from the side panel also shows a notice on
the desktop.

### Leases: which session acts in which tabs

A **lease** is one Claude Code session's right to act in the browser. Each
lease owns one tab group (colored, titled after the session). A session acts
only in tabs of its lease; a tab the user adds to the group joins the lease.

| Direction | Frame | Meaning |
|---|---|---|
| app → ext | `{"type":"lease.open","id":"…","lease":"l_…","session":{"id":"<uuid>","title":"Frontend · my-app"},"color":"blue"}` | Create the tab group (empty until a command opens a tab). Reply `{"type":"lease.opened","id":"…","lease":"l_…","groupId":123}`. |
| app → ext | `{"type":"lease.close","id":"…","lease":"l_…","closeTabs":false}` | End it: ungroup (or close) its tabs, detach the debugger. Reply `ok`. |
| ext → app | `{"type":"event","event":"lease.revoked","lease":"l_…","by":"user"}` | The user pressed Stop or closed the group: the desktop fails in-flight commands with `stopped` and refuses new ones for 60 s unless the user resumes. |

How the desktop uses leases *(added detail)*: it opens one on a session's
first command (and waits 10 s for `lease.opened`; an `error` reply fails
that command), in the pinned browser or else the one the extension used most
recently, colours chosen so the leases of one browser differ. A lease stays
however long its session goes without a command, ended sessions' too (a
session continued later keeps its tabs). The desktop sends `lease.close`
(`closeTabs: false`) when the user stops the session from the desktop,
connects it in another browser, or removes the browser; the reply is not
awaited for long. A `hello` with `freshBrowser: true` ends that browser's
leases from before, with no `lease.close` (the browser that held them is
gone), so the `state` that follows names none of them. A command whose
lease's browser is not connected waits up to 10 s for it to come back
before the session gets a new lease elsewhere. On `lease.revoked`
the desktop fails that lease's in-flight requests itself (a `result` with
`error.code` `stopped`: the extension needn't answer them any more), refuses
the session's commands for 60 s unless the user resumes it on the desktop,
withdraws the session's approvals, and asks the user's approval for any other
session's action in that browser during that minute. The user resumes it on
the desktop, or *(added)* in the side panel: the extension then sends the
event `lease.resumed` (below).

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
names it), `unsupported`, `internal`; *(added)* `secret_field` (see
`refuseSecret` below).

What the desktop does around commands *(added detail)*:

- It waits for the `result` until `timeoutMs` + 1.5 s, then fails the call
  itself; a late `result` is ignored. `timeoutMs` is 30 000 unless a tool
  asks for more (at most 120 000).
- `no_lease`: it reopens the lease once (`lease.open`) and resends the
  command. `navigate` answered `no_tab`: it sends `tabs.open {url, active:
  true}` instead.
- When it does not know which page a tab shows, it sends `tabs.list` first
  (and learns `url`, `title`, `active` from it).
- `snapshot` and `read` must report `url`: content from a page the desktop
  cannot place is refused. Any result whose `url` is on a site the session
  may not read is dropped before it reaches the model.
- *(added)* An action's result (`click`, `dblclick`, `hover`, `press`, …)
  may carry `url` (and `title`): the page it ended on. A `snapshot` of what
  changed is kept only when the result carries a `url` the session may read —
  without one (the page may have navigated anywhere) the desktop drops it.
  It may also carry `navigated: true` (a full navigation, no delta),
  `dialog: {kind, message}` (the action opened a JavaScript dialog; the result
  comes at once) and `newTab: {tabId, url, title?}` (a `target=_blank` link,
  opened in the lease's group) or `newTab: {url, refused: "site_off"}`. The
  desktop learns the page, the dialog and the tab from them; of a site the
  session may not read it passes on only the origin — no title, no snapshot,
  no dialog message.
- *(added)* `press` carries `refuseSecret: true` too (it types into the
  focused field). The extension refuses secret fields even without the flag
  unless a command carries `allowSecret: true`, which the desktop never sends.
- *(added)* `console`'s `level` is the least serious level to include:
  `debug` < `log` < `info` < `warning` < `error`.
- *(added)* An `eval` that throws is `internal` with `data.exception`; the
  desktop passes the exception's text on to the model (redacted, cut at 300
  characters).
- Before asking the user to approve an action on a `ref` it sends
  `highlight {ref, ms: 1500}` (best effort, not awaited).
- *(added)* `type` and `fill` carry `"refuseSecret": true`: the extension
  refuses with `secret_field` to type into a password field or any field
  whose value it would redact (the snapshot rules below). The desktop refuses
  those refs itself from the last snapshot; this covers refs it never saw.
- The desktop redacts every text again (snapshots, page text, `find` texts,
  console lines, `eval` values) before the model sees it.

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
| `drag` *(added)* | `{"ref"\|"selector","toRef"\|"toSelector","trusted"?:false}` | `{"html5","dropped"?,"snapshot"?,"url","title"}` (HTML drag-and-drop events when the source is draggable, else pointer events; `trusted`: the real mouse, needs `debugger`) |
| `network` *(added)* | `{"filter"?,"type"?,"failed"?,"since"?,"limit"?:50}` · `{"requestId","body"?:false}` · `{"clear":true}` | a list `{"requests":[{"id","method","url","type","at","status"?,"mimeType"?,"size"?,"ms"?,"failed"?,"fromCache"?,"redirects"?,"frames"?}],"total","recordingSince","note"?}`; one request `{"request":{… ,"requestHeaders","responseHeaders","postData"?,"frames"?},"body"?:{"text"\|"base64","size","truncated"?}\|{"unavailable"}}` (needs `debugger`; the last 500 requests per tab, across page loads) |
| `cookies` *(added)* | `{"name"?,"values"?:false}` | `{"url","cookies":[{"name","domain","path","size","httpOnly","secure","sameSite"?,"expires"?,"session"?,"value"?}]}` (the cookies sent to the page's URL; `value` only with `values`; needs `debugger`) |
| `cookies.set` / `cookies.delete` *(added)* | `{"name","value"?,"domain"?,"path"?,"secure"?,"httpOnly"?,"sameSite"?,"expires"?}` | `{"name"}` (the page's own site only: a `domain` it is not under is `unsupported`) |
| `storage` *(added)* | `{"area":"local"\|"session","key"?,"values"?:false}` · `{"area":"indexeddb","db"?,"store"?,"limit"?:20,"values"?}` | `{"area","origin","count","entries":[{"key","size","value"?}],"truncated"?}`; IndexedDB: `{"databases":[{"name","version"}]}`, `{"db","version","stores":[{"name","keyPath","autoIncrement","indexes","count"}]}` or `{"db","store","count","records":[{"key","size","value"?}]}` (read only: a database is never created or upgraded) |
| `storage.set` / `storage.remove` / `storage.clear` *(added)* | `{"area","key"?,"value"?}` | `{"area","key"?,…}` (local and session storage) |

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
| `dialog.opened` | `tabId, lease?, kind: alert\|confirm\|prompt\|beforeunload, message` | A JavaScript dialog blocks the tab until `dialog` (the desktop tells the model, `message` redacted and cut at 300 characters) |
| `download.started` | `tabId?, lease?, url, filename` | A download began in a lease tab |
| `lease.revoked` | `lease, by: user` | Stop pressed, or the group closed |
| `lease.resumed` | `lease, by: user` | *(added)* Resume pressed in the side panel for a stopped lease: the desktop lifts that Stop if it holds in this browser, audits it and says so on the desktop |
| `permission.changed` | `granted: ["debugger", "https://github.com/*", …]` | The user granted or removed optional permissions |

A `tab.updated` with a new `url` makes the desktop forget that tab's refs.
Events for a tab the desktop cannot place in a lease are ignored, as are
events it does not know.

### Approvals in the side panel

Dangerous browser actions are pending items in the desktop's approval store
(`crate::pending`), answered on the desktop, on a phone with the grant, or in
the side panel — the first answer wins.

| Direction | Frame | Meaning |
|---|---|---|
| app → ext | (inside `state`) `"pending":[{"id":"…","lease":"l_…","session":"…","summary":"Submit the form on github.com","detail":"…","dangerous":true,"createdAt":…}]` | What waits |
| ext → app | `{"type":"answer","id":"…","pending":"<pending id>","allow":true}` | The user's answer from the side panel. The desktop answers `ok`, or `error` `already_answered`. |

`allow: false` (or no `allow`) denies. *(added)* Besides `already_answered`
(someone answered first), the desktop may answer `error` `not_found` (no
such approval waits in this browser) or `refused`. Only approvals of this
browser's leases can be answered here, and every approval allowed in the
side panel shows a notice on the desktop (the extension's token is stored in
the browser profile, so its approvals stay visible).

## Limits and rules both sides keep

- One command at a time per tab; commands for different tabs may overlap.
  The extension queues per tab, in order.
- The desktop's `timeoutMs` is the deadline; the extension aborts the action
  and replies `timeout` itself 500 ms before it, so the desktop always gets
  a `result`.
- Nothing the extension sends contains the value of a redacted field. Cookie
  and storage values go only in answer to a `cookies` or `storage` command
  with `values: true`, which the desktop sends only after the user approved
  it (or a standing `secrets` grant covers the site). A `network` request's
  headers go as the browser sent them (cookies and `Authorization`
  included): the desktop hides every credential header whole before a model
  sees them, and redacts every text again (bodies, values, URLs).
- Classes: `network`, `cookies` and `storage` observe (a site in `read` is
  enough); `cookies.set`, `cookies.delete`, `storage.set`, `storage.remove`,
  `storage.clear` and `drag` act. Reading values and every change to cookies
  or storage need the user's approval (`secrets`).
- The extension never acts outside a lease's tabs, except in the active tab
  of a window when the user granted it for that tab (`activeTab`).

## Error codes the desktop sends

In `error` frames (`code`, `message`, and the `id` of the frame it answers
when there was one): `unknown_frame`, `bad_frame`, `frame_too_large`,
`unauthenticated`, `revoked`, `replaced`, `too_many_connections`, `timeout`
(60 s of silence), `bad_code`, `expired_code`, `rate_limited`,
`needs_desktop`, `invalid`, `already_answered`, `not_found`, `refused`,
`internal`. Treat an unknown code as a refusal of that one frame.

## Changes since v1 was fixed

All optional; an extension built to the first text keeps working.

1. `ping` is answered before `hello`.
2. The upgrade's HTTP 403 refusals and their codes (`not_local`,
   `origin_not_allowed`, `tunnel_exposed`).
3. `error` codes `replaced` (a newer connection of the same browser took
   over), `too_many_connections`, `timeout`, `bad_frame`, `invalid`,
   `not_found`, `refused`, `internal`; an oversized frame fails the request
   it answers.
4. `state.settings.sensitiveDomains`.
5. Wildcard keys `scheme://*.domain[:port]` in `siteModes` (and in
   `setSiteMode`).
6. `setSiteMode` with `"mode": null` removes a site's own entry.
7. `refuseSecret: true` on `type` and `fill`, and the result error code
   `secret_field`.
8. `url` (and `title`) on an action's result; without it the action's
   `snapshot` delta is dropped. Also `navigated`, `dialog` and `newTab` on
   it; of a site the session may not read only the origin goes on.
9. The event `lease.resumed` (Resume in the side panel); `refuseSecret` on
   `press`; `allowSecret` is the extension's own flag, never sent by the
   desktop; `hello` may also carry `granted` (as `permission.changed` does);
   `console` levels; `eval`'s `data.exception`.
10. The shapes of `state.leases`, and the desktop's behaviour around leases,
   commands and events written down (deadlines, `no_lease` and `no_tab`
   retries, `tabs.list` when a page is unknown, `highlight` before an
   approval, redaction).

From the extension's side (it implements 1–9):

10. The event `lease.resumed` (`lease`, `by: "user"`) after Resume in the side
    panel; the desktop lifts the Stop of that lease in that browser only. A
    desktop that does not know it keeps its own 60 s Stop.
11. `hello` carries `granted` (the optional permissions held) and more
    `capabilities` flags; `permission_needed` names the permission in both
    `error.data` and `data` (`{"permission": "…"}`).
12. An action's result may carry `navigated` (the whole page changed),
    `dialog` (a JavaScript dialog opened) or `newTab` (a link meant for a new
    tab opened inside the session's group); the desktop reads `navigated`
    and `dialog`.
13. A new lease's group holds a placeholder page (Chrome has no empty tab
    groups); commands that need a page answer `no_tab` until the first tab
    opens.
14. `allowSecret: true` on `type` and `fill` would let the extension type into
    a secret field the user allowed on the desktop. Reserved: the desktop
    never sends it in v1 (secret fields are refused outright).

Connecting a session from the side panel (both sides):

15. `ok` may carry `data`, an object: what the request asked for.
16. ext → app `{"type":"sessions","id"}` → `ok` with `data: {sessions:
    [{id, name, project, cwd, surface, status, lastActivityAt, owned,
    canTell, tellHow: "direct"|"relay"|null, whyNot, hasTools, lease}],
    toolsOn}`: the live sessions, the most recently active first (50 at
    most).
17. ext → app `{"type":"session.connect","id","session","tab":{"tabId","url"}}`
    → the desktop sends `lease.open` for that session (or keeps its lease
    here; one in another browser closes), then `ok` with `data: {lease,
    title, color, hasTools}`. The extension then moves the tab into the
    lease's group as its active tab and closes an untouched placeholder.
    Errors: `invalid`, `not_found` (not running), `unavailable` (no group).
18. ext → app `{"type":"session.tell","id","session","message","tab"?:
    {"tabId","url"}}` → `ok` with `data: {delivery: "sent"|"relayed"|
    "queued"}`. The message (at most 4,000 characters) reaches the session
    with the tab's origin only. Errors: `invalid`, `not_found`,
    `not_connected` (no lease in this browser), `needs_desktop` (it runs
    without permission prompts), `no_channel` (nothing reaches it).

Network, cookies, storage and drag (both sides):

19. The commands `network`, `cookies`, `cookies.set`, `cookies.delete`,
    `storage`, `storage.set`, `storage.remove`, `storage.clear` and `drag`
    (the table above). An extension that does not know one answers
    `unsupported`. The debugger keeps the browser's response bodies
    (`Network.enable` with buffers of 20 MiB in all, 5 MiB a resource), so a
    body can be read on demand while the browser still has it; a body of a
    response still loading is `{"unavailable"}` after a wait of up to 3 s.
20. The standing grant `secrets` ("read or change cookies and site storage")
    covers reading values and changing cookies or storage on one origin.

A closed browser (both sides):

21. `hello` may carry `freshBrowser: true`: the browser was closed (or the
    extension reloaded) since the desktop last welcomed this one — Chrome
    empties `chrome.storage.session` then, never when only the service
    worker restarts. The extension keeps the flag there until a `welcome`.
    The desktop ends that browser's leases from before (above). A desktop
    that does not know it keeps them until it quits or the user stops them.
