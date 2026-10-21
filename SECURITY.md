# Security

Mother Claude exposes a control plane for your Claude Code sessions. Whoever can
drive it can run code on your Mac, so every request is attributed to a
**principal** and checked against what that principal may do. The full user guide
for remote access is [docs/REMOTE.md](docs/REMOTE.md).

## Who is asking

Every listener tags its requests with the door they came through (loopback
`127.0.0.1:6725`, the LAN TLS listeners, the tunnel port, the bridge port
`127.0.0.1:5612` and every other API server's port). Tunnels (ngrok,
cloudflared, Tailscale) connect from 127.0.0.1, so the peer address alone
proves nothing. Two predicates are derived from each request:

- **machine-local**: a loopback listener, a loopback peer, exactly one `Host`
  header naming `localhost`, `127.0.0.0/8` or `[::1]`, and no proxy header at
  all (`Forwarded`, `Via`, `X-Forwarded-*`, `X-Real-IP`, `True-Client-IP`,
  `CDN-Loop`, `CF-*`, `Tailscale-*`, `ngrok-*` except the
  `ngrok-skip-browser-warning` our own app sends, and the other client-IP
  headers of common proxies: `X-Client-IP`, `X-Cluster-Client-IP`,
  `Fastly-Client-IP`, `Fly-Client-IP`, `X-Original-Forwarded-For`,
  `X-Envoy-External-Address`).
- **local**: machine-local and no `Origin`, or a local one (`tauri://localhost`,
  `http(s)://localhost`, `127.x`, `[::1]`). `Origin: null` is never local.

