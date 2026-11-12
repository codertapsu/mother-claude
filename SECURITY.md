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

Your other computers come through a door of their own, the Machines
transport: a request there is never local, no token is read on it, and its
principal is the paired key the connection authenticated (see
[Machines](#machines-your-other-computers)).

## Credentials

| credential                                                              | holder                                            | accepted                            | principal                                                                                                                                                |
| ----------------------------------------------------------------------- | ------------------------------------------------- | ----------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------- |
| desktop token — new on every launch, kept in memory                     | the desktop webview, via the `server_info` invoke | only from a _local_ request         | Desktop: everything                                                                                                                                      |
| local-script token — the `script-token` file in the app data dir (0600) | your own scripts                                  | only from a _local_ request         | data and control routes; never pairing, devices, remote access or plugin install; dangerous only on the bridge port, like a tokenless local caller there |
| device token `mcd_…`                                                    | paired phones and browsers                        | any ingress                         | Device                                                                                                                                                   |
| hook token `mch_…`                                                      | the foreign-control plugin                        | only local, only `/hooks/*`         | Hook                                                                                                                                                     |
| sidecar token `mcs_…`                                                   | the Path A sidecar (per run)                      | only local, only its callback route | Sidecar                                                                                                                                                  |
| machine key — Ed25519, kept in the OS secret store                      | your other computers, once paired                 | only on the Machines transport      | Peer: the peer routes, with the grants this computer gave it ([Machines](#machines-your-other-computers))                                                |

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
  settings, extra directories, `setMode: bypassPermissions`, a command a
  project may always run on another of your computers); a caller without
  the right is offered each rule scoped to the session instead (a run on
  another computer: once);
- approving a tool call the server classifies as dangerous, whatever channel it
  came from — a command a session asks to run on another of your computers
  included: an edit or shell command touching Claude Code configuration
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
- `rm`, transcript purge, deleting conversations or projects (a dry run too)
  and starting a sign-in flow;
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
  website;
- another of your computers you gave **Dangerous** (for the network it uses
  now), and then only for a person who holds the dangerous right on that
  computer ([Machines](#machines-your-other-computers)).

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
device, ingress and client address — and every state change another of your
computers makes, with who acted through it (`machine.action`), its pairings,
grants, revocations (`machine.revoked`, with who asked for one everywhere),
introductions (`machine.introduce`, `machine.introduced`,
`machine.intro.*`), asks, moves and take-overs, and every run of a command
for or by another computer (`machine.run*`: the program's name and the
command line's SHA-256, never the line). Switching *Start Mother Claude when
you sign in* is audited too (`app.autostart`).

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
- Signing in runs `claude auth login` or `claude setup-token` in a
  pseudo-terminal with `BROWSER=true`, so the CLI opens nothing itself; Mother
  Claude opens the link it shows, and only an `https` link to `claude.ai`,
  `claude.com` or `anthropic.com` (or a subdomain), without credentials or a
  port — any other link is never shown or opened. The link is the desktop's
  alone (never in a frame, another client's read, an error or an audit line;
  whoever completes it signs the account in). One sign-in runs at a time,
  across all accounts, and a run stores its token into the account that
  started it, never another (a token another account holds is refused).
  Opening the link, typing the browser's code and cancelling are desktop-only,
  like starting a sign-in.
- Who an account signed in as is kept only as the SHA-256 of its email
  (`identitySha256` in `accounts.json`, never the email), to warn when a
  sign-in lands in the wrong account or in another account's Claude account;
  keeping a new one is desktop-only and audited without the email.
- Adding, removing and signing in accounts, tokens, the backup passphrase and
  the hooks plugin installer are desktop-only. Moving and copying data, backups,
  restores, pulls, undo, the backup settings, account defaults, copy consent
  and renaming an account are open to the desktop and to paired devices (LAN or
  tunnel; [ACCOUNTS.md](docs/ACCOUNTS.md) §13.9), never to the local-script
  token; deleting conversations too, with the dangerous right. Every such action of a device is audited with the device and named to
  the desktop in a notice. Picking an account for a session is allowed to
  anyone who may start one.
- Our code writes into a Claude config directory in five explicit user
  actions only — a new account's empty directory (desktop only), move/copy,
  restore, pull, deleting conversations — and, with Machines, when a session
  moved here from another computer lands (as a pull does) — each refusing
  while a live process uses a shared file it would write, keeping an undo for
  seven days, writing atomically and leaving an audit line, whoever asked. A
  folder's trust (`hasTrustDialogAccepted`, its allowed tools and MCP
  choices) is never copied or restored without the explicit opt-in, which is
  a dangerous action.
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
- Deleting conversations or projects ([ACCOUNTS.md](docs/ACCOUNTS.md) §14.3)
  takes each conversation's files — and a project folder left empty — into an
  undo directory listed before anything moves, kept seven days unless the user
  asks for no undo, and then removed only once every file was taken out. It
  never touches `history.jsonl` or `.claude.json`, skips a conversation that
  runs (checked right before and after its files move), never follows a
  symlink, and is audited with counts, never paths. Showing a transcript or a
  project's folder in Finder or File Explorer is desktop-only, and the path is
  the index's, never the request's.
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

## Machines (your other computers)

Pairing your computers lets each one list and drive the others' Claude Code
sessions, move a session, with its code, from one to another, keep a
project's code the same on two of them, and let a session run commands on
another ([MACHINES.md](docs/MACHINES.md); the design is
[PLAN-MACHINES.md](docs/PLAN-MACHINES.md)). Another computer is a principal
like a paired phone, identified by a key instead of a token. _Turn on
Machines_ waits until the setup has no required check left, and the switch
guards this computer: while Machines is off here, a paired computer reaches
only what pairing and the setup need (the hello, pairing, the setup's
checks, asks and readiness test, settings offers, roots, leases) — never
this computer's session stream (`/ws`), its session routes, its moves or
its runs (`403 machines_off`) — and this computer uses none of the others
(its proxy and its moves answer `409 machines_off`, its sessions'
`machine_run` is refused; its links to their sessions stay closed). Turning
Machines off closes the links both ways at once and stops what this
computer runs for the others; it does not unpair — revoking does.

### Identity and the connection

- Each install makes a **machine key** (Ed25519) on first use and keeps it in
  the secret store (`machine-key`: the login Keychain, Windows Credential
  Manager, a `0600` file elsewhere), never in a file of ours, a log, a frame
  or a backup. Its public half is the computer's address on the transport.
  `machines.json` (0600) holds each paired computer's id, name, public key,
  grants and last addresses, and the shared settings: nothing secret.
  Reinstalling makes a new key, which the others refuse until you pair again.
- Computers talk over QUIC between their keys (iroh): the TLS 1.3 handshake
  authenticates both ends by their keys, so a paired computer _is_ its
  pinned key. Directly on the same network, hole-punched across networks,
  otherwise through a relay that forwards ciphertext only. The relay setting,
  meant to be the same on every computer: **public** (n0's relays, the
  default; a computer's key is found through n0's DNS), **your own**
  `iroh-relay` (an `https://` address; a peer is then dialled as its key plus
  that relay, and no n0 service is asked anything) or **none** (the same
  network only).
- The transport runs only once Machines is in use: a pairing, a paired
  computer, the setup's checks and test. A computer where nobody opens the
  setup or pairs makes no new connection.
- A connection from a key that is not paired is closed as it is accepted,
  except the one key pairing with this computer right now, which reaches
  `/peer/pair/*` only. Revoking a computer closes its connections at once,
  and every request checks the grants as they are then.

### Pairing two computers

- **Pair a machine** (desktop only) shows an 8-character code (Crockford
  base32, 40 bits) that works once, for five minutes. The code derives a
  short-lived rendezvous key, so the other computer finds this one from the
  code alone (mDNS on the local network — the _Nearby_ list — or n0's DNS
  with the public relays). The rendezvous answers only a caller that proves
  it holds the code (an HMAC over the caller's own key), and only with this
  computer's real key and address. A code proven by two different computers
  is spoiled for both.
- The joiner then connects to the host's machine key and pairs over that
  authenticated connection: an HMAC of the code over both keys, and a
  commitment to its nonce before it sees the host's. Both screens show six
  digits computed from the code, both keys and both nonces, so a computer in
  the middle — even one that saw the code — cannot make the two screens
  agree.
- When the joiner did not come over a direct path on the local network, the
  host never shows the digits: you type the ones the joiner shows. One wrong
  number ends the pairing.
- Each side chooses what the other may do on it, and nothing is stored until
  both confirmed, within ten minutes. Pairing, joining, confirming, cancelling
  and _Nearby_ are desktop-only; every pairing, grant change and revocation
  is audited.

### Introductions: never transitive

- Trust is pairwise. Being paired with a computer gives nothing on the
  computers it is paired with, and no computer can pair two others.
- **Introducing** (desktop only, `POST /api/machines/introduce`) only tells
  two paired computers about each other over the introducer's own links
  (`POST /peer/intro`, its `view` grant on each): each one's name, machine
  id, public key and last addresses — no secret, no grant. The receiver
  refuses one naming itself or its sender, keeps it ten minutes (at most four
  open from one computer), shows a desktop notice, and audits it.
- Nothing is paired until **each** desktop accepted it and confirmed the six
  digits: the joiner dials the host by the key named. While the
  introduction is open, that key may reach the host's pairing routes — the
  door the key pairing by code passes, nothing else — and only that key can
  join it (an unknown key is refused at the door, another paired computer
  at the join). The exchange is the code pairing's commit-then-reveal over
  both keys and both nonces, the introduction (its id and introducer) taking
  the code's place in the digits, so a computer in the middle — the
  introducer included — would make the two screens disagree. Across the
  internet the host types the digits. A decline on either side tells the
  other, and nothing is stored; the pairing is audited with its introducer.
- Each computer's hello tells a computer it lets see its sessions the ids
  of the computers it is paired with (ids only), so that one can offer to
  introduce two that aren't.

### Revoke everywhere: it only removes trust

- _Revoke everywhere_ (desktop only, `POST
  /api/machines/{id}/revoke-everywhere`) revokes the computer here, then asks
  every other paired computer to (`POST /peer/revoke`, any paired computer).
  The request is kept in `machines.json` until each took it: an offline one
  is told on its next hello, either way.
- A computer asked applies it at once — it only removes trust: the
  computer's key forgotten, its connections cut, an audit line naming who
  asked, a desktop notice. It names a third computer only: a request naming
  the computer asked or the one asking is refused (`422`), one naming a
  computer it doesn't know changes nothing, and nothing in it can add a
  computer or a grant.
- A compromised paired computer can therefore unpair your other computers
  from each other (a denial of service you undo by pairing again), and
  nothing more through this route.

### What a paired computer may do here

- Its requests reach the peer router only: `/peer/*` (hello, setup, settings
  offers, roots, moves, leases, copies, code sync, runs, tasks), and — while Machines is on
  here — the
  session stream `/ws` (this computer's own sessions — never a third
  computer's — and none of the desktop-only frames) and an allow-list of
  session routes under `/api/`:
  - **view**: the sessions list, a session and its transcript, agents,
    changes, files, context, MCP servers, terminal view and task output; the
    inbox, projects, defaults and plan usage; moves;
  - **control**: messages and uploads, interrupt, stop, respawn, model,
    effort, mode, thinking, ultracode, rename, answering approvals,
    continuing, keys, the message queue, and a new session (only in a folder
    Claude Code already trusts there, `untrusted_cwd`);
  - **move**: starting a move of a session that lives here, and cancelling,
    undoing or retrying one; choosing which computers keep a copy of one
    (its copies' state is **view**).

  Everything else answers `404`: the admin routes, accounts, backups,
  devices, deleting, Machines' own settings, `/hooks/*`, `/mcp`,
  `/ws/browser`, `/v1`.
- **Grants**, set per computer for the **same network** (a direct path to a
  private or loopback address, as the transport sees it for each request)
  and for the **internet** (relayed or public) apart: `view`, `control` and
  `move` (on for a new pairing), `run` (off: commands for its sessions, in a
  work folder, [below](#running-commands), and tasks handed to a session
  here, [below](#handing-a-task-over)) and `dangerous` (off). Driving
  needs seeing; moving, running and dangerous need driving. Changing them,
  renaming and revoking are desktop-only.
- **Driving through this computer.** The dashboard reaches another
  computer's sessions through `/api/machines/<id>/proxy/…`, which adds
  `X-MC-Acting`: `desktop`, a device's id and name and whether it holds the
  dangerous right _here_, or `script`. A dangerous action there needs both
  its `dangerous` grant for this computer on that path **and** the acting
  person's right here: a phone without the dangerous grant on the Mac cannot
  reach for it through the Mac. A missing, malformed or unknown value is
  never dangerous. A move another computer starts here reaches its target
  from this computer, which then says `machine` with the right it judged
  for that computer (its acting person's right ∧ this computer's
  `dangerous` grant for it); the target intersects that with its own
  `dangerous` grant for this computer, so each hop needs its grant. The
  local-script token may read moves through the proxy but never start,
  cancel, undo or retry one (as on its own computer), and a computer that
  says `script` acts through it is refused those too. A phone paired with
  one of your computers reaches the others this way, through it.
- The computer acted on writes every state change another computer makes to
  its audit log (`machine.action`: the computer, who acted through it, the
  method, path and status) and shows a desktop notice for every dangerous
  one. The proxy never passes on a `401` (a phone would unpair itself): the
  other computer refusing becomes `502 machine_refused`.

### Moving a session

- A move starts on the computer the session lives on
  (`POST /api/sessions/<id>/move`: its desktop, a paired device, or another
  computer through the proxy with the `move` grant there), and the receiving
  computer's `/peer/moves/*` need **its** `move` grant for the sender. The
  preflight only reads: nothing is written on either computer before the
  move is started.
- The receiving computer treats the move's record as untrusted input: the
  session id, account name, folders, file names (from a fixed list, each
  once), sizes, SHA-256 hashes, commit ids, branch and remote names, model
  and permission settings, the recipe and the queued messages are checked
  before git or the file system sees them, and every file's size and hash
  again before it lands.
- **git** runs from an argument list, never a shell, with the variables that
  would point it at another repository, index or configuration removed
  (`GIT_DIR`, `GIT_WORK_TREE`, `GIT_INDEX_FILE`, `GIT_CONFIG_PARAMETERS`,
  `GIT_CONFIG_COUNT`, …), `GIT_TERMINAL_PROMPT=0` and `GIT_OPTIONAL_LOCKS=0`;
  commits unsigned, no automatic maintenance, `core.fsmonitor=false`, and
  `core.hooksPath` set to a fresh, private, empty folder per operation —
  checked empty before every call (Git LFS writes its hooks there) and
  deleted after — so no hook of the repository runs (plumbing alone would run
  some). Pushes use `--no-verify`; fetches never follow submodules. With
  _Through your git remote_, the preflight checks first that the remote
  answers without a prompt on both computers (`git ls-remote`, no askpass,
  Git Credential Manager non-interactive).
- Files git ignores (`.env`, `node_modules`, build output) never travel,
  but for the ones a project lists to carry (setting the list needs the
  dangerous right): plain files under the repository's top that git ignores
  there, at most 1 MiB each and 50 in all, read and written never through a
  symbolic link or junction, in their own hashed `ignored.tar.gz` whose
  entries the target checks against the list. Carrying them needs the
  dangerous right on both computers (the mover's here, the target's
  `dangerous` grant for this one). The target writes them, atomically, only
  after the code landed; a file they replace is kept in the move's folder,
  and _Undo move_ puts it back and removes the ones it added, each only
  while it is as the move wrote it.
- A move writes a project — its working tree and `.git` — only through that
  plumbing (and the carried files git ignores, as above), keeping the
  previous `HEAD` and branch tip under `refs/mother-claude/undo/…` (_Undo
  move_ is offered for seven days). It never merges: a project folder with
  changes of its own, or a landing that would overwrite a file git ignores
  there, stops before anything changes and offers a separate worktree in
  Mother Claude's data folder instead.
- The conversation lands in the account's config directory as a **pull**
  does: one data job at a time, refused while the session runs there,
  staged and placed atomically, every file it replaces kept in its undo copy
  for seven days; a conversation both computers continued is kept as a
  conflict copy beside it, never merged. Sign-ins (tokens, keys,
  `.credentials.json`) never travel: the session runs under the account of
  the same name there.
- **Carrying a folder's trust** (its `.claude.json` entry), carrying files
  git ignores and moving a session that runs in `bypassPermissions` are
  dangerous on both computers: the person who starts the move needs the
  dangerous right where the session lives, and the receiving computer must
  have given that one `dangerous`.
  The trust entry is added only when there is none, and only while no live
  process uses that config directory.
- **One writer per session.** A lease names the computer a session lives on,
  with an epoch every move raises; every launch Mother Claude makes refuses a
  session leased elsewhere (`409 session_lives_elsewhere`). Leases are taken
  only from a computer holding this one's `move` grant, the higher epoch
  winning. _Take over_ works only while the computer the session lives on
  does not answer, and is audited.
- **Recipes** (a project's commands for after a move) are never run by
  Mother Claude: they are a request in the first message Claude gets there,
  and go through that session's own approvals.

### Copies kept current

- **What a copy holds**: the files a move carries of one session — its
  transcript, its subagents' and workflows' files, file-history snapshots,
  its todo list, its session environment — and nothing else: never its
  code, sign-ins (tokens, keys, `.credentials.json`), settings or any other
  file of a config directory. Every path the source names must be one of
  that session's own (the same rule as a backup archive's entries), and the
  receiving computer checks every size, offset, SHA-256 and prefix mark
  before it writes.
- **Where**: only in Mother Claude's data folder on the computer that keeps
  it, `machines/copies/<source>/<account>/<session>/` (folders `0700`,
  files `0600`), with `machines/copies/index.json` and each copy's
  `.mc/files.json` (sizes, times, marks — no content). It is never written
  into a config directory in the background: only a move that lands from it,
  or *Continue here from the copy* (a take over, audited), lands it there —
  as a pull lands a session, one data job at a time, refused while the
  session runs there, every replaced file kept for undo, a conversation that
  went on on both kept as a conflict copy.
- **Opt-in, and the grant.** Nothing is copied until someone chooses *Keep a
  copy on …* for a session (`PUT /api/sessions/<id>/copies`, any
  authenticated caller of this computer, or another computer through the
  proxy with its `move` grant here) or the desktop sets it for a project.
  The receiving computer takes a copy only through `/peer/copies/*`, which
  needs **its** `move` grant for the sender on the path the request came
  over, and Machines on there (`403 machines_off`; removing a copy is served
  without it). A copy offered for a session whose lease names the receiving
  computer is refused. Each receiving computer caps what it keeps of each
  other computer's copies (`copiesCapBytes`, 2 GiB by default, set on its
  desktop): the copy refreshed longest ago goes first, never one of a
  session that lives there.
- **Removing**: either computer may remove a copy (`DELETE
  /api/machines/copies/<id>`, the desktop or a paired device); stopping a
  copy on the source removes it there too, and revoking the computer stops
  everything it could send.
- Every copy is audited on both computers (`machine.copy.sent`,
  `machine.copy.received`, `.evicted`, `.removed`, `.dropped`, `.landed`):
  the session's id and sizes, never a byte of what it holds.

### Code sync

- **Both desktops consent, and both grants hold.** A project is kept in sync
  only once this computer's desktop turned it on (`PUT /api/machines/sync`,
  desktop only) and the other computer's desktop confirmed the request
  there (an asked action: shown, never run, until confirmed). The two
  folders must be the same project (the same `origin`, or a commit in
  common), mapped by a shared folder on both computers' own roots, and each
  computer must have given the other `move`: every write into this
  computer's checkout comes through `/peer/sync/*`, which needs this
  computer's `move` grant for the sender on the path the request came over,
  and Machines on here (`403 machines_off`). Turning it off is immediate on
  either computer; only `…/off` is served without the grant. *Sync now*,
  turning it on or off and choosing a side are the desktop's; a phone sees
  the state only.
- **It never merges and never overwrites.** A step lands only into a
  checkout that holds exactly its last synced state (HEAD, branch, staged
  tree, working tree); a change there in between stops it before anything
  is written, and both computers pause for a person to choose. The side not
  kept is first saved whole as one commit in its own repository, under
  `refs/mother-claude/sync/<id>/kept-<time>` (working tree with untracked
  files, staged state, its own commits), and is put back to the synced
  state only through git plumbing, refused with nothing changed when
  something could not come back (a file git ignores in the way, a changed
  submodule, an operation under way). A branch is reset over commits it
  has of its own only on the side that lost a choice, its old tip kept
  under the landing's undo ref too.
- **The same plumbing as a move**, with the same limits: git from an
  argument list, its hooks replaced by a private empty folder, every id,
  ref, name and path the other computer sends checked first, the step's
  files (a fixed list of names) checked by size and SHA-256 before they
  land, a landing that fails half-way put back. Files git ignores travel
  only when the project lists them and the receiving computer gave the
  sender `dangerous`. Nothing is pushed to a remote.
- **It waits, never queues.** Machines off, the other computer offline, a
  missing grant or a move of a session in the project pauses it; the next
  check starts from the checkouts as they are. Steps, conflicts and
  resolutions are audited on both computers (`machine.sync.*`: the project
  folder's name and counts, never a file's content).

### Running commands

A session's `machine_run` (Mother Claude's MCP tools, [MCP.md](docs/MCP.md))
runs a command on another of your computers at the session's code.

- **The `run` grant** lets a computer have this one run commands for its
  sessions: off until this computer's desktop ticks it, the same network
  and the internet apart, and it needs `control`. The `/peer/work/*` routes
  check it for the path the request came over, and that Machines is on here
  (`403 machines_off`). The setup's readiness echo (`/peer/work/echo`)
  needs the grant alone and runs only a fixed `echo machine-run-ok` in an
  empty scratch folder; `/peer/work/echo-back` (`view`) only has this
  computer ask the caller for that echo. A run is reached only by the
  computer that asked for it, and everything that computer sends —
  ids, the project's path, the folder to run in, the glob, file names,
  sizes and SHA-256 hashes, the snapshot's record — is checked before git
  or the file system sees it; a folder to run in or a glob never reaches
  outside the project or into `.git`.
- **Each run is approved on the computer that asks**, as a pending item
  (source `machine`: the Inbox, the session's page, paired phones) naming
  the computer, the folder and the command — unless a **standing grant**
  covers it: a project, a computer and a command prefix, kept in
  `machines/run-grants.json` (0600, written atomically), listed on the
  Machines page and revoked from the desktop (`DELETE
  /api/machines/run-grants/{id}`, desktop only). Attaching one is choosing
  a standing grant, so it needs the dangerous right: the local-script token
  or a phone without the grant may allow a run once, never always. A grant
  covers a command only when the command's first words are the prefix's
  words exactly (`cargo test` never covers `cargo test-foo`) and the
  command holds no shell metacharacter (`;`, `&`, `|`, a backtick, `$`,
  parentheses, braces, `<`, `>`, a line break), so nothing can be chained
  to a granted prefix. None is offered for a shell or a program that runs
  what it is given (`bash`, `pwsh`, `cmd`, `sudo`, `env`, `xargs`, …), nor
  for an interpreter alone (`python`, `node`).
- **A dangerous command needs both rights.** A command the shell-command
  rules of [Dangerous actions](#dangerous-actions) flag is a dangerous
  approval — only someone holding the dangerous right allows it, and no
  standing grant ever covers one — and the computer that runs it requires
  its own `dangerous` grant for the asking computer too: the asking
  computer sends `X-MC-Acting: machine; dangerous=1` only after such an
  approval, and the receiver intersects it with its grant. The receiver
  also classifies the command itself, before the approval (its verdict
  comes back with where the run would go): a flag on either side makes the
  approval dangerous, and the receiver checks again when the run starts. A
  dangerous run is a desktop notice on the computer that runs it.
- **It runs in a work folder, never in your checkout**:
  `machines/work/wk_…` in the data dir, one per project and asking
  computer, a detached git worktree of this computer's checkout of the
  project, reset to the snapshot the other computer sent (`git reset
  --hard`, `git clean -fd`: the files git ignores stay). The checkout's
  working tree, index, `HEAD` and branches are never written; its `.git`
  gains the worktree's entry, the snapshots' objects and refs under
  `refs/mother-claude/moves/wk_…/` (the last three). Git runs as for a
  move: from an argument list, its environment scrubbed, and a private,
  empty hooks folder for every call, so no hook of the repository runs.
- **Its environment** is scrubbed as for the sessions Mother Claude starts:
  the asking session's identity, Mother Claude's tokens, the accounts'
  tokens, API keys, provider switches and the git variables that point at
  another repository are removed; `MOTHER_CLAUDE_RUN=1` and
  `GIT_TERMINAL_PROMPT=0` are added; its input is empty. The work folder is
  not a sandbox: the command runs as you, with your rights.
- **It ends whole.** The shell starts in a process group of its own (Mac,
  Linux) or a job object (Windows). A cancel or the time limit (30 minutes
  by default, 2 hours at most) ends the whole tree (`SIGTERM`, `SIGKILL` 3 s
  later; the job terminated), and when the command ends by itself, what it
  left running in the background is ended too.
- **The audit holds no command line.** Both computers write
  `machine.run.asked`, `machine.run.start` (the computer that runs it) and
  `machine.run` (its end), and the asking one `machine.run.grant.add` and
  `machine.run.grant.remove`, with the program's name and the SHA-256 of
  the command line (of a grant's prefix), never the line itself, which can
  carry secrets — and the folder, the exit code and the duration. The runs'
  own records (`machines/work/runs/<run>/run.json`, and `output.log` on the
  computer that ran it; 0600) hold the command and its output for the Runs
  card, a day after the run ends (200 runs at most), and are then deleted.
- **Files come back only when asked**: one a run listed, at most 64 MiB,
  checked by size and SHA-256, into the asking project's
  `.mother-claude/artifacts/<run>/` — never through a link, with a
  `.gitignore` that keeps git from seeing it.
- **Taking it back stops it.** Turning Machines off here, revoking a
  computer or taking back its `run` grant (the column the run came in by)
  stops what it runs here at once.

### Handing a task over

A session's `machine_task` hands a task to a Claude Code session on another
of your computers, in its own worktree there at the session's code; what it
did comes back as a branch.

- **It needs `run`.** A session that works here runs code here, so the
  `/peer/work/tasks/*` routes check the same `run` grant for the path the
  request came over, and that Machines is on here (`403 machines_off`). A
  task is reached only by the computer that handed it over; its id, the
  project's path, the session's folder, the text (64 KiB at most), the
  permission mode, model and effort, the account's name, the branch's name,
  the commits that computer has, file names, sizes and SHA-256 hashes and
  the snapshot's record are checked before git, the file system or the
  session sees them.
- **Every task is approved, always.** A pending item on the computer that
  asks (source `machine`, `toolName: "machine_task"`) names the computer,
  its folder, the whole task, the permission mode, model, effort and
  account there. No standing grant ever covers a task — it is a free-form
  prompt — and its card offers none.
- **A session without approvals needs both rights.** `bypassPermissions`
  or `dontAsk` makes the approval dangerous (only someone holding the
  dangerous right allows it) and needs the receiving computer's
  `dangerous` grant for the asking one: the asking computer sends
  `X-MC-Acting: machine; dangerous=1` only after such an approval, and the
  receiver intersects it with its grant (checked again when the session
  starts; a desktop notice there). Only then is the session launched with
  `--allow-dangerously-skip-permissions`; in every other mode it is launched
  without it, as a session started by a caller without the dangerous right,
  and stopped if its settings bring it up without approvals anyway.
- **It works in its own worktree, never in your checkout**:
  `machines/work/tasks/<task>/<project>` in the data dir, a detached git
  worktree of this computer's checkout at the snapshot the other computer
  sent. The checkout's working tree, index, `HEAD` and branches are never
  written; its `.git` gains the worktree's entry and the snapshots' objects
  and refs under `refs/mother-claude/moves/<task>/` and `…/<task>-result/`.
  Git runs as for a move (an argument list, a scrubbed environment, a
  private empty hooks folder per call). Unlike a run, the session there is
  an ordinary owned session — not a sandbox: it reads and runs what your
  user can, through its own approvals in its permission mode, which anyone
  holding `control` for it answers from either computer. The worktree holds
  the asking project's own files, its `.claude/` settings, hooks and
  `.mcp.json` included, and the session loads them as Claude Code does in
  that project — running the project's code, which is what the `run` grant
  allows (a task's worktree is not a folder Claude Code was asked to trust
  there, and it is started as the desktop would start it).
- **Its account** is the one with the asking session's account's name,
  else the Default; a token account's token never travels, and one that is
  missing or signed out here refuses the task before anyone is asked.
- **What comes back is checked, and only ever adds a branch.** The result —
  a snapshot of the worktree, chained on the last result — is fetched into
  the asking repository and checked like every snapshot (its commits, trees,
  base, objects and manifest against its record), then becomes
  `refs/heads/mother-claude/<computer>/<task>`, a name the asking computer
  makes itself. It is never checked out and never merged; it moves only
  forward (a fast-forward), never while a worktree has it checked out, never
  over a commit someone added to it. A task that changed nothing makes no
  branch.
- **The audit holds no task text.** Both computers write
  `machine.task.asked`, `machine.task.start` (the computer that runs it),
  `machine.task` (each end) and `machine.task.remove`, and the asking one
  `machine.task.result`, with the task's id, the computer, the folder, the
  SHA-256 of the task's first line, the permission mode and the branch —
  never the task itself. The tasks' own records
  (`machines/work/tasks/<task>/task.json`, 0600) hold the task and its
  session's last message for the Runs card, and go with the task: removed
  from the card (`DELETE /api/machines/tasks/{id}`: the desktop or a paired
  device, never the local-script token), or 7 days after it last ended, its
  worktree and refs there with it — never its branch here.
- **Taking it back cancels it.** Turning Machines off here, revoking a
  computer or taking back its `run` grant (the column the task came in by)
  stops the session of each of its tasks that still works; a task whose
  turn ended keeps its session as it is.
- **An idle one stops.** A done task's session idle for 30 minutes with
  nothing to do is stopped here; a message to it — from either computer,
  under the session routes' own rules (`control`; a session without
  approvals needs the sender's dangerous right) — resumes it in its
  worktree.

### A team role on another computer

A team's template may name another computer for a role (`machine`); the
team, its board and its launcher stay on the lead's computer, and the role
runs on the other one as an owned session.

- **It needs `run`, and Machines on.** The computer that runs the role
  checks its `run` grant for the lead's computer on every `/peer/teams/
  roles/*` route (the path the request came over) and that Machines is on
  there (`403 machines_off`); it admits the role with its own capacity
  rules. A role without permission prompts needs its `dangerous` grant
  too, and the right of whoever started the team (`X-MC-Acting`), as for a
  task. Everything the lead sends — the binding, the team and role ids, the
  project's path, the session's name and charter, the role definitions,
  the guard's globs, the mode, model, effort, fallbacks, budget, file names,
  sizes and hashes — is checked before git, the file system or a session
  sees it. The session gets this computer's own settings line, team-tool
  list and guard callback, never ones from the request.
- **The binding.** The lead mints one per role and computer: an id
  (`rb_…`) and a 32-byte secret, sent once. The computer that runs the role
  keeps the secret in its record (`machines/work/roles/<binding>/role.json`,
  0600, never shown); the lead keeps only its SHA-256 in `teams.json`. The
  role session's team tools there go to the lead (`POST /peer/teams/
  board`), which answers only for a paired computer, with Machines on, a
  call naming that binding, from the computer it was minted for, for that
  team and role, with the secret (compared in constant time) and the
  role's current session. Anything else is `403 binding_refused`, audited
  (`team.remote.refused`), as if there were no such role. The board's own
  rules then apply as to a role session on the lead's computer: no task
  writing, its own tasks only, every end recorded.
- **Its worktree, never your checkout**: `machines/work/roles/<binding>/
  <project>` in the data dir, a detached worktree of that computer's
  checkout at the lead's snapshot, as for a task. It is moved to the lead's
  newer code at a start only when it holds nothing the lead has not got
  back and the lead's code already holds that; otherwise it keeps the role's
  work. The guard holds the role to its globs there (`initialize.hooks` and
  `can_use_tool`, as on the lead's computer).
- **What comes back only adds a branch.** When the role finishes a task,
  its worktree's snapshot is fetched into the lead's project and checked
  like every snapshot, then becomes `refs/heads/mother-claude/<computer>/
  <team>/<role>` — named by the lead — moved only forward, never checked
  out, never merged; the task records it.
- **Who stops it.** The lead's launcher (a stop, its idle stop, the team's
  stop); that computer's Machines page (*Stop*, and *Remove* with the
  desktop or a paired device); Machines off there, revoking the lead's
  computer or taking back its `run` (the column the role came in by); and,
  when the lead is not heard from for 30 minutes while the session sits
  idle, that computer itself.
- **The audit holds no task text**: `team.remote.start`, `.stop`,
  `.result`, `.board` (writes), `.remove` and `.refused` on the lead's
  computer; `machine.role.asked`, `.start`, `.stop`, `.result`, `.remove`
  on the other — the team, the role, the computer, the session, never a
  packet.

### Shared folders and asked actions

- Shared folders (_roots_, one folder per computer) steer where a move lands,
  so another computer's roots are merged here only when this computer gave
  it `move` (reading them needs `view`); a malformed one — a path that is not
  absolute, or holds `.`, `..` or a control character — is dropped.
- Another computer can only **ask**. A fix, a sign-in, adding an account,
  cloning a project or keeping one in sync shows here as a notice and under
  _Asked by your other computers_, checked against this computer's own state (one of its own
  checks or accounts; a clone only into a folder of one of its roots, from a
  remote without credentials, never prompting; at most 20 open asks from one
  computer), and runs only when someone confirms it on this computer's
  desktop.
- The fixes themselves run only from the desktop of the computer they change.
  The Windows firewall fix runs after the UAC prompt and adds one inbound UDP
  rule for Mother Claude's program, on the Private profile, removing the
  inbound block rules that name Mother Claude; no fix turns a firewall off.
  Shared settings another computer offers (the relay, the code options) apply
  only when this desktop accepts them.

### What this does not protect against

- A paired computer is trusted to say who acts through it: a compromised one
  can claim to be its desktop, so the grants you gave it are the limit. With
  `control` it answers ordinary approvals (a command it approves runs here);
  with `dangerous` it can switch a session to `bypassPermissions`, approve
  dangerous tool calls, send raw keys and carry a folder's trust and files
  git ignores here. Give `dangerous` only to a computer you would let act as
  this one's desktop.
- With `run`, a paired computer runs commands here as you. Each run is
  approved on _that_ computer, so a compromised one holding `run` can run
  any command it likes in its work folder here — and from there reach
  whatever your user can. Only commands the rules flag also need
  `dangerous`, and those rules are a list of known paths, not a sandbox.
  The same holds for tasks and team roles: a compromised computer holding
  `run` can start Claude Code sessions here with prompts and charters of its
  choosing (in a mode with approvals, unless it also holds `dangerous`).
  Give `run` only to a computer you would let type into a terminal here.
- _Revoke everywhere_ reaches the computers that are paired with the one
  you press it on; a computer paired only with the lost one is not told.
  The lost computer's key stays in its own secret store, and what moves left
  on it (a conversation up to its move, a worktree) stays on its disk.
- The lease binds what Mother Claude launches. A `claude --resume` you start
  by hand on the computer a session left is not stopped; what it adds is kept
  as a conflict copy when the session next moves there.
- In a session that runs in `bypassPermissions`, nobody approves a recipe's
  commands; that is one reason moving one needs the dangerous right on both
  computers.
- Relays and n0's DNS see which keys talk, when, from which addresses and how
  much; your own relay, or _No relay_, keeps that from n0. While the
  transport runs, mDNS announces this computer's key on the local network,
  and during a pairing its name and system too (the _Nearby_ list).

## Reporting

This is a research-preview-grade tool over undocumented internals. Audit before
relying on it in any shared environment.
