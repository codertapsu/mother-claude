# Mother Claude browser extension

An MV3 extension for Chrome, Edge and Brave that lets Claude Code sessions read
and act in the user's real, signed-in browser, through the Mother Claude
desktop app. It connects to the app over loopback only
(`ws://127.0.0.1:<port>/ws/browser`) and speaks the protocol in
[docs/BROWSER-PROTOCOL.md](../docs/BROWSER-PROTOCOL.md). The design is in
[docs/PLAN-BROWSER-AND-TEAMS.md](../docs/PLAN-BROWSER-AND-TEAMS.md) §5; the
user's guide is [docs/BROWSER-EXTENSION.md](../docs/BROWSER-EXTENSION.md).

Everything is run from the repository root.

| Command                     | What it does                                                                                               |
| --------------------------- | ---------------------------------------------------------------------------------------------------------- |
| `npm run ext:build`         | Chrome/Edge/Brave build → `dist-extension/chrome/` and `dist-extension/mother-claude-chrome-<version>.zip` |
| `npm run ext:build:firefox` | Firefox build → `dist-extension/firefox/` and its zip (see [Firefox](#firefox))                            |
| `npm run ext:watch`         | Rebuild `dist-extension/chrome/` on every change (inline source maps)                                      |
| `npm run ext:lint`          | `tsc` (sources and tests), ESLint (type-aware), Prettier                                                   |
| `npm run ext:test`          | Unit tests: Node's test runner + happy-dom                                                                 |
| `npm run ext:e2e`           | End to end in the installed Chrome against a mock desktop (below)                                          |

## Load it unpacked

1. `npm run ext:build`.
2. Chrome: open `chrome://extensions`, turn on **Developer mode**, press
   **Load unpacked** and pick `dist-extension/chrome`. Edge: `edge://extensions`,
   **Developer mode**, **Load unpacked**. Brave: `brave://extensions`, the same.
3. Pin it (the puzzle-piece menu) and click its icon: the side panel opens.
4. In Mother Claude: **Browser › Pair a browser** shows an 8-character
   code. Type it in the side panel (or on the options page) and press **Pair**.

After a rebuild, press the reload arrow on the extension's card in
`chrome://extensions`. The pairing survives reloads (it is in
`chrome.storage.local`); a removed and re-added extension pairs again.

## The extension id

The development id is **`bagfjjgeehalpbpjjajhlghbehfmfclf`**, and the desktop
pins it (it accepts only `Origin: chrome-extension://bagfjjgeehalpbpjjajhlghbehfmfclf`
until another id is paired). It follows from the public key in
[`manifest.json`](manifest.json) (`key`): Chrome takes the SHA-256 of that
DER-encoded SubjectPublicKeyInfo, keeps the first 16 bytes and writes their 32
hex digits as the letters a–p. `node extension/scripts/key.mjs` prints the id
of the key in the manifest.

Only the public key is in the repository. Loading the unpacked build never
needs the private key; packing a `.crx` with the same id would. To rotate the
key: `node extension/scripts/key.mjs generate <path outside the repo>.pem`,
put the printed `key` in `manifest.json`, and tell the desktop the new id. A
Chrome Web Store or Edge Add-ons listing gets its own id from the store
(copying the store's public key into `key` makes the development build match it).

## How the code is laid out

```
manifest.json            the Chrome manifest (build.mjs derives the other variants)
build.mjs                esbuild bundles, manifest variants, icons, zip
scripts/key.mjs          the extension id from the key; new key pairs
scripts/png.mjs, zip.mjs icons from public/icons (the app's artwork); a reproducible zip
src/shared/              pure logic, unit-tested: protocol codec, site modes, queue + deadline, backoff, keys
src/background/          the service worker
  index.ts               every chrome.* listener, registered at top level; desktop frames; UI requests
  connection.ts          the WebSocket: pair, hello, ping every 20 s, reconnect 1 → 30 s with jitter
  router.ts              one command at a time per tab, the deadline rule, lease/site/dialog checks, Stop
  commands.ts            each action
  leases.ts              lease ⇄ tab group, the placeholder, re-attaching after a restart
  page.ts                injecting and calling the content script; the page hook
  debugger.ts            chrome.debugger: trusted input, screenshots, console, dialogs, network idle
  state.ts               what survives a worker restart (storage.local / storage.session)
src/content/             the page script (isolated world, injected on demand, one file)
  snapshot.ts, roles.ts, accname.ts, refs.ts, redact.ts   the accessibility snapshot
  read.ts, find.ts, actions.ts, wait.ts, delta.ts         read, find, act, wait, "what changed"
  hook.ts                the MAIN-world hook: dialogs before they block, open fetch/XHR count
src/ui/                  side panel, options page, the lease placeholder page (plain TS + one CSS file)
test/unit/               node:test + happy-dom
test/e2e/                Chrome over the DevTools pipe, a mock desktop, fixture pages
```

Notes for changing it:

- **The service worker stops when idle.** Keep every listener at the top level
  of `background/index.ts`; anything that must survive goes through `Store`
  (`chrome.storage.session` for leases and per-tab ref counters, `local` for
  the pairing and settings). Chrome keeps the worker alive while the socket
  talks (the 20 s ping).
- **Content code is realm-safe.** Elements of a same-origin iframe belong to
  another realm: use the helpers in `content/dom.ts` (no global `instanceof`,
  constructors from `windowOf(el)`).
- **Functions passed to `chrome.scripting.executeScript` as `func`** (the page
  hook, the content call) are serialized: keep them self-contained.
- **Refs** are stable per element and never reused in a tab: the worker hands
  each new document the tab's next number, so a ref from before a navigation
  can only come back `stale_ref`.

## Tests

`npm run ext:test` runs the unit tests (75, about 2 seconds): snapshot
format and the protocol's own example, accessible names, roles, refs,
redaction, Markdown/text/HTML reading, find, the actions (including a
React-style value tracker), the codec, site modes (wildcards, sensitive sites),
the queue and deadline, backoff and keys. happy-dom has no layout, so nothing
there depends on boxes.

`npm run ext:e2e` builds `dist-extension/chrome-e2e/` and drives it in the
installed Google Chrome:

- Chrome is started headless with a throwaway `--user-data-dir`,
  `--use-mock-keychain` and `--password-store=basic` (never the user's profile
  or Keychain), and `--remote-debugging-pipe`. Branded Chrome 137+ ignores
  `--load-extension`, so the build is loaded with the DevTools command
  `Extensions.loadUnpacked`, which needs the pipe and
  `--enable-unsafe-extension-debugging`.
- The **e2e build** differs from the real one in two ways only: the fixture
  hosts (`127.0.0.1`, `localhost`) and the optional APIs (`webNavigation`,
  `downloads`) are required permissions, because optional ones can only be
  granted through a browser prompt automation cannot click; and its default
  port is 1, so nothing can reach a real app on 6725 before the test points it
  at the mock. The debugger is required in every build (Chrome refuses it as
  an optional permission) and the suite grants it on the options page, as a
  user does.
- `test/e2e/mock-desktop.ts` plays the desktop (pairing, hello, state, leases,
  commands, approvals, site modes) on an ephemeral port and refuses any Origin
  but the pinned id. `test/e2e/fixtures.ts` serves the fixture pages on
  ephemeral ports: forms with a React-style controlled input, an SPA, dialogs,
  open and closed shadow roots, same- and cross-origin iframes, a page that
  ignores synthetic clicks, uploads, downloads, a slow network, a "typical"
  issue list and a 3000-row page.
- It writes `dist-extension/e2e-report.json`: snapshot sizes per fixture,
  command round trips (p50/p95), the deadline and Stop timings.

`MC_E2E_BROWSER=/path/to/binary` runs it against another Chromium (Brave,
Edge, Chrome for Testing); `MC_E2E_HEADFUL=1` shows the window. All 37 tests
pass on macOS in Google Chrome 156.0.8078.80 and in Brave 1.99.55 (Chromium
157); Edge has not been run.

The side panel's **Work here with a session** lists the Claude Code sessions
running on the computer (`sessions`), connects one to the tab you are on
(`session.connect`, then the tab joins the lease's group) and tells it what
to do (`session.tell`): docs/BROWSER-EXTENSION.md has the user's view,
docs/BROWSER-PROTOCOL.md (15–18) the frames.

`test/e2e/desktop.e2e.ts` runs the same build against the **real desktop**
instead of the mock: the Rust server's `/ws/browser`, `/api/browser/*` and
`/mcp`, as the UI fixture serves them on a spare port over a synthetic home
(never the app you run). It pairs with a code the desktop made, calls the
`browser_*` tools over `/mcp` as a session (this process, named by a pidfile
in the fixture's home), answers approvals through the desktop's API and
stops the session. It is skipped unless `MC_E2E_DESKTOP` names the fixture's
info file:

```sh
MC_UI_PORT=6799 MC_UI_SECONDS=900 MC_UI_OUT=/tmp/mc-ui.json \
  cargo test --manifest-path src-tauri/Cargo.toml --test ui_fixture -- --ignored --nocapture
MC_E2E_DESKTOP=/tmp/mc-ui.json npm run ext:e2e
```

Measured on the "typical" fixture (an issue list with 25 rows), loopback, Chrome
156: `snapshot` with `interactiveOnly: true` is about 10 KB (~2,500 tokens;
the target is 4,000), the full snapshot about 11 KB; round trips desktop →
extension → page → desktop are under 1 ms for `tabs.list` and `read`, 2–6 ms
for `find` and `snapshot`, and about 106 ms for a click (its 100 ms settle
window for the delta).

## Protocol details this extension adds or reads

All optional and backward compatible (a field the other side does not know is
ignored):

- `hello.granted`: the optional permissions and origins granted now (the same
  list as `permission.changed.granted`; `debugger` once the user granted it on
  the options page, whatever Chrome holds); `hello.capabilities` also carries
  `webNavigation`, `downloads` and `screenshotFallback`.
- `permission_needed` names the permission in both `error.data` and the
  result's `data`: `{"permission": "debugger"}` or `{"permission": "https://github.com/*"}`.
- `type`/`fill`/`press` (and `trusted` typing): with `refuseSecret: true` a
  secret field answers `secret_field`; without it, `unsupported` unless the
  command carries `allowSecret: true` (the user granted it on the desktop).
- `press` and `type` take `trusted: true` (debugger key events) like the mouse.
- `click`/`dblclick`/`hover`/`press` (and `type`/`fill` with `submit`) answer
  `url` and `title` of the page they ended on with any `snapshot` delta;
  `navigated: true` when the page navigated (no delta then); `dialog: {kind,
message}` when the action opened a JavaScript dialog (answer it with
  `dialog`); `newTab: {tabId, url}` when a link meant for a new tab was opened
  in the lease (a scripted click cannot open one: the popup blocker drops it).
- `scroll` answers `atEnd`; `select` answers `selected`; `upload` answers `files`.
- `eval` needs the `debugger` permission (it runs through `Runtime.evaluate`:
  no `eval()` in extension code, and page CSP does not block it); an exception
  is `internal` with `data.exception`.
- `console {level}` is a minimum level (`debug` < `log` < `info` < `warning` < `error`).
- Events `lease.resumed` (`lease`, `by: "user"`) after **Resume** in the side
  panel (a desktop that does not know it keeps its own 60 s stop: Resume then
  only lifts the extension's); a `lease.revoked` that happened while offline is
  sent after the next `welcome`.
- Pairing codes: case, spaces and dashes are ignored; the extension always
  sends `ABCD-EFGH`.
- `error` `bad_frame` for a frame that is not a JSON object.
- From the desktop's additions: `error replaced` stops automatic reconnects for
  5 minutes; `state.settings.sensitiveDomains` keeps a sensitive site off
  unless it has its own entry, and the side panel does not offer **Act** for
  it; wildcard site-mode keys `scheme://*.domain[:port]` (exact origin, then
  the longest wildcard, then `*`); `setSiteMode` with `mode: null` (**Use the
  default** in the side panel); `answer` replies `not_found` and `refused`.
- A `timeoutMs` under 2 s keeps three quarters of it for the work instead of
  "less 500 ms".

## Firefox

`npm run ext:build:firefox` produces a Firefox variant: `background.scripts`
instead of a service worker, `sidebar_action` instead of `side_panel`,
`browser_specific_settings.gecko` (id `browser-extension@motherclaude.dev`,
Firefox 128+), no `key`, and no `debugger` permission. The code feature-detects
what is missing: trusted input, screenshots of background tabs, the console,
`eval` and dialog answers report `permission_needed`/`debugger_unavailable`;
visible-tab screenshots use `tabs.captureVisibleTab`. It has not been run in
Firefox (none is installed here), and three things are known to differ:
Firefox's `Origin` is `moz-extension://<random per-install UUID>`, which the
desktop cannot pin in advance (the protocol says "later"); `chrome.dom`
(closed shadow roots) does not exist there; and the `tabGroups` API exists
only in recent Firefox releases (not checked here), without which leases have
no group to live in.