Any request carrying a proxy header, or reaching the LAN listener from a
non-private address (a Tailscale `100.x` address included), is treated as coming
**through a tunnel**: it gets the reduced route set (no `/v1`, no
`/servers/*`, no `/hooks/*`), the tunnel pairing rules and the device's
internet grant. The client address used for rate limits, device records and
the audit log is the peer's, or behind a loopback tunnel the last
`X-Forwarded-For` value, else `Forwarded: for=`, else `CF-Connecting-IP` (see
[HTTP hardening](#http-hardening)).

## Credentials

| credential                                                              | holder                                            | accepted                            | principal                                                                                                                                                |
| ----------------------------------------------------------------------- | ------------------------------------------------- | ----------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------- |
| desktop token — new on every launch, kept in memory                     | the desktop webview, via the `server_info` invoke | only from a _local_ request         | Desktop: everything                                                                                                                                      |
| local-script token — the `script-token` file in the app data dir (0600) | your own scripts                                  | only from a _local_ request         | data and control routes; never pairing, devices, remote access or plugin install; dangerous only on the bridge port, like a tokenless local caller there |
| device token `mcd_…`                                                    | paired phones and browsers                        | any ingress                         | Device                                                                                                                                                   |
| hook token `mch_…`                                                      | the foreign-control plugin                        | only local, only `/hooks/*`         | Hook                                                                                                                                                     |
| sidecar token `mcs_…`                                                   | the Path A sidecar (per run)                      | only local, only its callback route | Sidecar                                                                                                                                                  |

Tokens travel as `Authorization: Bearer`. `?token=` is accepted only from a
local request (local scripts, SSE), so no token ever rides in a URL through a
tunnel or across the LAN. WebSockets authenticate with their first frame
(within 5 s: a rejected or empty token closes the socket with 4401, a missing
or late one with 4408 so the client retries); an upgrade whose `Origin` is
neither local nor the page's own host is refused, and a device may hold at most
8 sockets. Only SHA-256 hashes of device tokens are stored (`devices.json`,
0600), compared in constant time. No Mother Claude token is passed to a
`claude` child process; the Path A sidecar gets its own per-run token and
deletes it from its environment before starting Claude. The old master token
(the `token` file, which earlier versions put in pairing links, logs, child
environments and hook headers) is deleted on upgrade and never accepted again.

**Desktop-only (admin) routes**: pairing codes and approvals, the device list and
its grants, remote-access settings, and the foreign-control plugin's status,
install and uninstall (install and uninstall also require a local request).
Neither a device, whatever its grants, nor the local-script token reaches them.

**Being local never grants a device anything.** A raw TCP forward (`ssh -R`,
`ngrok tcp`) can forge `Host: localhost` without adding proxy headers, so
authorization depends on the principal, not on the address.

## Pairing

- The desktop creates a one-time code: 12 Crockford base32 characters (60 bits),
  valid 5 minutes, at most 5 at a time. The QR encodes
  `https://<host>/#/pair?c=<code>` — the code sits in the URL fragment, so it
  never reaches a server, tunnel or proxy log.
- A **LAN** code pairs at once only when it is redeemed on the LAN listener, from
  a private address, with no proxy headers. Presented through a tunnel it is
  refused.
- Everything else (every **Internet** code) waits for the desktop: the phone shows
  4 digits and you type them on the Mac to approve. Three wrong tries deny the
  request. The desktop never receives the digits or the poll secret from the
  server.
- A code presented twice marks the pending request **contested**: it can no
  longer be approved, and both phones and the desktop are told.
- Pending requests expire after 10 minutes; at most 5 wait at once. Failed
  exchanges and polls are limited per client address (10 a minute, then an
  exponential block). There is no global pairing limit a stranger could
  exhaust.
- A paired phone can create a code for **itself** (the iOS Home Screen app has its
  own storage); it adds a second token to the same device record.
- Revoking a device (on the desktop, or "Unpair this device" on the phone) erases
  its tokens, closes its WebSockets and drops its push subscriptions. Devices idle
  for 30 days (`MOTHER_CLAUDE_DEVICE_IDLE_DAYS`) expire. Every new device raises a
  notice.

## Dangerous actions

These are **dangerous**:

- switching to, starting in or continuing in `bypassPermissions`;
- choosing a standing permission grant (a rule saved to user, project or local
  settings, extra directories, `setMode: bypassPermissions`); a caller without
  the right is offered each rule scoped to the session instead;
- approving a tool call the server classifies as dangerous, whatever channel it
  came from: an edit or shell command touching Claude Code configuration
  (`.claude/`, `.claude.json`, `.mcp.json`), Mother Claude's data dir, `~/.ssh`,
  shell startup files, launch agents, `.git/hooks` or `.git/config`, running
  `claude plugin|mcp|config`, or any read of Mother Claude's own data dir (its
  tokens and keys: `Read`, `NotebookRead`, `Glob` and `LS` inside it, a `Grep`
  rooted at or above it). Hook cards are classified on the tool's full input,
  before it is trimmed for display, and bridge approvals on the server's own
  classification;
- anything sent to a session that runs in `bypassPermissions` (messages, typed
  text, keys, continuing it), since nobody approves what it then does;
- raw terminal keys, a leading `!` or a non-allowlisted slash command typed into
  a tmux session, and experimental PTY injection;
- `rm`, transcript purge and starting a sign-in flow;
- carrying a folder's trust (its `.claude.json` entry: `hasTrustDialogAccepted`,
  allowed tools, MCP choices) with a move, copy, restore or pull, and copying
  the `settings` or `mcpServers` customizations to another account (they
  choose how Claude signs in, what runs in every session, which programs
  start);
- HTTP bridge options that run code nobody approves (MCP servers, pre-approved
  tools, extra directories, an untrusted folder, work sent into a conversation
  whose approvals are off).

Spawning or continuing a session from anything but the desktop is also limited
to folders Claude Code already trusts (`untrusted_cwd`), because a headless run
shows no trust dialog. Dangerous actions are allowed for:

- the desktop;
- a device you granted it under Remote › Paired devices — **Allow dangerous
  actions (LAN)** applies only on the LAN; **… over the internet** is a separate
  grant for tunnels, proxies, Tailscale addresses and any other path;
