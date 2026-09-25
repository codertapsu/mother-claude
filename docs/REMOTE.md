# Remote access

Use Mother Claude from your phone — at home over Wi-Fi, or from anywhere through
a tunnel — without turning on Claude's own Remote Control. Everything here is
configured on the desktop app's **Remote access** page. The security model behind
it is in [SECURITY.md](../SECURITY.md).

## At home: pair over the LAN

1. On the Mac, open **Remote access** and choose **Local network**. A QR code and
   a 12-character code appear; they are valid for 5 minutes and work once.
2. Scan the QR with the phone's camera (or open `https://<Mac's LAN address>:6725`
   and type the code on the Pair page).
3. The phone is paired immediately and keeps its own token. Nothing else is
   shared: the code in the QR is not a password.

LAN pairing is automatic only for a phone that reaches the Mac directly, from a
private address, over Wi-Fi or Ethernet. Use it only on a network you control.

### Trusting the Mac's certificate

The LAN listeners use a certificate from a local certificate authority that only
covers private addresses. Without trusting it the phone shows a warning you can
tap through (check that the fingerprint matches the one on the Remote page), but
some features need a trusted certificate: live updates on iOS Safari, the
Home Screen app, and push notifications.

- **iPhone / iPad**: open `https://<Mac>:6725/api/pair/ca.mobileconfig`, install
  the profile in Settings › General › VPN & Device Management, then enable it
  under Settings › General › About › Certificate Trust Settings.
- **Android**: download `https://<Mac>:6725/api/pair/ca.crt` and install it under
  Settings › Security › Encryption & credentials › Install a certificate › CA
  certificate.

The CA can only vouch for private addresses and `localhost`, so it cannot be used
against any public website even if its key were stolen.

Mother Claude binds only private IPv4 addresses. To also listen on your Tailscale
address (100.x.y.z), turn on **Also listen on Tailscale**.

## Away from home: Internet access through a tunnel

1. On the Remote page, turn on **Internet access**. Mother Claude starts
   listening on `127.0.0.1:6726` — a port that only a tunnel should reach.
2. Start a tunnel to that port. The page shows the exact commands:

   | tunnel                                                     | command                                          | URL                                   |
   | ---------------------------------------------------------- | ------------------------------------------------ | ------------------------------------- |
   | **Tailscale Serve** (recommended, private to your tailnet) | `tailscale serve --bg 6726`                      | `https://<mac>.<tailnet>.ts.net`      |
   | Tailscale Funnel (public)                                  | `tailscale funnel --bg 6726`                     | same, reachable by anyone             |
   | ngrok                                                      | `ngrok http 127.0.0.1:6726 --inspect=false`      | random, or your free static domain    |
   | cloudflared quick tunnel                                   | `cloudflared tunnel --url http://127.0.0.1:6726` | new `trycloudflare.com` URL every run |

   Mother Claude finds the public URL through each tool's local API (you can also
   enter it by hand). It never trusts a URL a request claims.

3. Choose **Internet** for the QR. The phone opens the public URL and shows **4
   digits**; type the same digits on the Mac to approve. Nobody gets in without
   you approving on the desk.

Turning Internet access off closes every connection that came through the
tunnel at once.

### Warnings you may see

- **ngrok inspector is on** — ngrok records every request (headers included) at
  `http://127.0.0.1:4040`. Restart it with `--inspect=false`.
- **Quick tunnel URL changes** — cloudflared quick tunnels get a new URL each run,
  so paired phones and push notifications break. Prefer Tailscale or ngrok's
  static domain.
- **Tunnel points at 6725** — it works (those requests are treated as remote),
  but 6726 serves fewer routes and insists on HTTPS.
- **Tunnel points at the Claude API port (5612)** — anyone who finds the URL
  could try to drive Claude. The API's tokenless mode is switched off until that
  tunnel stops.
- **Raw TCP tunnel** — hides that traffic comes from the internet; use an HTTP
  tunnel instead.

## Devices

The Remote page lists every paired device with its last address, how it
connects, its open connections and its recent actions.

- **Rename**, or **Revoke** — revoking closes the device's connections
  immediately and it has to pair again.
- **Allow dangerous actions (LAN)** — lets the device approve `bypassPermissions`
  and "always allow" rules that are written to settings, remove sessions, send raw
  terminal keys, and similar. Applies only while it is on your LAN.
- **… over the Internet** — the same, through a tunnel. Leave it off unless you
  really need it: a stolen phone would then be able to run anything on your Mac.
- Devices unused for 30 days are unpaired automatically.
- On the phone, **Settings › This device › Unpair** removes it from its side.
- The iOS Home Screen app keeps its own storage: from a paired Safari tab, use
  **Set up the Home Screen app** to get a one-time code for it.

## Notifications

On the phone, **Settings › Push notifications › Enable** subscribes this device
to Web Push. It needs a trusted HTTPS address: the tunnel's, or the LAN address
once the phone trusts the Mac's certificate. On iPhone and iPad, add Mother
Claude to the Home Screen first and enable push from there.

- Per event: permission requests, questions, plans, finished turns (30 s or
  longer), failures, notifications Claude sends itself, and idle sessions.
- Per session (the ⋯ menu): **Follow**, **Mute**, and a one-shot **Notify me
  when this turn finishes**.
- Notifications show only the session name and what happened; turn on **Show
  details** to include the command or question. Nothing is sent to a device
  that has the app open in the foreground, and a revoked device's
  subscriptions are deleted.

## Sessions started elsewhere

Sessions you start in a terminal, VS Code or JetBrains can be watched from the
phone right away. To answer and message them too, install **foreign-session
control** from **Remote › Foreign-session control** on the Mac: it installs a
small Claude Code plugin with `claude plugin install` (your `settings.json`
hooks are never touched) and can be removed the same way.

| session | message | approve / answer | interrupt | model, mode, effort |
| --- | --- | --- | --- | --- |
| started by Mother Claude | direct, with images | yes | immediately | yes |
| with the plugin | at the next tool call or turn end | yes, first answer wins | at the next tool call | no — "Continue here" |
| in tmux | typed into the pane | with the plugin, else keys | immediately (Esc) | slash commands |
| background job | experimental PTY | — | — | — |
| anything else | — | — | — | "Continue here" forks it |

Every session shows what it supports and why in its channel badge.

## For scripts

The file `script-token` in the app data directory
(`~/Library/Application Support/dev.motherclaude.Mother-Claude/script-token` on
macOS) works for scripts on this Mac as `Authorization: Bearer <token>`. It can
read and drive sessions, but it cannot pair devices, change remote access or
perform dangerous actions. The desktop app uses its own token, new at every
launch.

**Upgrading from an older version:** the old `token` file is deleted at the
first launch and its value no longer works anywhere. Older versions put it in
pairing links and QR codes, logs, every Claude process they started and the hook
entries in `~/.claude/settings.json`, so treat any copy of it as leaked. Point
your scripts at `script-token`, re-pair phones, and remove any leftover Mother
Claude hook entry that carries an `Authorization: Bearer` header from
`~/.claude/settings.json` (and its `.mc-backup`).

The Claude HTTP API on `http://127.0.0.1:5612` needs no token from this Mac; see
[BRIDGE.md](BRIDGE.md).
