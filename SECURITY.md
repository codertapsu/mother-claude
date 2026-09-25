# Security

Mother Claude exposes a control plane for your Claude Code sessions. Whoever can
drive it can run code on your Mac, so every request is attributed to a
**principal** and checked against what that principal may do. The full user guide
for remote access is [docs/REMOTE.md](docs/REMOTE.md).

## Who is asking

Every listener tags its requests with the door they came through (loopback
`127.0.0.1:6725`, the LAN TLS listeners, the tunnel port, the bridge port
`127.0.0.1:5612`). Tunnels (ngrok, cloudflared, Tailscale) connect from
127.0.0.1, so the peer address alone proves nothing. Two predicates are derived
from each request:

- **machine-local**: a loopback listener, a loopback peer, exactly one `Host`
  header naming `localhost`, `127.0.0.0/8` or `[::1]`, and no proxy header at
  all (`Forwarded`, `Via`, `X-Forwarded-*`, `X-Real-IP`, `True-Client-IP`,
  `CDN-Loop`, `CF-*`, `Tailscale-*`, `ngrok-*`).
- **local**: machine-local and no `Origin`, or a local one (`tauri://localhost`,
  `http(s)://localhost`, `127.x`, `[::1]`). `Origin: null` is never local.

Any request carrying a proxy header, or reaching the LAN listener from a
non-private address, is treated as coming **through a tunnel**: it gets the
reduced route set (no `/v1`, no `/hooks/*`) and the tunnel pairing rules.

## Credentials

| credential                                                              | holder                                            | accepted                            | principal                                                                                         |
| ----------------------------------------------------------------------- | ------------------------------------------------- | ----------------------------------- | ------------------------------------------------------------------------------------------------- |
| desktop token — new on every launch, kept in memory                     | the desktop webview, via the `server_info` invoke | only from a _local_ request         | Desktop: everything                                                                               |
| local-script token — the `script-token` file in the app data dir (0600) | your own scripts                                  | only from a _local_ request         | data and control routes; never pairing, devices, remote access or plugin install; never dangerous |
| device token `mcd_…`                                                    | paired phones and browsers                        | any ingress                         | Device                                                                                            |
| hook token `mch_…`                                                      | the foreign-control plugin                        | only local, only `/hooks/*`         | Hook                                                                                              |
| sidecar token `mcs_…`                                                   | the Path A sidecar (per run)                      | only local, only its callback route | Sidecar                                                                                           |

Tokens travel as `Authorization: Bearer`. `?token=` is accepted only from a
local request (local scripts, SSE); WebSockets authenticate with their first
frame. Only SHA-256 hashes of device tokens are stored (`devices.json`, 0600).
No Mother Claude token is passed to a `claude` child process. The old master
token (the `token` file, which earlier versions put in pairing links, logs, child
environments and hook headers) is deleted on upgrade and never accepted again.

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
  for 30 days (configurable) expire. Every new device raises a notice.

## Dangerous actions

Approving `bypassPermissions` or a standing permission grant, `rm`, raw terminal
keys, PTY injection, transcript purge, starting a sign-in flow, and HTTP bridge
options that run code nobody approves (MCP servers, pre-approved tools, extra
directories, an untrusted folder, work sent into a conversation whose approvals
are off) are **dangerous**. They are allowed for:

- the desktop;
- a device you granted it in Remote access › Devices — **Allow dangerous actions
  (LAN)** applies only on the LAN; **… over the Internet** is a separate grant for
  tunnels, proxies and any other path;
- on the bridge port, an anonymous request that is _local_ (curl, Postman, a
  localhost page) — never a page on another website.

`MOTHER_CLAUDE_ALLOW_REMOTE_DANGEROUS=1` extends this to every device on the LAN
(never through a tunnel). Every dangerous action by a device raises a desktop
notice and is written to the audit log.

## TLS on the LAN

- The LAN listeners bind only private IPv4 addresses (RFC 1918 and 169.254/16;
  Tailscale's 100.64/10 when you enable it). Public IPv4 and IPv6 addresses are
  never bound; the skipped ones are listed on the Remote page.
- Certificates come from a **local CA** generated on first run (key 0600 in the
  app data dir). It is name-constrained to private addresses and `localhost`
  (Tailscale's 100.64/10 only once you bind a Tailscale address) and limited to
  TLS server certificates, so even a leaked CA key cannot impersonate a public
  site or sign mail or code. Any process running as you can read that key, so
  removing the profile on the phone is how you revoke the trust. The leaf is
  valid 397 days and re-issued whenever the LAN addresses change.
- Trust the CA on a phone (`/api/pair/ca.crt`, or `/api/pair/ca.mobileconfig` on
  iOS, then enable full trust) for a secure context: WebSockets, the service
  worker and push then work on the LAN. Otherwise tap through the warning and
  compare the fingerprint shown on the Remote page.
- Pair over the LAN only on a network you control.

## Internet access

- Off by default. When on, Mother Claude listens on `127.0.0.1:6726` (change it
  with `MOTHER_CLAUDE_TUNNEL_PORT`) for a tunnel to forward to. That port requires the last `X-Forwarded-Proto` to be
  `https`, serves no `/v1` and no `/hooks/*`, and accepts only device tokens.
  Switching Internet access off closes every connection on it.
- The public URL is read from the tunnel's own local API (ngrok, cloudflared,
  `tailscale serve status`) or entered by hand — never from request headers.
- **Tailscale Serve** (`tailscale serve --bg 6726`) keeps access inside your
  tailnet with a trusted certificate and is the recommended private option.
  ngrok, cloudflared and Tailscale Funnel make the URL public; everything behind
  it still needs a device token.
- Warnings on the Remote page: ngrok's local inspector recording requests (use
  `--inspect=false`), quick-tunnel URLs that change every run, tunnels pointing
  at 6725 (works, less defended) or at the bridge port, raw TCP tunnels.

## The Claude HTTP bridge (port 5612)

It stays tokenless and open to any origin for **machine-local** requests, so
local tools, Postman, Swagger and pages you are building can call it. A
tunnelled request needs a token (`tunneled_request_requires_token`), and while a
detected tunnel points at 5612 the tokenless mode is off entirely
(`bridge_exposed_requires_token`). The `/v1` mount on the LAN-reachable port
always needs a token.

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

## Hooks & secrets

Foreign-session control ships as a Claude Code plugin that authenticates with its
own hook token, accepted only from local requests on `/hooks/*`. The hook token
is new at every launch; the plugin's scripts read it from `hook.curlrc` (0600, in
the app data dir), which the app rewrites at startup.

Tokens, certificates and keys are created at runtime in the app data dir and are
**never committed** (`.gitignore` excludes `*.pem`, `*.key`, `*.token`).

## Reporting

This is a research-preview-grade tool over undocumented internals. Audit before
relying on it in any shared environment.