- on the bridge port, a request that is _local_ (curl, Postman, a localhost
  page), with no token or with the local-script token — never a page on another
  website.

`MOTHER_CLAUDE_ALLOW_REMOTE_DANGEROUS=1` extends this to every device on the LAN
(never through a tunnel). Every dangerous action by a device raises a desktop
notice and is written to the audit log.

## TLS on the LAN

- The LAN listeners bind only private IPv4 addresses (RFC 1918 and 169.254/16;
  Tailscale's 100.64/10 when you enable it). Public IPv4 and IPv6 addresses are
  never bound; the skipped ones are listed on the Remote page.
- Certificates come from a **local CA** generated on first run (key 0600 in the
  app data dir). It is name-constrained to private and link-local IPv4 ranges,
  `127.0.0.0/8` and `localhost` (Tailscale's `100.64.0.0/10` only once you bind
  a Tailscale address) and limited to TLS server certificates, so even a leaked
  CA key cannot impersonate a public site or sign mail or code. Any process
  running as you can read that key, so removing the profile on the phone is how
  you revoke the trust. The leaf is valid 397 days and re-issued whenever the
  LAN addresses change. Only the public CA certificate is ever served.
- Trust the CA on a phone (`/api/pair/ca.crt`, or `/api/pair/ca.mobileconfig` on
  iOS, then enable full trust) for a secure context: WebSockets, the service
  worker and push then work on the LAN. Otherwise tap through the warning and
  compare the fingerprint shown on the Remote page.
- Pair over the LAN only on a network you control.

## Internet access

- Off by default. When on, Mother Claude listens on `127.0.0.1:6726` (change it
  with `MOTHER_CLAUDE_TUNNEL_PORT`) for a tunnel to forward to. That port
  requires the last `X-Forwarded-Proto` to be `https`, serves no `/v1`, no
  `/servers/*` and no `/hooks/*`, and accepts only device tokens. Switching
  Internet access off closes every connection on it (a tunnel pointed at 6725
  instead is not affected, which is one reason to use 6726).
- The public URL is read from the tunnel's own local API (ngrok, cloudflared,
  `tailscale serve status`) or entered by hand — never from request headers.
- **Tailscale Serve** (`tailscale serve --bg 6726`) keeps access inside your
  tailnet with a trusted certificate and is the recommended private option.
  ngrok, cloudflared and Tailscale Funnel make the URL public; everything behind
  it still needs a device token.
- Warnings on the Remote page: ngrok's local inspector recording requests (use
  `--inspect=false`), quick-tunnel URLs that change every run, Tailscale Funnel
  names published in certificate logs, tunnels pointing at 6725 (works, less
  defended) or at an API server's port (named), raw TCP tunnels, and Internet
  access switched on with no tunnel found.

## The Claude HTTP bridge (port 5612)

It stays tokenless and open to any origin for **machine-local** requests, so
local tools, Postman, Swagger and pages you are building can call it. A
tunnelled request needs a token (`tunneled_request_requires_token`), and while a
detected tunnel points at 5612 the tokenless mode is off entirely
(`bridge_exposed_requires_token`). Dangerous requests additionally need a local
or absent `Origin`, or a token with the dangerous right. The `/v1` mount on the
LAN-reachable port always needs a token and is never served through a tunnel.

The API screen's _Open to tunnels_ switch (`openToTunnels` on
`POST /api/bridge/start`, which only the desktop principal may set to `true`;
`MOTHER_CLAUDE_BRIDGE_REQUIRE_TOKEN=never` sets its starting position) is the
operator's explicit opt-out: no request to 5612 needs a token, from anywhere,
tunnels included — whoever has a tunnel's URL can run ordinary turns and
approve ordinary tool calls. The
dangerous right still needs a _local_ request, and while a tunnel points at the
port no tokenless request is treated as local (a raw TCP tunnel is
indistinguishable from this machine), so only a token holder has it. The `/v1`
mount is unaffected. The app warns at start, on the API screen and on the
Remote page.

