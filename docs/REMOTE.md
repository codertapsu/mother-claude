# Using Mother Claude from your phone

Mother Claude puts every Claude Code session on your Mac on your phone: what each
one is doing, its agents, and the approvals, questions and plans waiting for you.
It works at home over Wi-Fi and, if you choose, from anywhere through a tunnel
you run yourself. It does not use or need Claude's own Remote Control.

Everything is set up on the desktop app's **Remote** page (sidebar → Remote). On
a phone the same page only says that it is managed from the desktop.

The security model behind this guide is in [SECURITY.md](../SECURITY.md).

- [At home: pair over Wi-Fi](#at-home-pair-over-wi-fi)
- [Away from home: Internet access through a tunnel](#away-from-home-internet-access-through-a-tunnel)
- [Devices](#devices)
- [Sessions started elsewhere](#sessions-started-elsewhere)
- [What you can do with each session](#what-you-can-do-with-each-session)
- [Notifications](#notifications)
- [The security model in plain words](#the-security-model-in-plain-words)
- [Troubleshooting](#troubleshooting)
- [For scripts](#for-scripts)

## Ports at a glance

| Port   | Listens on                                           | Used by                                         | Credential                                                           |
| ------ | ---------------------------------------------------- | ----------------------------------------------- | -------------------------------------------------------------------- |
| `6725` | `127.0.0.1` (HTTP)                                   | the desktop app, your scripts, the hooks plugin | the desktop, script or hook token (this Mac only), or a device token |
| `6725` | each private LAN address (HTTPS)                     | phones at home                                  | a device token                                                       |
| `6726` | `127.0.0.1` (HTTP), only while Internet access is on | the tunnel you start                            | a device token; HTTPS-forwarded requests only                        |
| `5612` | `127.0.0.1` (HTTP), only once the API is started     | local tools ([BRIDGE.md](BRIDGE.md))            | none from this Mac; a token for anything tunnelled                   |

`MOTHER_CLAUDE_PORT`, `MOTHER_CLAUDE_TUNNEL_PORT` and `MOTHER_CLAUDE_BRIDGE_PORT`
change them.

## At home: pair over Wi-Fi

1. On the Mac, open **Remote → Pair a device**, choose **Same Wi-Fi** and press
   **Show pairing code**. A QR code and a 12-character code appear. The code
   works once and expires after 5 minutes. With more than one network address,
   pick the one the phone can reach.
2. Scan the QR code with the phone's camera. Or open the address shown under
   the code (`https://<Mac's address>:6725`) on the phone and type the code on
   its Pair page.
3. The phone is paired immediately and opens your sessions.

The QR code carries only that one-time code, in the part of the link after
`#`, which browsers never send to a server. No password or token is in it.

A **Same Wi-Fi** code pairs only a phone that connects straight to the Mac from
a private network address (home Wi-Fi or Ethernet). Presented any other way,
for example through a tunnel, it is refused. Pair over Wi-Fi only on a network
you control.

Mother Claude listens on the Mac's private IPv4 addresses (`192.168.x.x`,
`10.x.x.x`, `172.16–31.x.x`, `169.254.x.x`) and never on a public or IPv6
address. The Remote page lists any address it skipped and why. The first time a
phone connects, macOS may ask whether Mother Claude may use the local network:
allow it.

### The Mac's certificate

At home the connection is HTTPS with a certificate from a small certificate
authority that Mother Claude creates on the Mac the first time it runs. The
phone does not know it, so it warns once ("This connection is not private").
You can tap through; the Remote page shows the certificate's fingerprint to
compare.

Until the phone trusts it, some things don't work: iOS refuses live updates over
an untrusted certificate (the app then shows **Limited** and refreshes every few
seconds instead), and push notifications and the installed Home Screen app need
a trusted address. To trust it:

- **iPhone / iPad**: on the phone's Pair page open **Trust this Mac → Download
  the profile** (`/api/pair/ca.mobileconfig`). Install it under Settings ›
  General › VPN & Device Management, then turn on full trust under Settings ›
  General › About › Certificate Trust Settings. Reload the page.
- **Android**: **Download the Mac's certificate** (`/api/pair/ca.crt`) and
  install it under Settings › Security › Encryption & credentials › Install a
  certificate › CA certificate.

This authority can only vouch for private addresses, Tailscale addresses
(`100.64.0.0/10`) and `localhost`. Even if its key were stolen it could not
impersonate a public website. The leaf certificate is renewed automatically when
the Mac's addresses change.

### The iPhone Home Screen app

An app added to the Home Screen (Safari › Share › Add to Home Screen) keeps its
own storage, separate from Safari, so it has to pair on its own. In a paired
Safari tab, open **Settings › Home Screen app › Set up the Home Screen app**. It
shows a one-time code that works for 5 minutes and only for this device; type it
on the Home Screen app's Pair page. The Remote page then shows the device "with
its Home Screen app".

The same code works for any second address of the same phone. A pairing
belongs to the address it was made on, because browser storage is per site, so
a phone paired at home that later opens your tunnel URL (or your Tailscale
address) starts unpaired there. Create a code in Settings where you are paired
and type it at the new address.

### Tailscale addresses

With **Also listen on Tailscale** (Remote › Internet access) the dashboard is
served on the Mac's Tailscale address (`100.x.y.z`) as well. A request that
arrives there is treated like one from the internet: a **Same Wi-Fi** code is
refused, and the device's **… over the internet** grant applies. Pair on Wi-Fi
first and add the Tailscale address with a code from Settings (above), or use
Tailscale Serve (below), which gives a real certificate.

## Away from home: Internet access through a tunnel

Mother Claude never opens a port to the internet itself. You run a tunnel
(Tailscale, ngrok or Cloudflare) on the Mac, and it forwards to a port that
only listens on `127.0.0.1`.

1. On the Remote page, under **Internet access**, turn on **Allow access
   through a tunnel**. Mother Claude starts listening on `127.0.0.1:6726`. It
   is off by default.
2. Start one of these on the Mac. The Remote page shows each command with a
   copy button and marks the tools it found installed:

   | Tunnel                                               | Command                                          | Address you get                                             |
   | ---------------------------------------------------- | ------------------------------------------------ | ----------------------------------------------------------- |
   | **Tailscale Serve** (recommended; your tailnet only) | `tailscale serve --bg 6726`                      | `https://<mac>.<tailnet>.ts.net`                            |
   | ngrok                                                | `ngrok http 127.0.0.1:6726 --inspect=false`      | random, or your static domain with `--url https://<domain>` |
   | Cloudflare quick tunnel                              | `cloudflared tunnel --url http://127.0.0.1:6726` | a new `*.trycloudflare.com` address every run               |
   | Tailscale Funnel (public)                            | `tailscale funnel --bg 6726`                     | `https://<mac>.<tailnet>.ts.net`, open to anyone            |

   Tailscale Serve and Funnel need HTTPS certificates turned on for your
   tailnet; Tailscale asks the first time. Serve is recommended because only
   devices signed in to your tailnet can reach the address at all.

3. Mother Claude finds the public address by itself by asking the tunnel tools
   running on the Mac (every 10 seconds while the Remote page is open or
   Internet access is on; Tailscale every 30):
   - ngrok: its local agent API on `127.0.0.1:4040` (or `4041`);
   - cloudflared: the quick-tunnel metrics on `127.0.0.1:20241–20245`;
   - Tailscale: `tailscale serve status --json` (Serve and Funnel).

   A named Cloudflare tunnel or any other proxy is not detected. Type its
   `https://` address under **Or enter it yourself**. Mother Claude never
   believes an address that a request claims for itself.

4. Under **Pair a device**, choose **Internet** and press **Show pairing code**.
   Scan it with the phone (on mobile data, say). The phone opens the public
   address and shows **4 digits**.
5. On the Mac, a **Waiting for your approval** card appears (a dialog pops up
   wherever you are in the app). Press **Approve…** and type the 4 digits the
   phone shows. Three wrong tries deny the request. Nobody gets in through the
   internet without you typing those digits at the Mac.

Turning Internet access off stops the tunnel port and cuts every connection that
came through it, live updates included. A tunnel you pointed at `6725` instead
keeps working, because it bypasses that port, so point tunnels at `6726`.

### Warnings on the Remote page

- **ngrok's inspector is recording.** By default ngrok keeps every request,
  headers included, at `http://127.0.0.1:4040` for anything on the Mac to read.
  Restart it with `--inspect=false`.
- **The quick tunnel's address changes.** A new `trycloudflare.com` address on
  every run means phones have to pair again and push notifications stop. Use
  Tailscale, ngrok's free static domain or a named Cloudflare tunnel.
- **Tailscale Funnel is public.** Its name is published in certificate logs and
  gets scanned; for your own devices use Serve.
- **A tunnel points at 6725.** It works (those requests are treated as coming
  from the internet), but 6726 serves fewer routes and insists on HTTPS.
- **A tunnel points at the Claude API port (5612).** Anyone who finds the address
  could try to drive Claude. The API's tokenless mode is switched off until that
  tunnel stops.
- **A raw TCP tunnel.** It hides that traffic comes from the internet (to 6725),
  or cannot work at all (to 6726). Use an HTTP tunnel.
- **Internet access is on but no tunnel was found**, or **a tunnel points at 6726
  while Internet access is off**.
- **The tunnel port is not listening**: another program holds 6726. Set
  `MOTHER_CLAUDE_TUNNEL_PORT` to another port.

## Devices

**Remote → Paired devices** lists every device with its platform, how it last
connected (local network or internet), when and from which address, and its
open connections.

- **Rename** (the pencil) or **Revoke**. Revoking erases the device's tokens,
  closes its connections at once and deletes its push subscriptions. It has to
  pair again.
- **Allow dangerous actions (LAN)** lets this device do what is otherwise
  reserved for the Mac: switch a session to `bypassPermissions`, choose "always
  allow" rules that are saved to settings, approve a tool call flagged
  dangerous, remove sessions, send raw keys to a terminal, type into a session
  that runs without permission prompts, and similar. It applies only while the
  device is on your home network.
- **… over the internet** is the same grant for requests through a tunnel,
  proxy or Tailscale address. Leave it off unless you really need it: with it, a
  stolen phone could run anything on your Mac.
- Devices unused for 30 days are unpaired automatically
  (`MOTHER_CLAUDE_DEVICE_IDLE_DAYS` changes that).
- On the phone, **Settings › Connection › Unpair this device** removes it from
  its own side.

Every new device and every dangerous action by a device raises a notice on the
Mac and is written to the audit log (see [SECURITY.md](../SECURITY.md#logs-and-audit)).

## Sessions started elsewhere

Mother Claude shows every Claude Code session on the Mac, whatever started it:
terminal, VS Code, JetBrains, Claude Desktop, background jobs (`claude --bg`)
and its own. It learns about running sessions from the small status files each
Claude Code process writes under `~/.claude/sessions/`, and reads what they did
from the transcripts under `~/.claude/projects/`.

Sessions that Mother Claude starts ("owned") can be driven completely. The ones
you started elsewhere ("foreign") can be watched right away. To answer and
message them too, install **foreign-session control**.

### Foreign-session control (the hooks plugin)

On the Mac, open **Remote → Foreign-session control → Install foreign-session
control**. This needs the `claude` command on your PATH and runs, on your
behalf:

```bash
claude plugin marketplace add "<app data>/claude-plugin" --scope user
claude plugin install mother-claude@mother-claude-local --scope user
```

(`<app data>` is `~/Library/Application Support/dev.motherclaude.Mother-Claude`
on macOS.) The Claude Code CLI records the plugin in your user settings
(`enabledPlugins`, `extraKnownMarketplaces`). Mother Claude never edits your
`settings.json` and never touches your own hooks.

The plugin is two small shell scripts and a `hooks.json`:

- `mc-hook` forwards Claude Code's hook events (session start and end, prompts,
  tool calls, permission requests, subagents, compaction, notifications, stop)
  to Mother Claude on `127.0.0.1`, and hands back its answer where one is
  needed: an approval, a message to deliver, an interrupt.
- `mc-wake` waits in the background after a turn ends, so a message you send to
  an idle session can wake it up.

Both run `curl` against loopback only, authenticate with a token that is new at
every launch of the app, and always exit quietly: with Mother Claude closed,
your sessions behave exactly as before. Sessions Mother Claude runs itself are
skipped, so nothing is reported twice.

After installing, **new** sessions report in. A session that was already
running needs `/reload-plugins` typed in it, or a restart. The Remote page
counts the sessions reporting and those that need a reload.

**Uninstall** (press twice) first switches the scripts off, so the hooks that
running sessions already loaded go quiet, then runs `claude plugin uninstall`
and `claude plugin marketplace remove`. The files stay in the app data folder,
inert. Install and uninstall work only from the desktop app itself, never from a
phone.

### How each kind of session is reached

- **Owned** (started from **New session**, or taken over with **Continue
  here**): Mother Claude runs `claude` itself and speaks its control protocol,
  the way the VS Code extension does. Everything works, immediately, and replies
  stream word by word.
- **With the plugin** (terminal, VS Code, JetBrains, Claude Desktop): a
  permission request is held for the phone _while_ the session shows its own
  dialog. Whoever answers first wins, and the other side's prompt goes away.
  Messages are delivered at Claude's next tool call or at the end of the turn;
  an idle session is woken to read them. They reach Claude but don't appear in
  the session's own window, only Claude's reply does. An interrupt lands at the
  next tool call.
- **In tmux**: Mother Claude types into the session's pane (`tmux send-keys`),
  so messages, slash commands and Esc work like typing at the terminal. It
  refuses while a dialog is open, so it never answers one by accident. Approvals
  go through the plugin when it is loaded; without it they can be answered with
  keys from the terminal view.
- **Background jobs** (`claude --bg`): stop, respawn and remove (by their short
  job id), a read-only terminal view (`claude logs`), and, experimentally,
  typing into them through `claude attach`, which needs a device allowed
  dangerous actions (`MOTHER_CLAUDE_FOREIGN_INJECTION=0` turns it off).
- **Anything else** can be watched. **Continue here** takes it over as an owned
  session: a session that is still running elsewhere is **forked** (a copy with
  a new id, so the two never write the same transcript); an ended one resumes
  in place.

Each session's badge ("Full control", "Via hooks", "Via tmux", "Experimental",
"Watch only") opens the list of what works for it and why, with the action that
fixes each limit (install the plugin, reload plugins, continue here).

## What you can do with each session

| From the phone                        | Owned                         | With the plugin (VS Code, terminal, JetBrains, Desktop)  | tmux, with the plugin | tmux, without it      | Without the plugin       |
| ------------------------------------- | ----------------------------- | -------------------------------------------------------- | --------------------- | --------------------- | ------------------------ |
| Watch the conversation, agents, cost  | yes, word by word             | yes, message by message                                  | yes                   | yes                   | yes                      |
| Send a message                        | straight in                   | at the next tool call or turn end; wakes an idle session | typed into the pane   | typed into the pane   | —                        |
| Attach images and files               | images inline, files as paths | as file paths                                            | as file paths         | as file paths         | —                        |
| Approve or deny a permission          | yes                           | yes, in parallel with the desk (first answer wins)       | yes                   | with keys ¹           | —                        |
| Answer questions, review plans        | yes                           | yes                                                      | yes                   | with keys ¹           | —                        |
| Interrupt                             | immediately                   | at the next tool call                                    | immediately (Esc)     | immediately (Esc)     | —                        |
| Switch model or effort                | yes                           | no ²                                                     | `/model`, `/effort` ³ | `/model`, `/effort` ³ | —                        |
| Switch ultracode or thinking          | yes                           | no ²                                                     | no ²                  | no ²                  | —                        |
| Switch permission mode                | any time                      | only while approving (e.g. a plan)                       | only while approving  | —                     | —                        |
| Slash commands                        | yes                           | no ²                                                     | yes ⁴                 | yes ⁴                 | —                        |
| Rename                                | yes                           | —                                                        | yes (`/rename`)       | yes (`/rename`)       | —                        |
| Terminal view                         | —                             | —                                                        | yes                   | yes                   | background jobs only     |
| Stop a background task (shell, agent) | yes                           | —                                                        | —                     | —                     | —                        |
| Continue here                         | as a fork                     | as a fork                                                | as a fork             | as a fork             | fork, or resume if ended |

1. Keys (Enter, Esc, arrows, digits) need a device allowed dangerous actions, or the Mac.
2. **Continue here** forks the session into an owned one, which can.
3. Typed into the terminal, these also change your default for new sessions.
4. `/compact`, `/clear`, `/context`, `/usage`, `/cost`, `/status`, `/todos`,
   `/model`, `/effort`, `/rename` and `/btw`. Others (and a leading `!` for shell
   mode) need a device allowed dangerous actions.

Two more rules apply everywhere:

- **Sessions without permission prompts.** A session in `bypassPermissions` runs
  whatever it is told with nobody approving it, so messages, typed text and keys
  sent to it need a device allowed dangerous actions. It also asks almost
  nothing, so with the plugin only its questions, plans and the few actions no
  mode approves automatically reach the phone.
- **New sessions and Continue here from a phone** work only in folders Claude
  Code has already been trusted in on this Mac (opened there once, or with
  transcripts). Otherwise you get `untrusted_cwd`: open the folder once on the
  Mac first.

### Effort, ultracode, thinking and fast mode

Each session shows where these values come from, because Claude Code reports
them differently for each kind of session:

- **Sessions Mother Claude owns** ask Claude Code what it actually applies
  (`get_settings`, no API call) after it starts, after every change and after
  each turn. Ultracode and thinking are switched there as session-only
  settings, never through `/effort`, which would also change your default for
  new sessions. Ultracode shows **unavailable** when you asked for it but the
  model or workspace can't run it; the request stands (it takes effect on a
  model that can run it) until you switch it off. Until Claude Code confirms a
  change, the new value shows as requested. A model without effort levels
  (Haiku) shows no effort. Nothing reports thinking back, so the app shows
  what it set.
- **Other sessions** show the effort of their last message, or the effort the
  plugin reported for a later turn — none once a message went out on a model
  without effort levels, such as Haiku. Ultracode shows **as of the last
  message** sent in the session: Claude Code notes it in the conversation
  only when a message is sent (a marker when it changes, nothing while it
  stays the same), so a switch made in VS Code shows after the next message
  there. Until then it shows as unknown, with the reason: a new conversation
  has no message yet, and a restarted session starts with ultracode off
  (VS Code doesn't switch it back on), so what it was before the restart
  says nothing about now. Thinking shows as launched, read from the process's
  own arguments — VS Code can switch thinking later without anything
  recording it.
- **Fast mode** shows when the last message ran in fast mode.

### Plan usage

The sessions page shows your claude.ai plan's limits (the current 5-hour session
and the weekly windows) with their reset times. It is read automatically only
while a dashboard is connected (a manual Refresh reads even when none is):

- when a dashboard connects and the numbers are older than the refresh
  interval;
- every **5 minutes** (Settings › Plan usage refresh: 3, 5 or 10 minutes, or
  manually only) while some session is working, every 30 minutes otherwise;
- once when a session hits a limit, and when you tap Refresh (at most once a
  minute).

Sessions Mother Claude owns report the 5-hour and weekly numbers for free
whenever they change; the other windows and extra usage keep the time of the
last full read. Otherwise the app asks Claude Code through an owned session, or
starts a short-lived `claude -p` of its own for the read (no tools, no model
call, no conversation saved; it never shows as a session). Until the first
read, and while reads fail, it shows Claude Code's own last reading from
`~/.claude.json` when it is less than an hour old and belongs to the signed-in
account. Nothing is shown for API-key or third-party-provider sign-ins. An
owned session started in a project whose settings sign in differently (its own
`env` or `apiKeyHelper`) is never used for these numbers.

The short-lived `claude -p` is the one exception to "Mother Claude never writes
under `~/.claude`": the app itself still writes nothing there, but Claude Code,
while it runs, writes its usual pidfile under `~/.claude/sessions` (removed when
it exits) and refreshes its own usage and feature caches in `~/.claude.json`.
It runs only when no owned session can answer: for the automatic reads above,
which need a connected dashboard, and for a Refresh. With **Manually only**, it
starts only for a Refresh.

## Notifications

Push notifications reach a phone even when the app is closed. On the phone,
open **Settings › Notifications › Push notifications › Enable**.

They need a **trusted HTTPS address**, because browsers allow push only in a
secure context:

| Where you enable them                                    | Push works?                                                                                     |
| -------------------------------------------------------- | ----------------------------------------------------------------------------------------------- |
| Your tunnel address (Tailscale Serve, ngrok, Cloudflare) | yes: its certificate is trusted                                                                 |
| The home-network address, Mac certificate trusted        | yes, but tapping a notification opens that address, which only works at home                    |
| The home-network address, certificate not trusted        | no: in-app alerts only (sound, vibration, the waiting count in the title) while the app is open |
| iPhone / iPad in Safari                                  | no: add Mother Claude to the Home Screen, open it from there, pair it, then enable push         |
| The desktop app                                          | not needed: the Mac shows everything live                                                       |

The notifications travel from the Mac through Apple's, Google's or Mozilla's
push service, so the Mac has to be awake and online. A quick-tunnel address
that changes breaks existing subscriptions.

What you get:

- **Per event** (Settings, once push is on): permission requests, questions,
  plans, finished turns (30 s or longer), failures, notifications Claude sends
  itself ("tell me when the tests finish"), and sessions waiting for your next
  prompt.
- **Per session** (the session's ⋯ menu): **Follow**, **Mute**, and a one-shot
  **Notify me when this turn finishes**.
- A notification shows the session's name and what happened. **Show details**
  adds the command or question.
- Nothing is pushed to a device that has the app open in front of you, and bursts
  from one session are combined (at most one notification per session every
  10 seconds, followed by a summary).
- **Send a test** checks the whole path.

## The security model in plain words

- **Your Mac is the server.** Sessions, transcripts and tokens stay on it. Nothing
  goes through a Mother Claude cloud; there isn't one.
- **Every phone has its own key.** Pairing gives each device a random token that
  only it holds. The Mac keeps only a hash of it, and you can revoke any device
  on its own.
- **Codes, not passwords.** Pairing codes work once, for 5 minutes. Through the
  internet a code is not enough: you also type the 4 digits the phone shows,
  on the Mac.
- **Being "local" is proven, not assumed.** Tunnels connect from `127.0.0.1`, so
  Mother Claude looks at the listener, the `Host` header and every proxy header
  before it treats a request as coming from this Mac. A request through a tunnel
  is always treated as coming from the internet.
- **The risky things stay on the Mac.** Pairing devices, changing remote access
  and installing the plugin work only in the desktop app. Skipping permission
  prompts, saving "always allow" rules to settings, removing sessions and raw
  terminal keys need the desktop app or a device you granted dangerous actions
  (separately for home and for the internet). A phone without that grant is
  offered "always allow … in this session" instead.
- **Only you open the door to the internet.** Internet access is off until you
  switch it on and start a tunnel, and switching it off cuts the tunnel's
  connections.
- **The local API stays local.** The Claude HTTP API on port 5612 needs no token
  only for programs on this Mac. Through a tunnel it asks for a token, and while
  a tunnel points at it the tokenless mode is off entirely.

## Troubleshooting

**"This code is for your home network and cannot be used through a tunnel or
proxy."** You scanned a **Same Wi-Fi** code over mobile data, a tunnel or a
Tailscale address. Create an **Internet** code instead.

**"This code was already used by another device."** A code was presented twice.
A request made with it can no longer be approved; deny it and create a new code.

**"Those digits do not match the ones on the phone."** Type the 4 digits the
phone shows. After three wrong tries the request is denied and the phone has to
start again.

**The phone shows "Limited".** Live updates cannot connect, usually because the
phone does not trust the Mac's certificate, and the app is refreshing every few
seconds instead. Trust the certificate (see above) or use the tunnel address.

**"Can't reach Mac".** The Mac is asleep or offline, the tunnel stopped, or a
quick tunnel's address changed. Keep the Mac awake while you're away.

**"The tunnel's browser warning page is in the way."** ngrok's free plan shows an
interstitial page. Open the address once in the phone's browser, accept it and
reload.

**`https_required` (400) on the tunnel port.** The request did not arrive over
HTTPS (the last `X-Forwarded-Proto` was not `https`). Open the `https://`
address the tunnel prints, and point the tunnel at plain
`http://127.0.0.1:6726`.

**"No public URL yet".** No tunnel was detected. Start one of the commands on
the Remote page, or type the address under **Or enter it yourself** (needed for
named Cloudflare tunnels and other proxies).

**"Mother Claude is not listening on your local network."** It was started with
`MOTHER_CLAUDE_HOST=127.0.0.1`, or the Mac has no private network address (for
example it is on a network that hands out public addresses). Use a tunnel, or
**Also listen on Tailscale**.

**"This device is no longer paired."** The device was revoked on the Mac, unused
for too long, or its storage was cleared. Pair it again.

**A foreign session says "Started before foreign control was installed".** Type
`/reload-plugins` in it, or restart it.

**A message stays "queued".** The session is busy and has not reached a tool call
or the end of its turn yet, or it is idle but no wake listener is connected (the
Mac tells you after 10 seconds). Cancel it from the queue if you change your
mind. A message nothing picks up within 30 minutes is dropped with a notice, and
anything still queued fails when the session ends (`session_ended`: continue it
here instead).

**`session_waiting` when typing into a tmux session.** It is showing a dialog;
typing now would answer it. Answer the dialog first, or open the terminal view.

**`session_not_foreground` when typing into a tmux session.** Claude is not the
program in front in that pane (a shell, an editor or pager it opened, or Claude
suspended with Ctrl+Z), so typing there could run a command. Bring Claude Code
back to the front in its terminal first.

**`dangerous_blocked`.** That action needs the Mac, or a device allowed dangerous
actions on the network it is using now.

**`untrusted_cwd`.** Claude Code was never opened in that folder on the Mac. Open
it there once, or start the session from the desktop.

**Push can't be enabled.** "Blocked in the browser's site settings": allow
notifications for the site. On iPhone: add Mother Claude to the Home Screen and
enable push from there. On the home network: trust the Mac's certificate first.

**"Install foreign-session control" is greyed out.** The `claude` command was not
found on the PATH the app sees. Install Claude Code, or start the app from a
shell where `claude` works.

## For scripts

The file `script-token` in the app data directory
(`~/Library/Application Support/dev.motherclaude.Mother-Claude/script-token` on
macOS) works for scripts on this Mac as `Authorization: Bearer <token>` against
`http://127.0.0.1:6725/api/…`. It can read and drive sessions, but it cannot
pair devices, change remote access, install the plugin or take dangerous
actions, and it is refused from anywhere but this Mac. The desktop app uses its
own token, new at every launch and never written to disk.

**Upgrading from an older version:** the old `token` file is deleted at the
first launch and its value no longer works anywhere. Older versions put it in
pairing links and QR codes, logs, every Claude process they started and the hook
entries in `~/.claude/settings.json`, so treat any copy of it as leaked. Point
your scripts at `script-token`, re-pair phones, and remove any leftover Mother
Claude hook entry that carries an `Authorization: Bearer` header from
`~/.claude/settings.json` (and its `.mc-backup`).

The Claude HTTP API on `http://127.0.0.1:5612` needs no token from this Mac; see
[BRIDGE.md](BRIDGE.md).