- A development server or reverse proxy **on this Mac** that relays to 5612 is
  accepted for the tokenless mode when its only proxy headers are
  `X-Forwarded-*`, every `X-Forwarded-For` hop is loopback and
  `X-Forwarded-Host` names this machine. Any tunnel fingerprint (`cf-*`,
  `ngrok-*`, `via`, `forwarded`, a public hop) fails that, and such a request is
  never _local_ for dangerous actions.
- With `MOTHER_CLAUDE_BRIDGE_REQUIRE_TOKEN=1`, a page allowlisted in
  `MOTHER_CLAUDE_BRIDGE_ORIGINS` calling 5612 from this Mac may present the
  local-script token (header or `?token=`); the desktop, hook and sidecar tokens
  still need a _local_ request.
- A bridge approval answered by a caller without the dangerous right cannot
  replace the tool's input, and may only pick one of the request's own
  session-scoped suggestions.
- Revoking or expiring a device ends its open `/v1` event streams and fails its
  pending runs (`device_revoked`).
- `POST /v1/threads` with a resume id that another process is running forks it
  rather than taking it over.

### Several API servers

Besides the main API server (5612), the API screen can add more, each on a
loopback port of its own, with its own token policy, started gate and defaults
(docs/BRIDGE.md, docs/ACCOUNTS.md §13.5). Everything above holds for each of
them, **port by port**:

- A detected tunnel to one server's port switches off that port's tokenless
  mode only. A request that reaches a server through `/servers/<id>/` on 5612
  gets the target server's token policy, account and started gate, while
  whether a tunnel points at the port is decided by the port it arrived on
  (5612) — so a tunnel to 5612 makes every server tokenless-off through it,
  and a tunnel to another server's port leaves 5612 alone.
- `/servers/<id>/v1/` on the LAN-reachable port always needs a token, whatever
  the target's own policy, and is never served through a tunnel or on the
  Internet-access port.
- A server with `accountOnly` runs as its account only: a request naming another
  account is refused (`403 account_not_served`), and another account's
  conversations, operations, events and tool requests do not exist through it
  (`404`). Its `/auth/login` signs that account in, and still needs the
  dangerous right.
- Adding, starting, stopping, changing and removing servers is open to every
  dashboard principal, as starting the main one is; defaults that skip
  approvals and *Open to tunnels* stay desktop-only, an untrusted default
  folder needs the dangerous right — also when a phone starts a server whose
  saved defaults have them. *Open to tunnels* is never saved: an added server
  starts closed to tunnels, `MOTHER_CLAUDE_BRIDGE_REQUIRE_TOKEN=never`
  notwithstanding, and `MOTHER_CLAUDE_BRIDGE_AUTOSTART=all` starts every saved
  one closed.
- The saved servers live in `api-servers.json` in the app data dir (0600): ids,
  names, ports and defaults — no token, no secret.

## HTTP hardening

- `Content-Security-Policy` with `script-src 'self'` (no inline scripts; the
  build fails if `index.html` gains one), `connect-src 'self'` plus the exact
  public/LAN `wss://` origins, `frame-ancestors 'none'`; `nosniff`,
  `Referrer-Policy: no-referrer`, `Cache-Control: no-store` on the API, HSTS
  behind an HTTPS tunnel. The desktop webview has its own Tauri CSP.
- CORS is open only to local requests (the desktop webview).
- Failed authentication: 20 a minute per client address, then an exponential
  block. Machine-local callers and valid credentials are never limited, and a
  block never stops a paired device's WebSocket (a bad first-frame token is still
  refused). A flood of new addresses never lifts an active block.
- Behind a tunnel the client address is the **last** `X-Forwarded-For` value,
  which ngrok, Cloudflare and Tailscale append or set themselves;
  `CF-Connecting-IP` is read only when no forwarding chain exists, and header
  values naming loopback, "any" or multicast are ignored. Limits key that address
  apart from real local callers, and count forwarded IPv6 clients per /64. The
  tunnel provider on a pairing card comes from the app's own tunnel detection,
  never from request headers.
- Unauthenticated WebSockets (every browser socket until its first frame, which
  must arrive within 5 s and fit in 4 KiB) have separate pools for this Mac, the
  LAN and tunnels: at most 4 per address and 64 per pool (32 for this Mac). A
  full pool closes its oldest waiting socket instead of refusing the newcomer, so
  nobody on the internet can lock the desktop or a paired phone out.

## Logs and audit

Tokens appear in logs only as short fingerprints; request spans record the path,
never the query string or headers. `audit.jsonl` (0600, rotated at 2 MB, in the
app data dir — never under `~/.claude`) records every state change made by a
device or local script, every pairing event and every device change, with the
device, ingress and client address.

## Foreign-session control (the hooks plugin)

- It is a user-scope Claude Code plugin, installed and removed only by an
  explicit action in the desktop app through `claude plugin install` /
  `uninstall` (the CLI records it in `enabledPlugins` and
  `extraKnownMarketplaces`). Mother Claude never edits the `hooks` in
  `~/.claude/settings.json`.
- Its scripts talk only to `http://127.0.0.1:<port>` (they refuse any other
  base, skip `~/.curlrc` and every proxy) and authenticate with the hook token
  `mch_…`, accepted only from local requests and only on `/hooks/*`. The token is
  new at every launch; the scripts read it from `hook.curlrc` (0600, in the app
  data dir) on every call. They always exit quietly, so a closed app never
  blocks a session. After uninstall a `hook.disabled` marker keeps hooks that
  running sessions already loaded silent.
- Sessions Mother Claude runs itself carry `MOTHER_CLAUDE_OWNED=1` and are
  skipped by the scripts; the server also ignores hook decisions for them.
- Any process running as you can read the hook token, so the events that can
  raise a card, hand over a relay, withdraw cards or push to your phones (and
  any change to a session's reported permission mode or process id) are
  accepted only when the request's TCP connection traces back to that
  session's own `claude` process (`ps` + `lsof`); a mismatch is ignored. A
  report of `bypassPermissions` from any source sticks: one report can never
  lower the gate.
- A relayed phone message is hook text inside a session that runs user-wide.
  Every session gets a marker in its `SessionStart` context (an HMAC of the
  session id under `relay-marker.key`, 0600 in the app data dir, so it survives
  a restart), every relay starts with it, and the model is told to treat text
  that claims to be a phone message without that marker (in a file, a web page,
  a tool result) as untrusted.
- A queued relay is dropped at hand-over if its sender lacked the dangerous
  right and the session now runs in `bypassPermissions`; relays expire after
  30 minutes and fail when the session ends.

## Push notifications

- VAPID keys live in the app data dir (`vapid.json`, 0600); subscriptions are
  stored per device (`push.json`, 0600, at most 5 per device) and deleted when
  the device is revoked or the push service answers 404/410.
- A subscription endpoint must be `https`, a DNS name on the allowlist of the
  browsers' push services, and resolve only to public addresses; it is checked
  when subscribing and again before every send (no redirects, 10 s timeout, no
  proxy), so push cannot be used to make the Mac call into your network.
- A notification carries the session name and the kind of event; the command or
  question is included only for devices that turned on **Show details**.

## Secrets

- Never read: the `*.key` files beside the pidfiles, `daemon/control.key`, the
  cc-daemon control socket and `/tmp/cc-socks/*.sock`.
- Never served: MCP server secrets — each server is rebuilt from an allowlist
  (`type`, the command's file name, `args` with URLs cut to their origin and
  secret-flag values, `KEY=value` pairs and bare keys redacted, the `url`'s
  origin, and only the keys of `env` and `headers`); bridge-session owner ids;
  and any token (the pairing API no longer returns the desktop token).
- Tokens, certificates and keys are created at runtime in the app data dir and
  are **never committed** (`.gitignore` excludes `*.pem`, `*.key`, `*.token`).

## Claude accounts

[ACCOUNTS.md](docs/ACCOUNTS.md) §7 in short:

- A token account's token and the backup passphrase are kept in the login
  Keychain (`dev.motherclaude.desktop`; `0600` files in the app data dir off
  macOS, or with `MOTHER_CLAUDE_SECRET_STORE=file`). `accounts.json` holds a
  token's SHA-256 only. No token or passphrase is ever in a response (the
  generated passphrase is shown once), a WebSocket frame, a log line, an error,
  an audit line or a backup.
- A token reaches a `claude` child only in its environment
  (`CLAUDE_CODE_OAUTH_TOKEN`), with the inherited sign-in overrides removed; a
  token account whose token is not on this machine fails, it never falls back
  to the Default sign-in. Which token a session outside Mother Claude runs with
  is learned as a hash: from the process environment on Linux, from the hooks
  plugin's `X-MC-Token-Sha256` on macOS (the hook hashes it inside the
  session); the value is compared and dropped.
- Adding, removing and signing in accounts, tokens, the backup passphrase and
  the hooks plugin installer are desktop-only. Moving and copying data, backups,
  restores, pulls, undo, the backup settings, account defaults, copy consent
  and renaming an account are open to the desktop and to paired devices (LAN or
  tunnel; [ACCOUNTS.md](docs/ACCOUNTS.md) §13.9), never to the local-script
  token. Every such action of a device is audited with the device and named to
  the desktop in a notice. Picking an account for a session is allowed to
  anyone who may start one.
- Our code writes into a Claude config directory in four explicit user
  actions only — a new account's empty directory (desktop only), move/copy,
  restore, pull — each refusing while a live process uses a shared file it
  would write, keeping an undo for seven days, writing atomically and leaving
  an audit line, whoever asked. A folder's trust (`hasTrustDialogAccepted`,
  its allowed tools and MCP choices) is never copied or restored without the
  explicit opt-in, which is a dangerous action.
- Backups are encrypted (age, X25519 per set, the set's key under the
  passphrase with scrypt); the only plaintext in a set is `set.json` (account
  name, the set's public key, machine ids, names and times), and it is display
  only. A backup folder is treated as untrusted input: the set's index carries
  an HMAC-SHA256 keyed from the set's secret key, every archive and bundle is
  checked against the authenticated index before it is opened, a set that
  fails the check is refused and never written over, and no id or file name
  from a set is used as a path before it is validated. Someone who can write
  the folder can delete files or put older ones back, nothing more. Credentials
  are never backed up.
- A move, copy, restore or pull checks its preconditions again right before it
  writes, never leaves a file it moved only in a deleted undo directory, and
  never follows a symlink in a session's place.
- Customizations (`CLAUDE.md`, settings, skills, agents, commands, output
  styles, keybindings, MCP servers, prompt history) are only ever copied,
  between accounts in different directories, each replaced file kept for the
  undo. A symlinked skill is made again as a link to the same target, never
  followed. MCP servers are named only by name in plans and results: their
  `env` and `headers` values are never served. The dry run warns when copied
  settings would choose how the target signs in or run commands.
- Storages besides the backup folder hold sets encrypted the same way, under
  the same passphrase; a passphrase change re-keys every one of them and
  refuses while one is not there. A conversation moved into a set is removed
  from the account only after the set's copy was checked against it byte for
  byte.

## Reporting

This is a research-preview-grade tool over undocumented internals. Audit before
relying on it in any shared environment.
