# Machines: one Mother Claude across a Mac, a Windows desktop and more

**Status: proposal, its open questions decided on 2026-11-10 (§14). Nothing
here is built yet.** It is grounded in what v0.10.2 already has, and marks
what a spike must confirm first.

Words used below:

| Word | Meaning |
|---|---|
| **machine** | one install of Mother Claude on one computer (its id: `backup::ops::machine_id_in`, made once per install) |
| **peer** | another machine this one is paired with |
| **home** | the machine a session runs on — the only one that may write its transcript |
| **move** | handing a session's home to another machine: its conversation, its code and its folder |
| **copy** | a machine's read-only replica of another machine's session, kept in Mother Claude's data dir |

("Fleet" already means this machine's sessions in the code, so the feature is
called *Machines*.)

---

## 0. In one page

**What you get**

1. **Pair machines once** — a QR code or a short code on one, the same
   six digits shown on both to confirm. On the same network or across the
   internet, with no port opened and no tunnel to run.
2. **See and drive every machine's sessions from any machine** — the
   Windows desktop's sessions appear in the Mac's list with a badge; open one,
   send a message, answer its approvals, stop it, change its model, exactly
   as if it ran on the Mac. A phone paired with either sees all of them.
3. **Move a session** — *Continue on DESKTOP-PC*: the conversation, the
   uncommitted code (as a git snapshot) and the folder mapping go in one step;
   it resumes on Windows, and you keep driving it from the Mac if you like.
   *Bring back* is the same in reverse. Marking a session *Keep a copy on
   DESKTOP-PC* refreshes that copy after every turn, so a move is instant and
   still works when the Mac is asleep.
4. **Later: work on another machine without moving** — a session on the Mac
   asks Windows to build and test (`machine_run`), or hands it a
   Windows-only task (`machine_task`) that runs in its own worktree there and
   comes back as a branch to review. Team roles can be bound to a machine.

**The decisions (D1–D11)**

| | Decision | Why |
|---|---|---|
| D1 | **One writer per session, across machines.** A session moves; it is never co-edited or merged. A lease with an epoch names its home. | A transcript is an append-only log where every turn depends on the one before: two writers produce two histories that cannot be merged. The same rule already holds on one machine ("never two writers on one transcript"). |
| D2 | **Seeing is remote, not copied.** The Mac shows Windows' sessions by asking Windows' Mother Claude, through the API the phone already uses. | Live, exact, no file sync, nothing to reconcile. |
| D3 | **Copies live in Mother Claude's data dir.** A config directory (`~/.claude`) is written only when you move or continue a session, through the existing pull code (undo copy, atomic writes, audit, refusal while a process uses it). | Keeps the hard rule "nothing writes to a config directory in the background, ever". |
| D4 | **Code travels through git.** A snapshot commit of the working tree (uncommitted and untracked files included, ignored files not), sent machine to machine as a git bundle or pushed to a hidden ref on your remote; never merged automatically. | Git already understands line endings, renames, ignores and history; file sync would carry `node_modules`, build output and CRLF noise. |
| D5 | **Paths are mapped, never rewritten.** Folder pairs (they exist) place the session in the right project; the transcript keeps its old paths; the session is told about the move when it resumes. | Rewriting history changes what Claude remembers and breaks the byte-for-byte continuation check the pull relies on. |
| D6 | **Machines are principals.** `Principal::Peer`, with per-peer grants (view, control, move, run, dangerous — on the LAN and over the internet separately), the acting person carried along, everything audited on the machine that acts. | Same model as paired phones, which already works and is understood. |
| D7 | **One transport for LAN and internet.** QUIC between machine keys (the `iroh` library): direct on the LAN, hole-punched or relayed across networks, end-to-end encrypted, no inbound port. The existing axum API runs over it unchanged (`Ingress::Peer`). | One code path; works across networks without a tunnel or a VPN; identity is a public key, not an address. Spike-gated (§12, Phase 0). |
| D8 | **Writing a project is an explicit act.** Mother Claude's own code writes a project's working tree or `.git` only when you move a session with its code or approve a machine task, through git plumbing, with an undo ref. | Today Mother Claude never writes a project; a move that carries code has to, so it gets the same rules as writing a config directory. |
| D9 | **One app on every OS.** No separate builds; platform differences stay behind `cfg`. A headless mode for a build box comes later. | The app is already cross-platform. |
| D10 | **Both ways, a default for each.** Relays: n0's public relays by default, or your own relay. Code: machine to machine (a git bundle) by default, or through your git remote. Landing the code: the target's checkout when it is clean by default, or always a separate worktree. Chosen in the setup (§5.14) for all machines, changeable per project and per move. | Each default works with nothing more to set up; each alternative covers a real case (a private relay, machines rarely online together, a checkout you never want touched). |
| D11 | **Nothing before setup passes.** *Set up Machines* runs the same checks on every machine — OS permissions, firewall, network profile, sleep, folders, git, Claude Code and its sign-ins, accounts, pairing, grants — fixes each one or opens the right settings page, and ends with a test between every pair of machines. The feature unlocks only when no blocker is left, and the checks keep running as *Machines health*. | A move that fails half-way on a firewall prompt or an expired sign-in on the other machine is the worst way to find out. |

**The order**: Phase 0 (spikes) → 1 (set up and pair) → 2 (see and drive) →
3 (move, then keep copies) → 4 (work on another machine) → 5 (more machines,
polish). Phases 0–3a replace the NAS round trip and give remote control; that
is the first release worth shipping.

---

## 1. The scenario

Today, to continue a task from the MacBook on the Windows desktop:

1. **Back up now** on the Mac, to the NAS (`/Volumes/NAS`, a Windows share).
2. **Force pull** on Windows.
3. Pull the code from GitHub on Windows (after pushing it on the Mac).
4. *Continue* the session on Windows.

It works, but it is four manual steps on two machines, the code and the
conversation travel separately, and there is no way to watch or drive the
Windows session from the Mac.

**Requirements**

| | Requirement |
|---|---|
| R1 | Continue a session on another machine without a manual backup and pull. |
| R2 | See and control a session that runs on another machine, from this one (and from a phone). |
| R3 | Projects live in different folders on each machine (`/Users/marcus/development/projects/app` ↔ `D:\development\projects\app`). |
| R4 | The code follows the session, uncommitted work included. |
| R5 | Same network first; then across the internet, each machine on its own network. |
| R6 | More than two machines. |
| R7 | Never lose work: nothing overwritten silently, everything undoable. |
| R8 | At least as safe as pairing a phone today. |
| R9 | Platform-specific work (a Windows build, a Windows-only feature) without leaving the Mac. |
| R10 | Still works offline: the NAS backup stays as the path when machines are not online together. |

---

## 2. What v0.10.2 already gives us

Most of the hard parts exist; what is missing is a live channel between two
Mother Claudes, and a transaction that moves a session and its code together.

| Capability | Where | Used here for |
|---|---|---|
| A session's archive: transcript, subagents', workflows', file-history, session env, todos | `claude/backup.rs` (`archive_session`) | the payload of a move and of a copy |
| "Replace only when the incoming transcript continues this one, byte for byte; else keep a conflict copy beside it" | `claude/backup.rs` (`file_is_prefix_of_file`), pull | accepting a move, a copy, a take-back |
| Folder pairs: longest match, Windows case- and separator-insensitive, both machines may edit, removals stick | `claude/backup/pairs.rs`, `claude/paths.rs` (`view_cwd`, `project_dir_name`) | mapping a session's folder to the target |
| *Continue* with **Choose its folder on this PC**, and *Pair …* from the answer | pull / continue | a move to a folder no pair maps |
| A machine id per install (Migration Assistant aware) | `claude/backup/ops.rs` (`machine_id_in`) | machine identity |
| Stop, reap and resume an owned session in place | `claude/switch.rs` (account switch) | quiescing a session before it moves |
| One-time pairing codes, per-device tokens (hash kept), grants for LAN and internet, revoke, audit, desktop notices | `server/pairing.rs`, `server/devices.rs`, `server/auth.rs` | the model for peers |
| LAN HTTPS with a local CA, Tailscale listening, tunnels with 4-digit approval | `server/tls.rs`, `server/listeners.rs`, `server/remote.rs` | fallback transport (§5.3) |
| `/ws` diff stream of sessions, transcripts and pending items | `server/monitor.rs`, `server/ws.rs` | the federated view |
| Teams: boards, task packets, launcher, Claude Code worktrees | `orchestration/` | delegating work to another machine |
| The MCP tool registry | `mcp/` | `machine_*` tools |

**Missing**: machine-to-machine trust and transport; a federated view and a
proxy for actions; a cross-machine lease; a move transaction that carries code;
copies kept current; work on another machine.

---

## 3. Analysis

### 3.1 A session is a single-writer log

A Claude Code transcript is append-only JSONL, and each turn is computed from
everything before it. If the Mac and Windows both continue the same session,
the two histories diverge after their common prefix and there is no meaningful
merge — the backup's pull already recognises this and keeps a conflict copy.

So "syncing a session in real time" cannot mean two live writers. It means:

- **one writer** (the home) at a time, and an explicit **move** of the writer;
- **live viewing** of that writer from every other machine (§5.6);
- optionally, **copies** on other machines kept current after each turn, so a
  move needs no transfer and survives the home going to sleep (§5.9).

### 3.2 Seeing is not having

Watching and driving the Windows session from the Mac needs no copy at all:
Windows' Mother Claude already serves the session list, transcripts as they
grow, pending approvals and every action over HTTP and WebSocket — a phone
uses exactly that. The Mac's Mother Claude becomes a client of Windows' (and
the reverse), and merges what it gets into its own list. No file is copied,
nothing is reconciled, the view is as live as on the machine itself.

### 3.3 The conversation and the code must move together

The transcript is Claude's memory; the working tree is the world it acted on.
Moving one without the other gives a session that remembers edits it can no
longer see. A move therefore carries both, as one transaction (§5.8): the
session's archive and a git snapshot of its project, applied together, undone
together.

### 3.4 The config directory rule

`CLAUDE.md`: *Nothing writes to a config directory in the background, ever.*
Continuous replication into `~/.claude` would break it. So copies land in
Mother Claude's data dir (`<app data>/machines/copies/…`), where nothing of
Claude Code reads them; only a move or a *Continue* — explicit acts — writes
the config directory, through the pull code that already has the safety rules
(undo copy, atomic temp + rename, refusal while a live process uses a shared
file, audit line).

### 3.5 Paths

Transcripts hold absolute paths: every record's `cwd`, file paths in tool
calls, shell commands. Claude Code finds a session by the folder it is
resumed in (`projects/<encoded cwd>/<id>.jsonl`), which the folder pairs
already handle. The old paths inside the history stay — rewriting them would
change what Claude remembers, break the byte-for-byte continuation check, and
the backup already decided "the transcripts are never rewritten". Instead the
session is told, once, in **a message of the conversation**: the first
message after a move starts with *this conversation moved from macOS
(`/Users/marcus/dev/app`) to Windows (`D:\dev\app`); paths in earlier messages
refer to the old machine; the shell here is PowerShell / Git Bash*, prepended
to your next message so it costs no extra turn. Not the system prompt: spike
S3 found that Claude Code (2.1.322) records a conversation's first system
prompt in the transcript and replays it on every resume, so an
`--append-system-prompt` given on resume never reaches the model
(`spikes/machines/RESULTS.md`). A message is recorded, survives every later
resume, and works for owned and foreign continuation alike.

### 3.6 Platform differences that bite

| Difference | Rule |
|---|---|
| Line endings (`core.autocrlf`, CRLF files) | Recommend `.gitattributes` `* text=auto eol=lf` per project; the move's preflight warns when the two machines' settings would show the snapshot as changed. |
| Build output, dependencies (`node_modules`, `target/`, `.venv`, `DerivedData`) | Never travel (they are ignored by git). A per-project **recipe** says what to run after a move on each OS (`npm ci`, …), run only when you approve. |
| Shells (zsh vs PowerShell / Git Bash) | Said in the move note (§3.5). |
| Case-insensitive file systems, 260-character paths | Preflight warns about two paths differing only in case, and about long paths on Windows. |
| Tool versions (Claude Code CLI, Node, Rust, Xcode) | The Machines page shows each machine's versions; a move warns when the target's Claude Code is older than the source's. |
| Things only one OS has (Xcode, iOS simulators; MSVC, Windows SDK) | That is what R9 is for: run that work where it can run (§5.11). |
| Secrets in ignored files (`.env.local`) | Not carried unless the project lists them, per project, opt-in, shown in the move dialog. |

### 3.7 Networks

On the LAN, two machines can reach each other directly; across the internet
they usually sit behind NATs with no inbound port. Today the app solves the
phone case with a loopback listener behind a tunnel the user starts; for
machine to machine that would mean a tunnel on every machine and a public
endpoint per machine. A peer-to-peer transport that dials *by key* — direct
when it can, hole-punched when it can, relayed when it must — covers both cases
with one code path and no exposure (§5.3).

### 3.8 Failure

Machines sleep, Wi-Fi drops mid-transfer, clocks drift. So: a move is a
transaction with a fencing **epoch** (not timestamps); every step is
idempotent and can be retried; nothing is deleted on the source; a failure
before the target has landed the session leaves both machines as they were;
a failure after leaves the target with a copy, not a running session.

---

## 4. Options considered

| Option | Live view & control | Moving a session | Code | Across networks | Verdict |
|---|---|---|---|---|---|
| **Today**: backup to NAS, force pull | no | manual, 2 machines | separate (git) | cloud folder | **Keep** as the offline path (R10) |
| File sync of `~/.claude` and the repo (Syncthing, Mutagen) | no | races two writers | carries build output, CRLF noise | yes | **No**: breaks D1 and the config-directory rule |
| Remote desktop / SSH + tmux into Windows | pixels / a terminal | n/a | n/a | needs a VPN or ports | **No**: no approvals on the Mac or the phone, none of Mother Claude |
| Peers over today's LAN HTTPS (+ a tunnel per machine for the internet) | yes | yes | yes (git) | only with tunnels, each public | **Fallback** (Phase 1 can start on it) |
| Overlay VPN (Tailscale, ZeroTier, WireGuard) + the LAN design | yes | yes | yes | yes, needs their app and account on every machine | **Supported**, not required (the app already listens on Tailscale) |
| **Built-in peer-to-peer QUIC** (`iroh`) | yes | yes | yes | yes, no ports, no account | **Recommended** (spike-gated) |

---

## 5. Architecture

### 5.1 Picture

```text
 MacBook (machine M)                                   Windows desktop (machine W)
 ┌──────────────────────────────────┐                  ┌──────────────────────────────────┐
 │ SPA  ── /api/machines/W/… ──┐    │                  │                                  │
 │                             ▼    │   QUIC (iroh)    │                                  │
 │ axum ── machines::federation ════╪══════════════════╪══► axum (Ingress::Peer,          │
 │   │     machines::moves          │  E2E encrypted,  │      Principal::Peer{M})          │
 │   │     machines::copies         │  direct/relayed  │        │                         │
 │   ▼                              │                  │        ▼                         │
 │ claude/ (local sessions, owned)  │                  │ claude/ (W's sessions, owned)    │
 │ <app data>/machines/copies/W/…   │                  │ <app data>/machines/copies/M/…   │
 └──────────────────────────────────┘                  └──────────────────────────────────┘
          ▲  phone paired with M sees M's and W's sessions (M proxies to W)
```

New Rust module `src-tauri/src/machines/`:

| File | Owns |
|---|---|
| `identity.rs` | the machine id (reused) and the machine key (Ed25519, in the Keychain / Credential Manager via `claude/secrets.rs`) |
| `store.rs` | `machines.json` (0600): peers, grants, addresses seen, roots |
| `pairing.rs` | pairing codes, the short authentication string, introductions |
| `transport.rs` | the QUIC endpoint; accept loop serving the axum router per stream; outgoing connections with backoff |
| `protocol.rs` | the handshake (versions, capabilities) |
| `federation.rs` | subscriptions to each peer's stream; the merged view; the action proxy |
| `lease.rs` | each session's home and epoch, replicated between peers |
| `moves.rs` | the move transaction (both sides) |
| `code.rs` | git snapshots, bundles, applying them |
| `copies.rs` | keeping copies current after each turn |
| `work.rs` (Phase 4) | `machine_run`, `machine_task` |

### 5.2 Identity

- **Machine id**: the existing per-install id (stable, Migration-Assistant
  aware). Shown as the machine's name (`marcus-mbp (macOS)`, renameable).
- **Machine key**: an Ed25519 key pair made at first start, private half in
  the OS secret store (`dev.motherclaude.desktop`, account `machine-key`),
  never in a file, log, frame or backup. Its public half *is* the address on
  the peer transport (iroh's `NodeId`).
- A machine restored from another's backup keeps its own id and key (the
  existing machine-id rule).

### 5.3 Transport (D7)

**Recommended: `iroh`** (Rust; MIT/Apache-2.0; QUIC via `quinn`):

- connections are addressed by the peer's public key; TLS 1.3 with raw
  public keys authenticates both ends — a peer's key pinned at pairing *is*
  its identity, no certificates to issue or trust;
- **on the LAN** it finds peers by mDNS and connects directly;
- **across networks** it hole-punches; when that fails it relays through a
  relay server (n0 runs public ones; `iroh-relay` can be self-hosted), which
  only ever sees ciphertext;
- no inbound port, no tunnel, no third-party account.

**The API rides on it unchanged.** Each QUIC bidirectional stream carries one
HTTP/1.1 connection: the accept loop joins the stream's halves into one
`AsyncRead + AsyncWrite` and hands it to
`hyper::server::conn::http1::Builder::serve_connection(..).with_upgrades()`
with the existing axum router (WebSocket upgrades included). The context
middleware gets `Ingress::Peer`, and the principal comes from the
authenticated key, not from a token. On the client side, `reqwest` and
`tokio-tungstenite` run over a connector that opens a stream on the peer
connection. So every route, guard and test the phone path has applies to
peers too.

**Relays (D10)** — a setting chosen in the setup and meant to be the same on
every machine (two machines on different relay sets may never meet): changed
on one machine, it is offered to the others, and each applies it when its
desktop accepts (until then the setup lists the mismatch as a blocker):

| Choice | What it is | When |
|---|---|---|
| **Public relays** (default) | n0's relay servers, the library's own default; n0's DNS finds a peer's current relay from its id | works at once, anywhere |
| **Your own relay** | the URL of an `iroh-relay` you run (on the NAS, a small VPS); optionally its pinned certificate. A peer is dialled as *its id + this relay*, so no n0 service is asked anything | you want no third party in the path, or a relay near you |
| **No relay** | direct connections only: the LAN, or a machine with a public address or a mapped router port (iroh coordinates hole punching through a relay, so without one machines on two networks rarely meet) | machines that only ever meet on one network |

Every machine also keeps its peers' last direct addresses and tries them
first: mDNS is filtered on some networks (guest Wi-Fi, some routers).

Measured in spike S1 (both ends on one Mac): public relays ~2–3 MiB/s with a
~78 ms round trip from here — fine for driving sessions, slow for a large
first move; a direct path or your own relay is what makes moves fast.

Across two separate networks (spike S1: a phone on mobile data and the home
Wi-Fi) hole punching failed — the carrier's NAT — and every packet went
through the relay: ~110 ms a request, fine for driving sessions, but 1.1
MiB/s, and a 1 s round trip while a transfer filled the path. So **bulk
streams (moves, copies) run at a lower QUIC stream priority than the API
and the dashboard's frames** (*built 2026-11-13*: `transport::BULK_PRIORITY`,
-1 against every stream's 0, set by the side that sends the bytes — a
request whose body is `application/octet-stream`, every chunk of a move,
copy, sync step, run, task or role going up, and a served answer of that
type, an artifact, a result's file or `/peer/bulk` coming down; QUIC sends
a connection's higher-priority buffered data first), a move says how long it will take before it
starts, and a large first move across networks suggests your own relay or
your remote (D10). After a network change the endpoint needs time to find
a relay again (15 s in the spike): Machines health says "reconnecting", not
"offline".

A relay only ever carries ciphertext between machine keys; it learns which
machines talk, when, and from which addresses. The setup's readiness test
(§5.14) checks that every machine reaches the chosen relay, and shows for each
pair of machines whether they talk directly or through the relay. The Machines
page shows which path each connection uses right now.

**Fallback (and Phase 1-lite if the spike fails):** peers over the existing
LAN HTTPS listener, each pinning the other's certificate fingerprint at
pairing, with a peer token like a device token; across networks through
Tailscale addresses (already listened on) or the tunnel port. Same API, same
grants; only `transport.rs` differs.

**Spike S1 must confirm**: macOS and Windows builds; the key in the
Keychain / Credential Manager; LAN discovery with the macOS local-network
permission and the Windows firewall prompt; a relayed connection between two
networks (a phone hotspot); WebSocket over a stream; throughput (target:
≥ 50 MB/s on the LAN); binary size; the crate's version and stability at the
time. *Done* (iroh 1.4.1, stable; the API rides on it unchanged; at most
+13 MB of binary; between the Mac and Windows, direct by key alone and **as
fast as plain TCP** on the same Wi-Fi — 26–30 MiB/s there, so the target is
the link's own speed, not a fixed figure): `spikes/machines/RESULTS.md`.

### 5.4 Pairing

1. On the Mac, **Machines → Pair a machine**: a QR code and an 8-character
   code, valid once for 5 minutes. The QR carries the Mac's key and a
   one-time secret; the code is resolved on the LAN by mDNS (and via the
   relay's discovery across networks).
2. On Windows, **Machines → Join**: scan or type. Windows dials the Mac *by
   key* — so it cannot be talked to by anyone else — and proves the one-time
   secret.
3. **Both screens show the same six digits** (a short authentication string
   computed from both keys and the secret); you confirm on both. Across the
   internet, the Mac also asks you to type the digits Windows shows, as phone
   pairing does today.
4. Each side stores the other in `machines.json`: id, name, OS, key, grants
   (defaults below), paired-at. Nothing else is exchanged.

**Default grants for a newly paired machine** (both are yours):

| Grant | Lets the peer | LAN default | Internet default |
|---|---|---|---|
| `view` | list sessions, read transcripts, see pending items | on | on |
| `control` | send messages, answer ordinary approvals, interrupt, stop, set model/effort/mode (not bypass) | on | on |
| `move` | move sessions to and from this machine, keep copies here | on | on |
| `run` (Phase 4) | run commands and tasks here | off | off |
| `dangerous` | bypass permissions, dangerous approvals, raw keys, carrying a folder's trust and ignored files in a move (no delete route is open to a peer) | off | off |

Changing a grant, revoking, renaming: desktop only, audited. **Revoke**
closes the connections at once and forgets the key; *Revoke everywhere* asks
every online peer to do the same.

### 5.5 Authorization and audit (D6)

- `Principal::Peer { machine }`, built once per request by the context
  middleware from the transport's authenticated key (§5.3).
- New guard extractors in `server::auth`: `PeerView`, `PeerControl`,
  `PeerMove`, `PeerRun`; `ctx.may_dangerous()` is true for a peer only with
  its `dangerous` grant for the ingress (LAN or internet, decided by the
  transport: a direct LAN path vs a relayed or public one).
- **Acting person**: a proxied request carries `X-MC-Acting` (`desktop`, or
  `device:<id>:<name>`). The target trusts its paired peer to say who acts,
  audits both names, and allows a dangerous action only if the peer has the
  dangerous grant **and** the source said its own principal had the dangerous
  right there — a phone without the grant on the Mac cannot reach for it
  through the Mac.
- Never for peers: admin routes (pairing, remote access, plugin install,
  accounts' secrets), the local-script and MCP tokens' routes, `/hooks/*`,
  `/ws/browser`.
- Every peer action is audited on the machine that acts (`machine.action`:
  peer, acting person, route, session) and every dangerous one raises a
  desktop notice there, as devices do today.

### 5.6 Seeing and driving other machines (federation)

- `machines::federation` keeps one connection per online peer and subscribes
  to its `/ws` (session summaries and diffs, pending items; a session's
  transcript stream only while a page shows it).
- What it receives is republished on the local bus with `machine: <id>`;
  local frames get `machine: <self>`. The SPA groups sessions by machine (a
  filter and a badge), keys them by `(machine, session)`.
- Actions: the SPA calls `/api/machines/<machine>/<the same path as locally>`
  — one proxy route forwards an allow-list of session routes (message,
  interrupt, stop, approvals, model / effort / mode, transcript pages,
  terminal view, new session) over the peer link, adding `X-MC-Acting`.
- **New session on another machine**: *New session on …* takes the folder
  mapped from the one you are in (folder pairs), or asks for a folder there
  (the peer lists its projects and trusted folders).
- Pending approvals from peers appear in the approvals list with their
  machine; answering forwards; the first answer wins, as today.
- A peer that goes offline greys out its sessions ("DESKTOP-PC is offline
  since 14:32") instead of removing them.

### 5.7 The lease

```json
{ "session": "0f1e…", "home": "9a3c…", "epoch": 7, "since": 1794230000000 }
```

- Kept per session in `<app data>/machines/leases.json` (0600), replicated to
  peers on every change; the higher epoch wins, ties broken by machine id.
- A session with no lease is home where its files are (today's behaviour).
- Mother Claude refuses to start or continue a session leased to another
  machine: *Lives on DESKTOP-PC — Open there · Bring back here · Take over*.
- **Take over** (the home is offline): epoch + 1, with a warning; when the old
  home returns and had continued it meanwhile, its turns become a conflict
  copy (the pull's existing rule).
- Foreign sessions (VS Code, a terminal) cannot be stopped by Mother Claude:
  moving one means *Continue as a copy on …* (a fork with a new id — the
  existing *Continue on* semantics), unless you end it first.

### 5.8 Moving a session (the transaction)

```text
source S                                              target T
──────────────────────────────────────────────────────────────────────────
preflight  ── ask T: account? folder? git state? ──►  answers (nothing written)
           ◄── ok / problems to show ──────────────
freeze     wait for the turn to end (or Interrupt), close to new messages,
           stop and reap the process (claude/switch.rs)
package    session archive (backup format) + code snapshot + manifest
send       ── bulk stream (resumable, hashed) ──────►  receive into a temp dir
                                                       verify every hash
                                                       apply code (§5.10)
                                                       place the session (pull code:
                                                         folder pair, undo, conflict rule)
           ◄── landed (manifest hash) ───────────────
commit     lease: home = T, epoch + 1  ── replicate ─►  lease accepted
                                                       resume the owned session:
                                                         claude --resume <id> in the
                                                         mapped folder, same account,
                                                         model, effort, mode;
                                                         the move note leads the
                                                         next message (§3.5)
```

- **Preflight** (nothing written anywhere): T has an account of the same name
  and it is signed in; the folder maps (a pair, or *Choose its folder on …*,
  asked on S with T's candidates); T trusts that folder (or carry the trust,
  with the dangerous right, as the pull does); T's Claude Code is not older
  than S's (warn); the repo exists there with the same origin, T's checkout is
  clean or a worktree is chosen (§5.10); disk space; T is not running that
  session.
- **Before "landed"**, any failure unfreezes S: the session resumes where it
  was; T deletes its temp dir. **After "landed" but before the lease**, T keeps
  what it received as a copy (not running); a retry finds it there (the
  manifest hash makes the step idempotent).
- **Bringing it back** is a move T → S. S's old copy is a prefix of T's
  transcript, so the pull's rule replaces it (fast-forward); the code applies
  on top of S's checkout (its base is S's last snapshot or a descendant).
- The source keeps its copy (never deleted); the session page on S shows
  *Lives on DESKTOP-PC* and drives it there through the proxy (§5.6).
- `Continue on DESKTOP-PC` is one button; the dialog shows what will move
  (conversation size, code: branch, N changed files, untracked ones, ignored
  files listed in the project's opt-in), the folder there, the recipe to run,
  and *Keep driving it from here*.

### 5.9 Keeping copies current

For a session marked **Keep a copy on DESKTOP-PC** (per session, or per
project):

- after each turn ends (the session goes `idle` or `background`, debounced
  5 s), S sends T what grew since T's last acknowledgement: the transcript's
  new bytes with a hash of the prefix T already holds, new or grown subagent
  and workflow files, new file-history snapshots, the todos and session env.
  A mid-turn state is never copied (the backup's rule).
- T appends to its copy in `<app data>/machines/copies/<S>/<account>/<session>/`
  only when the prefix hash matches what it holds (else it asks for the whole
  file); 0700/0600; a size cap per peer, oldest copy evicted first.
- A move then sends only what the copy lacks (usually nothing): instant.
- **When S is asleep**, T offers *Continue here from the copy of 14:32*: a
  take-over (§5.7) from the copy, with the conflict rule if S continued
  meanwhile.
- The code is not part of a copy — snapshots are made at the move (a copy of
  the repo state after every turn would be heavy and rarely used). Spike S2
  measures whether a snapshot per turn is cheap enough to offer.

### 5.10 Code (D4, D8)

**Snapshot on the source** (git plumbing; the real index, branch and working
tree are untouched):

```text
base      = HEAD (or none, an unborn branch)
branch    = the current branch (or detached)
copies    = two copies of .git/index, each keeping the original's mtime   (F1)
idx_tree  = GIT_INDEX_FILE=copy1 git write-tree                # what is staged
wt_tree   = GIT_INDEX_FILE=copy2 git add -A && git write-tree  # working tree, untracked
                                                               # included, ignored not
check     = paths `git status` reports changed == paths differing between
            the trees; on a mismatch re-hash every file and check again  (F1)
index     = git commit-tree idx_tree -p base       # the staged tree travels too (F2)
snap      = git commit-tree wt_tree -p base [-p earlier snapshots of this
            session that T has]                    # small incremental bundles (F6)
keep      refs/mother-claude/moves/<session>/<epoch>/{snap,index}  (so gc keeps them)
always    -c core.hooksPath=<a fresh, private, empty dir per operation>,
          GIT_OPTIONAL_LOCKS=0, never the real index, commits never signed
          (F3, F4, F9); on Windows -c core.longpaths=true, and a "Filename
          too long" warning fails the snapshot                         (F8)
```

Spike S2 (Mac half, 132 of 132 runs as designed) found F1–F7, folded in
here; details in `spikes/machines/code/RESULTS-S2.md`. F1 matters most: a
plain copy of the index, dated now, made git trust stale stat data and
**silently miss an edit** in 6 of 60 snapshots. The Windows half added two: **F8**, Git for Windows without
`core.longpaths` warns, exits 0 and leaves a long path out of both `add -A`
and `status` (so F1's check agrees), and **F9**, Git LFS writes its hooks into
the overriding hooks folder, which a shared folder would then run.

The Rust port (`machines/code/`, 2026-11-11) added a tenth: **F10**,
`read-tree -m -u` silently overwrites a file the *target* ignores when the
snapshot tracks or carries one at that path. Landing checks for such paths
first and stops before anything changes (*would overwrite*, offering a
worktree); a step that fails half-way puts the checkout back. Undo refuses
when the landing changed since the apply (it would wipe that work), and
every id, ref, name and path in a move's record is checked before git or
the file system sees it, as it comes from the other machine. When the two
machines ignore different files, the proof (`verify`) reports a file only
the target ignores as a **limit** (`ignored-there`, since 2026-11-13), not
a failure: an untracked file the snapshot carried, gone from the target's
`git add -A` tree, on disk byte for byte as the source had it, and named
by the target's `git check-ignore` (its `info/exclude` or
`core.excludesFile`; the `.gitignore` files are the snapshot's). One that
came out different or went missing still fails.

**Transfer**, either way (the project setting, P2P by default):

- **Machine to machine** (default): T says which commits it has for that
  branch (preflight); S runs `git bundle create - <refs> --not <what T has>`
  and streams it; T `git fetch`es from the bundle into
  `refs/mother-claude/moves/…`. No remote needed, nothing pushed anywhere.
  Fetches never follow submodules (`--no-recurse-submodules --no-tags
  --no-write-fetch-head --no-auto-maintenance`): a submodule commit missing
  on its remote otherwise fails the whole fetch (F5); submodule commits are
  checked separately.
- **Through your remote** (GitHub or any remote): S pushes the snapshot and
  its staged tree to hidden refs (`refs/mother-claude/<machine>/<session>/{snap,index}`,
  `--no-verify --recurse-submodules=no`) — and the branch, if
  you ticked *Also push the branch*; T fetches. Useful when the machines are
  rarely online together, or a repo is too large to send whole the first
  time. Both machines need non-interactive access to the remote (the setup
  checks it, §5.14). After the move the hidden ref is deleted from the
  remote (kept locally for undo); *Keep hidden refs on the remote* is off by
  default.

**Where each choice is made (D10)**: the setup picks the default for all
machines; a project may override it (*Project settings → Machines*); the move
dialog shows the choice in force and lets you change it for that move. The
same holds for where the code lands, below. A move falls back on its own only
from *machine to machine* to *through your remote* when the bundle would be
larger than 1 GB and the remote is reachable — and says so. *Built
2026-11-13*: the bundle is made, and when it is over the limit
(`moves::BUNDLE_LIMIT`, injected in tests) and the remote answers without a
prompt here and there (the setup's check here; the target's preflight asked
again with `mode: remote`), the snapshot is made again through the remote,
the bundle's refs dropped (`code::drop_snapshot`), and the record says so
(warning `code_through_remote`); otherwise the bundle goes as it is.

**Applying on the target** (default: the checkout when it is clean; or always
a worktree), one of:

- **The checkout** (default when T's working tree is clean): the branch is
  checked out at `base` (fast-forwarding it when it is behind; **stop** when it
  has commits `base` does not — offer a worktree, or *reset to the Mac's*,
  keeping T's tip as `refs/mother-claude/undo/<time>`; *built 2026-11-13*:
  the blocker `branch_diverged` offers `worktree-or-reset` when T lists the
  `replaceBranch` capability, `replaceBranch: true` on the move turns it
  into a warning, the move's code carries the tip the preflight saw
  (`CodeSpec::replaces`) and T lands with `Reset::IfTip` — a tip that moved
  meanwhile stops the landing — keeping it under the landing's
  `refs/mother-claude/undo/<time>/branch`, which *Undo move* puts back),
  then
  `git read-tree -m -u <T's HEAD> base`, `git read-tree -m -u base snap`
  (working tree = the snapshot), `git read-tree idx_tree` (index = what was
  staged) and `git update-index --refresh`, so staged, unstaged and untracked
  files are exactly as they were on S. *Clean* means `git status --porcelain
  -uall` is empty (untracked files count) and no merge, rebase, cherry-pick,
  revert or bisect is in progress; a branch checked out in another worktree
  stops too. Plumbing alone does **not** keep hooks from running
  (`post-index-change`, `reference-transaction`, `pre-push` all fire): every
  git call disables them (F3).
- **Coming back** (*Bring back*): the source checkout still holds exactly
  what it sent, so it would never look clean. A checkout whose HEAD is a
  snapshot's base, index its staged tree and working tree its tree — for a
  snapshot of *this* session — is **parked** and counts as clean for that
  session (F7); any edit made meanwhile makes it an ordinary dirty checkout.
  A session that lived in a worktree on T comes home to its home branch,
  which the move records.
- **A worktree** (chosen as the default, chosen for this move, or offered
  when T's checkout has changes of its own): `git worktree add` at the
  snapshot in `<app data>/machines/worktrees/<session>` (outside the
  project), and the session continues in that folder (mapped back to S's
  project on the way home). The checkout is never touched in this mode, and
  nothing is ever merged into it on its own. When the session leaves T or
  ends, *Remove the worktree* is offered: its work went with the session's
  snapshot, and stays under `refs/mother-claude/moves/…` for 7 days.
  *Built 2026-11-13*: `POST /api/moves/{id}/remove-worktree` (on either
  computer: the source's record asks T, `POST
  /peer/moves/{id}/remove-worktree`), the record's view carrying
  `worktree: {path, removable, removedAt}` (removable once the lease left T,
  or T's sessions say it ended); T refuses while the session runs there
  (`worktree_in_use`) and while the worktree is not parked at one of the
  session's snapshots (`worktree_unsaved`, `code::remove_move_worktree`),
  removes it with its `mother-claude/<session>` branch, keeps the snapshot
  refs, and *Undo move* is no longer offered; the move card offers it with
  a second tap.

**Undo**: T's previous branch tip and a stash of anything the apply touched
are kept under `refs/mother-claude/undo/<time>` for 7 days; *Undo move* puts
them back.

**Warnings in preflight**: the project uses Git LFS (objects come from the
remote, not the bundle); submodules (their commits must exist on T; changes
inside one never travel); paths differing only in case, and a case-only
rename made on disk (invisible to git on a case-insensitive disk); names
Windows cannot hold (`aux.txt`, a trailing dot, `?`); Windows path length;
unstaged mode changes and new executables (they lose +x on Windows: stage
them); untracked symlinks (plain files on Windows without Developer Mode);
line-ending settings that would show the snapshot as changed (S2: every
`autocrlf`/`eol` combination was tree-exact, only raw line endings differ).

**Recipe** (optional, per project): commands to run after a move, per OS —
kept in Mother Claude's data dir, editable on the project, or read from a
committed `.mother-claude/recipe.json` if the repo has one:

```json
{ "afterMove": { "windows": ["npm ci"], "macos": ["npm ci"] },
  "carryIgnored": [".env.local"] }
```

Run only when you tick it in the move dialog; shown as a background task of
the session.

### 5.11 Working on another machine without moving (Phase 4)

**MCP tools** (registered in `mcp::registry()`, reaching peers through
the source machine):

| Tool | Does | Returns |
|---|---|---|
| `machine_list` | the machines this session may use: name, OS, online, versions | a list |
| `machine_run` | runs a command on a machine, in a fresh worktree at the session's code snapshot, mapped folder | exit code, output tail, artifacts listed (by path, fetched on request) |
| `machine_task` | starts an **owned** session there with a task, in its own worktree at the snapshot | when done: its summary, and its changes as a branch `mother-claude/<machine>/<task>` here, to review |

- Every call is a **pending approval on the source** (it runs code on another
  machine), unless a standing grant names the project, the machine and the
  command prefix (`npm test`, `cargo build`, `xcodebuild …`). The target also
  needs the `run` grant for the source.
- Worktrees live in the target's data dir, outside the project; removed when
  the task ends (kept on failure, for a look, for 1 day).
- **Teams**: a template role may carry `machine` (`"DESKTOP-PC"` or
  `"os:windows"`); the launcher starts that role on the peer (as an owned
  session there, through `capacity::admit` *there*), the board stays with the
  lead, and the role's `task_*` tools reach the lead's board through the peer
  link. A multi-platform app gets a lead on the Mac, an `ios` role on the Mac,
  a `windows` role on the desktop.

### 5.12 More than two machines

- Trust is **pairwise** and explicit. **Introduce**: pairing C with A, while A
  is paired with B, offers *Also pair C with B?* — B and C each confirm the
  six digits; nothing is trusted transitively.
- Folder pairs become **roots**: a named folder with one path per machine.
  A pair is a root with two entries, so existing pairs carry over as they are:

  ```json
  { "root": "projects",
    "paths": { "9a3c…": "/Users/marcus/development/projects",
               "5e71…": "D:\\development\\projects",
               "c08d…": "/home/marcus/projects" } }
  ```

  (Keyed by machine id: the 32 hex characters backups already use,
  `identity.rs`.)

  Replicated between peers (last writer wins per entry, removals stick — the
  pairs' existing rule) and still stored in the backup sets.
- Leases, copies and the proxy work per pair of machines; nothing assumes two.

### 5.13 A machine without anyone at it (later)

A build box (a Windows PC in a cupboard) needs Mother Claude running without
anyone logged in to the UI: tray-only start at login first; a headless
binary from the same crate (`mother_claude_lib` without the webview) or a
Windows service after that. Not in the first release.

### 5.14 Setting it up on every machine (D11)

Machines is off until you turn it on: **Machines → Set up**, or the first
*Continue on …* / *Keep a copy on …* / *New session on …* you press, opens the
setup. It runs on each machine — only a machine can check its own OS — and
once two machines are paired, each one shows **every** machine's checklist, so
the setup is done in one place and says where each step must happen.

**The steps**

1. **This machine.** Its checks (table below), each with *Fix* (it does it,
   or opens the exact settings page) and *Check again*. Blockers first.
2. **Pair the others.** For each other machine: install or update Mother
   Claude there (the setup shows the download link and the version needed),
   *Machines → Join* there, scan or type, confirm the six digits on both. The
   confirmation screen on each machine also shows **what that machine lets the
   other do** (the grants, defaults pre-ticked), so one confirmation per
   machine settles them. A third machine is offered the existing ones
   (*introductions*, §5.12).
3. **Every machine's checks.** The same table, run on each paired machine
   and shown side by side. A fix that needs a person at that machine (a
   firewall prompt, a macOS permission, an administrator password) reads
   *On DESKTOP-PC:* and appears there too, as a notice and on its Machines
   page; *Ask DESKTOP-PC to fix* raises it. Nothing on another machine changes
   without being confirmed on that machine.
4. **What each machine may do on the others.** A matrix (who acts × on
   whom), from step 2, editable: `view`, `control`, `move` on by default on the
   LAN and over the internet; `run` off until Phase 4 is used; `dangerous`
   off. A change is made on the machine that gives the grant (desktop only),
   and confirmed there.
5. **Accounts and folders.**
   - Accounts are matched by name on every machine, with each one's sign-in
     state per machine: *Add account there*, *Sign in…* (runs the existing
     sign-in on that machine, its browser).
   - Roots are proposed by matching the git repos found under likely folders
     on each machine by their `origin` URL
     (`~/development/projects/app` ↔ `D:\development\projects\app` proposes
     `~/development/projects` ↔ `D:\development\projects`): confirm, edit, or
     add one. Existing folder pairs are kept as roots.
   - Repos present on one machine only are listed: *Clone it on DESKTOP-PC*
     (an explicit act: `git clone <origin>` into the mapped folder there).
6. **Options (D10).** Relay (public — default; your own, with its URL and a
   reachability test; none). How code travels (machine to machine —
   default; through your remote). Where it lands (the checkout when clean —
   default; always a worktree). Space for copies (default 5 GB per machine).
   *Keep this computer awake while another machine uses it* (default on).
7. **Readiness test**, for every pair of machines, both ways (§ below). When
   no blocker is left, Machines turns on; warnings stay listed.

**What is checked**

B = blocks the feature (or the part named); W = works, but worse — said why.

| Area | Check | How it is detected | | Fix |
|---|---|---|---|---|
| Mother Claude | every machine speaks the same Machines protocol | `hello` versions (§8) | B | *Update* on that machine (Software updates) |
| | starts at login, so a restarted machine is reachable | the autostart entry | W | *Turn on* |
| | the computer stays awake while another machine uses it | the setting; sleep timers (macOS `pmset -g`, Windows `powercfg /query`) | W | *Keep awake while used* (a power assertion held by the app — `IOPMAssertionCreateWithName` / `SetThreadExecutionState` — no system setting changed); *Open energy settings* |
| Network (macOS) | Local Network access for Mother Claude (the app's `Info.plist` gains `NSLocalNetworkUsageDescription`, so the prompt says why) | an mDNS query and a UDP send to the LAN (denied: `No route to host`, os error 65; there is no API to read the setting, so it is tried — from inside the app, which macOS holds responsible for everything it launches; while macOS's own prompt is open the check says so). Spike S5: today's app, without the key, is refused without a prompt | B† | *Open Local Network settings* (`x-apple.systempreferences:…Privacy_LocalNetwork`), switch Mother Claude on |
| | the application firewall lets Mother Claude in | `socketfilterfw --getglobalstate`, `--getblockall`, `--getappblocked <app>` | B† | *Open Firewall settings*, allow Mother Claude |
| Network (Windows) | the network is **Private** (Public blocks incoming connections) | `Get-NetConnectionProfile` | B† | *Open network settings* (`ms-settings:network-status`), set Private — or allow on Public, explained |
| | a Defender Firewall rule lets Mother Claude in for the current profile, and no block rule of its own overrides it (Windows creates block rules when its first *Allow access?* prompt is cancelled) | `Get-NetFirewallApplicationFilter -Program <exe>` and its rules | B† | *Allow through the firewall*: one script run as administrator after the Windows UAC prompt — `New-NetFirewallRule -Program <exe> -Direction Inbound -Action Allow -Profile Private -Protocol UDP` (TCP too if the fallback transport is used), and Mother Claude's own block rules removed |
| Network (both) | the chosen relay answers | connecting to it (the endpoint's home relay) | B across networks, W on one LAN | check the URL, a proxy, or pick the public relays |
| | every machine uses the same relay setting | compare via `hello` | B | *Use this setting everywhere* |
| | UDP gets out (direct connections across networks) | the readiness test's path | W ("relayed only: slower") | — |
| Files (macOS) | Mother Claude may read and write each root (Documents, Desktop, iCloud Drive and external volumes are protected) | listing the root (macOS asks then) | B | the system prompt; else *Open Files and Folders* / *Full Disk Access* (the existing invoke) |
| Files (Windows) | Controlled Folder Access does not block Mother Claude in a root | `Get-MpPreference` (`EnableControlledFolderAccess`, protected folders) | B when it would | *Open Ransomware protection*, *Allow an app* |
| | long paths | `LongPathsEnabled` (registry), git `core.longpaths` | W | *Turn on* (registry: as administrator; git: for this user) |
| Files (both) | every root exists on every machine | the roots | B | *Choose its folder* |
| | each project is on every machine you will move it to | repos by `origin` | W (that project only) | *Clone it there* |
| | free space for copies | the data dir's volume | W | lower the cap |
| Git | git is installed, 2.30 or newer | `git --version` | B | install link (Xcode command line tools; Git for Windows) |
| | *through your remote* only: the remote answers without a prompt | `GIT_TERMINAL_PROMPT=0 git ls-remote origin`, one repo per root (each repo again before its first move) | B for that mode | *Sign in to GitHub* (`gh auth login`, Git Credential Manager) |
| | Git LFS where a project uses it | `git lfs version`; `filter=lfs` in `.gitattributes` | B for that project | install link |
| | line endings agree | `core.autocrlf` per machine; `.gitattributes` per project | W | the `.gitattributes` lines to commit (shown; Mother Claude does not write them) |
| | symlinks on Windows | `core.symlinks`; Developer Mode | W per project | instructions |
| Claude Code | installed on every machine, not older than Mother Claude's minimum | `claude --version` | B | *Update Claude Code* (`claude update`, run when you press it) or the install link |
| | the same version everywhere | compare | W | *Update the older one* |
| | Windows: Git Bash found for its Bash tool | `CLAUDE_CODE_GIT_BASH_PATH`, the usual install paths | B | install Git for Windows, or set the path |
| | each account you will use exists on every machine **with the same name** | the accounts lists | B for that account | *Add account there* |
| | … and is signed in there | `claude auth status` (per account, on that machine) | B for that account | *Sign in…* on that machine |
| | the roots' projects are trusted there | `hasTrustDialogAccepted` | W | carried on the move, as the pull does (with the dangerous right) |
| | Claude Code tools (Mother Claude's MCP server) are on | the setting | W (B for Phase 4's tools) | *Turn on* |
| Pairing | every machine you added is paired with every other you chose | the stores | B | *Pair* (step 2) |
| | no machine's key changed since it was paired (a reinstall) | `hello` | B | *Pair again* |
| | the grants a move needs are given both ways | the matrix | B for moves that way | step 4 |

† B with the *No relay* setting. With a relay it is W: the machines still
meet, but every connection goes through the relay — slower, and only while
both have the internet — so the setup asks for the fix all the same.

**The readiness test**, for each pair of machines, both ways, with a result
per line and the time it took:

1. connect: direct or relayed, round trip;
2. move 20 MB over a bulk stream: throughput;
3. a git round trip on a scratch repo in the data dir (never a project):
   snapshot, transfer the chosen way, apply in a temp dir, compare trees;
4. Claude Code on the other machine: `--version`, and `auth status` for
   each account to be used;
5. every root resolves on the other machine; one project per root is found
   there;
6. a session-shaped transfer: a fake archive placed into a temp config dir
   with the pull's placement code (never a real one), checked, removed.

**Machines health.** The checks rerun at start, every 30 minutes, when a
network changes, and before every move (with the move's own, per project:
§5.8). A blocker that comes back — a firewall rule removed, a sign-in expired,
the network turned Public — turns Machines' badge red, says what and where,
and the move's preflight names it before anything is frozen.

**Where the checks live**: `machines/setup.rs` (the checks and fixes, one
`cfg` per OS), `GET /api/machines/setup` (this machine), `GET /peer/setup`
(another machine's, for the side-by-side view), `POST /api/machines/setup/fix`
(desktop only: the fixes this machine can do), `POST /api/machines/test` (the
readiness test). Opening a settings page goes through a Tauri `invoke`, the
one place the architecture allows it (a desktop-only OS concern); the
elevated firewall and registry fixes run only after the OS's own
administrator prompt.

### 5.15 Code sync (before v0.11.0)

*Built on the `machines` branch (2026-11-12): `machines/sync/` (the pair
and its record in `machines/sync.json`, the step, the watcher, the peer's
half, the routes `GET|PUT /api/machines/sync`, `POST …/{id}/now`, `POST
…/{id}/resolve` and `/peer/sync/{id}/*`), `code::sync` (a checkout's state,
`keep_and_put_back`), a landing parked at a given state
(`ApplyRequest::parked`) with its reset rule, the asked action of kind
`sync`, the *Code sync* card and the per-project switch, and
`tests/sync.rs`. The user guide is [MACHINES.md](MACHINES.md) "Keeping
code in sync". Choices made where this section left one: a project syncs
with one other computer at a time; a step always travels machine to
machine (a bundle), never through the remote; the first sync sends the
side with changes (or newer commits) to the side that is clean and behind,
and is a conflict when neither is; a landing records the state it made,
an exec bit or other limit of the receiving computer included, so limits
never bounce back; the side that loses a conflict may have its branch
reset over its own commits (kept under the kept ref and the undo ref);
`Sync now` on the side that did not change asks the other to send; the
files git ignores that a project carries travel with each step when the
other computer gave `dangerous`, and a change to one alone starts no step;
each side keeps the refs of its last three steps, and every 32nd step goes
unchained so old steps' commits can be collected.*

Decided on 2026-11-12: a project's code is kept the same on two computers,
both ways, **off by default** and turned on per project. Built on §5.10's
snapshot, transfer and landing; nothing new runs git differently.

- **What travels** (git-aware): the branch's commits and the staged,
  unstaged and untracked changes — what a move carries. Files git ignores
  stay put, except the ones the project lists to carry (`carryIgnored`, the
  dangerous right, as moves do).
- **Turning it on** for a project, from the desktop, pairs this folder with
  the folder a root (§5.12) maps it to there. It is on only once **both**
  computers turned it on: the first asks, the other's desktop confirms (the
  asked-actions rule). Both folders must be git repositories of the same
  project (the same `origin`, or a common commit), and each computer must
  have given the other its `move` grant (sync writes into its checkout).
  Machines off, the other computer offline or no grant: sync waits, says
  why, and never queues writes for later.
- **When**: a file watcher on the project notices a change; a few quiet
  seconds later (debounced; `.git/` changes count — a commit, a branch
  switch) the computer that changed syncs. A check every minute is the
  safety net. *Sync now* runs it at once.
- **One step**: both computers keep the **last synced state** (HEAD, branch,
  staged tree, working tree). The changed side asks the other for its state:
  unchanged → it sends a snapshot (a bundle without what the other has,
  chained on the last one) and the other lands it in its checkout, which
  counts as parked for this sync (F7), so it is clean by definition; both
  record the new state. The other side's checkout changing in between makes
  the landing stop (it is no longer parked) — that is a conflict, never an
  overwrite.
- **Both changed** (a conflict): sync **pauses** for that project, on both
  computers, and the dashboard shows both sides: *Keep the Mac's* / *Keep
  DESKTOP-PC's*. Nothing is merged on its own. The losing side's changes
  are first kept as commits under `refs/mother-claude/sync/<id>/kept-<time>`
  (nothing lost), its checkout is put back to the last synced state, and
  the winner's lands as an ordinary step. Identical changes on both sides
  are no conflict.
- **Never alongside a move** of a session working in that project: sync
  pauses while it runs.
- Every step is audited on both computers (no file contents); a step that
  fails leaves both checkouts as they were (the landing's own undo).

---

## 6. Data

| File (app data dir, 0600) | Holds |
|---|---|
| `machines.json` | `{ self: {id, name}, enabled, peers: [{id, name, os, key, grants: {view, control, move, run, dangerous: {lan, internet}}, pairedAt, lastSeen, versions}], roots: [...], settings: {relay: {mode: "public"\|"custom"\|"none", url, certSha256}, code: {transfer: "direct"\|"remote", apply: "checkout"\|"worktree", keepRemoteRefs: false}, copiesCapBytes, keepAwake}, projects: {"<root-relative path>": {code: {…}, recipe}} }` — `settings` replicated to peers |
| `machines/setup.json` | the last result of each check on this machine, and of the readiness test per peer (for *Machines health*) |
| `machines/leases.json` | `{ "<session>": {home, epoch, since} }` |
| `machines/copies/<machine>/<account>/<session>/…` | copies, laid out like a config directory |
| `machines/copies/index.json` | per copy: its source, account, folder there, sizes, its transcript's acknowledged mark (size and prefix check), the last turn's time; each file's state in `<copy>/.mc/files.json` |
| `machines/copies.json` | this computer's side: each session's own choice of computers, and how each copy elsewhere stands |
| `machines/moves/<id>.json` | a move's manifest and state, while it runs (§5.8) |
| `machines/sync.json` | the projects kept in sync (§5.15): per pair its id, the other computer, both folders, each side's consent, the last synced state `{head, branch, idxTree, wtTree, snapshot, step}`, its status, a conflict's two sides and the kept refs; `machines/sync/<id>/` holds a step's files while it travels |
| `machines/worktrees/…` | worktrees for moves into a busy checkout, and for machine tasks |
| Keychain / Credential Manager | `machine-key` (the private key) |

**Move manifest**:

```json
{ "id": "mv_…", "session": "0f1e…", "account": "Work", "from": "9a3c…", "to": "5e71…",
  "epoch": 8, "sourceCwd": "/Users/marcus/dev/app", "targetCwd": "D:\\dev\\app",
  "archive": {"sha256": "…", "size": 18342211},
  "transcript": {"sha256": "…", "size": 17110034},
  "code": {"mode": "bundle", "branch": "feat/windows-tray", "base": "a1b2…", "snapshot": "c3d4…",
           "indexTree": "e5f6…", "apply": "checkout"},
  "settings": {"model": "…", "effort": "high", "permissionMode": "default"},
  "recipe": ["npm ci"] }
```

---

## 7. HTTP API

Local (desktop, or a device where noted):

| Route | Guard | Does |
|---|---|---|
| `GET /api/machines` | Authed | this machine and its peers: online, versions, grants |
| `GET /api/machines/sessions` | Authed | the polling fallback's view of the other machines' sessions (the federation's cache; ETag), empty while Machines is off |
| `POST /api/machines/pair` · `/join` · `/confirm` | DesktopOnly | pairing (§5.4) |
| `PATCH /api/machines/:id` · `DELETE …` | DesktopOnly | rename, grants, revoke |
| `ANY /api/machines/:id/proxy/<allowed path>` | Authed (+ the route's own guard, checked locally first) | forwards to the peer (§5.6) |
| `POST /api/sessions/:id/move` | Authed; DangerousOk when it carries ignored files or trust | starts a move: `{to, code: "bundle"\|"remote"\|"none", apply: "checkout"\|"worktree", recipe, trust, carryIgnored, keepDriving, takeOver, from}` — `takeOver` when the session's machine is offline, `from` a copy's time; `409 machines_off` while Machines is off here |
| `POST /api/sessions/:id/move/preflight` | Authed | the move sheet's checks, the target's `/peer/moves/preflight` asked through the source; nothing written |
| `GET /api/moves/:id` · `POST …/cancel` · `POST …/undo` | Authed | follow, cancel (before landed), undo |
| `GET /api/sessions/:id/copies` · `PUT …` | Authed | which machines keep a copy (`{machines: [id…]}`, `null`: as its project says) and how each copy stands |
| `GET /api/machines/copies` · `DELETE /api/machines/copies/:id` | ManageData | the copies held here and kept elsewhere; `held.<machine>.<session>` removes one held here, `kept.<machine>.<session>` stops keeping one there |
| `GET /api/roots` · `PUT …` | ManageData | roots (§5.12): `{machine, machines: [{id, name, os, here, paired}], roots: [{id, name, paths: {"<machine id>": "<path>"}, updatedAt, by, pair?}], max}`; `PUT {upsert: [{id?, name, paths}], remove: [id]}` — all or nothing, a root without `id` is new, one with an `id` gets that name and that whole `paths` map; answers the same view |
| `GET /api/machines/setup` | Authed | this machine's checks, and every peer's (fetched through `/peer/setup`) |
| `POST /api/machines/setup/fix` | DesktopOnly | `{check}`: runs this machine's fix (or opens its settings page through the desktop) |
| `POST /api/machines/setup/ask` | DesktopOnly | `{machine, check}`: asks a peer to show that fix to whoever is at it; `check` also names the asked actions — *Sign in…*, *Add the account*, *Clone it*, *Change the grants* — each confirmed on that machine |
| `GET /api/machines/nearby` | DesktopOnly | computers on this network showing a pairing code (§5.4) |
| `POST /api/machines/test` | DesktopOnly | the readiness test, for one pair or all |
| `GET /api/machines/settings` · `PUT …` | DesktopOnly | relay, code transfer, where code lands, copies, keep awake (D10); a project's overrides |
| `POST /api/machines/enable` | DesktopOnly | turns Machines on — refused while a blocker is left |

Peer-only (`Ingress::Peer`):

| Route | Guard | Does |
|---|---|---|
| `POST /peer/hello` | the transport's key | handshake: versions, capabilities, grants it holds |
| `GET /peer/setup` · `POST /peer/setup/ask` | PeerView | this machine's checks; a request to show a fix here (shown, never run, until confirmed here) |
| `POST /peer/settings` | PeerView | the replicated settings (relay, code defaults) — applied only when this machine's desktop accepts them |
| `GET /peer/roots` · `POST …` | PeerView · PeerMove | the roots with their tombstones and digest; `POST {roots}` merges the sender's (the `move` grant is the consent: roots steer where a move lands) and answers with the receiver's, which the sender takes only when it gave the receiver `move` too |
| `POST /peer/moves/preflight` · `/receive` · `/land` | PeerMove | the target's half of a move |
| `POST /peer/leases` | PeerMove | lease changes |
| `POST /peer/copies/:session/offer` · `PUT …/rounds/:round` · `POST …/rounds/:round/apply` · `GET`/`DELETE /peer/copies/:session` | PeerMove (Machines on there, but for `DELETE`) | a copy: what it holds, a round's blob in chunks, applying it, its state, dropping it |
| `POST /peer/run` · `/task` (Phase 4) | PeerRun | machine work |
| everything a device may call | PeerView / PeerControl / dangerous rules | the proxied API |

## 8. Peer protocol

- `hello` exchanges app version, a `machines` protocol version (starts at
  1) and capabilities (`moves`, `copies`, `bundle`, `run`, …). A feature is
  used only when both sides list it, so a Mac on 0.12 and a Windows on 0.11
  still see each other's sessions; the Machines page says what an older peer
  lacks and suggests updating it. It also carries the digest of the
  sender's roots (`rootsDigest`, with the `roots` capability): when the two
  differ they are exchanged (`/peer/roots`), once per change of either
  digest or of the `move` grant either way.
- Bulk transfers (archives, bundles, copies) use their own streams: chunked,
  zstd-compressed, each chunk hashed, resumable from the last acknowledged
  offset.
- Liveness: a ping every 20 s on the control stream; a peer silent for 60 s is
  offline.

## 9. UI

The screens are drafted in `spikes/machines/UX.md`; its eight decisions were
taken as recommended on 2026-11-10:

1. *Machines* is the first entry of the sidebar's *Connect* group, with a red
   dot for health.
2. The move is **Continue on another computer…** (button *Continue on
   DESKTOP-PC*, and *Bring back to this Mac*), beside *Continue on another
   account…*; never "Move to…".
3. The page is *Machines*; sentences say "computer".
4. Machine chips only on other computers' sessions; the *Machine* filter
   covers the rest.
5. Two desktops pair by the code and a *Nearby* list; the QR code is small
   and secondary.
6. *Keep driving it from here* is on by default in the move sheet.
7. A phone may start a move when paired with the session's computer or one
   that may *Move*; files outside git and a folder's trust need its dangerous
   permission; setup, pairing, grants and options stay desktop-only.
8. A target folder with changes of its own gets *Use a separate folder
   (worktree)* in the sheet — never a wait or a merge.

- **Machines setup** (a full-page wizard, §5.14): the seven steps with a
  progress rail; each check a row (state, why it matters in one line, *Fix* /
  *Open settings* / *Ask DESKTOP-PC*, *Check again*); machines side by side
  from step 3; the readiness test as a live grid (pairs × tests). It can be
  left and resumed; *Turn on Machines* stays disabled while a blocker is left,
  and says which.
- **Machines** (new sidebar page): this machine (name, id, key fingerprint,
  versions), peers (online/offline, network path: LAN direct, internet direct,
  relayed), grants, *Pair a machine*, *Join*, roots, the settings of D10,
  recent moves, and **Machines health** (green, or the blockers with their
  fixes). A red dot on the sidebar entry when a blocker comes back.
- **Sessions**: a machine filter; a chip on other computers' sessions
  (`🖥 DESKTOP-PC`), none on this computer's; offline machines greyed.
- **A session's page**: *Continue on another computer…* (the move sheet), *Keep a copy on …*,
  *Lives on DESKTOP-PC — Open there · Bring back here · Take over*; the
  composer and approvals work the same for remote sessions.
- **New session**: *on this machine / on DESKTOP-PC*.
- **Phone**: the same badges and filter; moving from a phone needs the
  `ManageData`-like rules (a device may start a move; carrying ignored files or
  trust needs its dangerous grant).
- Help topics (English and Vietnamese) and the onboarding card: "Using two
  computers".

## 10. Security review

| Threat | Mitigation |
|---|---|
| A device on the LAN impersonates a machine | Peers are addressed by pinned keys; QUIC/TLS with raw public keys; pairing confirmed by six digits on both screens. |
| A pairing code is seen or leaked | One use, 5 minutes, bound to the initiator's key; the six digits; across the internet you also type them on the existing machine. |
| One machine is compromised and drives the others | Grants per peer; `run` and `dangerous` off by default (internet separately); every action audited on the machine that acts, dangerous ones noticed there; revoke from any machine, *revoke everywhere*. |
| A stolen laptop | Revoke it on the others; its key is in its OS secret store; copies on it are only of sessions you chose to copy there. |
| The relay reads or alters traffic | End-to-end encryption between machine keys; a relay sees ciphertext and timing; self-hosting `iroh-relay` is an option. |
| A phone without the dangerous grant reaches for it through its desktop | The target allows a dangerous action only with its peer's grant **and** the acting principal's right on the source. |
| A move loses work | A transaction with undo on both sides; the source copy is never deleted; conflicts become conflict copies, never merges. |
| A move runs code from the repo (hooks, filters) | Every git call with an empty, private `core.hooksPath` and pushes with `--no-verify` (plumbing alone still runs hooks — spike S2, F3); recipes run only when ticked, as the session's background task. |
| Secrets leave a machine unintentionally | Ignored files never travel unless the project lists them and you tick them; `.credentials.json` and tokens never travel (sign-in stays per machine, as with backups). |
| A replayed move | Move ids and lease epochs; a lower epoch is refused. |
| The setup is used to weaken a machine from another one | A machine only *asks* (`/peer/setup/ask`): every fix runs on the machine it changes, after its own desktop confirms it, and an elevated one after the OS's administrator prompt as well. A fix opens a port to Mother Claude only, on the Private profile only, and the setup never turns a firewall off. |

## 11. Invariants (must stay true)

- One writer per transcript, now across machines (D1).
- Nothing writes to a config directory in the background (D3).
- Mother Claude writes a project only on a move with code or an approved task, through plumbing, with undo (D8).
- Authorize by principal; `Principal::Peer` never reaches admin routes; dangerous = peer grant ∧ acting principal's right (D6).
- All dashboard data still flows through axum, now also over the peer transport.
- Owned vs foreign: a foreign session on another machine gets exactly the control it gets locally (hooks, tmux), no more.
- Secrets (tokens, keys, `.credentials.json`) never travel.
- Machines stays off while the setup has a blocker left; nothing on a machine changes because another machine asked until that machine's desktop confirms it (D11).

---

## 12. Plan

Sizes are for one engineer, rough, before AI help; the order matters more
than the numbers.

### Phase 0 — spikes (≈ 1½ weeks)

| Spike | Question | Done when |
|---|---|---|
| S1 transport | iroh on macOS and Windows: key in the secret store, LAN discovery, relayed connection across two networks, axum + WebSocket over a stream, throughput, binary size; the three relay settings (n0's public relays, an `iroh-relay` run on the NAS, none) | a two-machine demo: the Windows dashboard's session list rendered on the Mac over iroh, LAN and hotspot, with each relay setting |
| S2 code | snapshot / bundle / apply on real repos: staged, unstaged, untracked, deleted, renamed files; `autocrlf` true vs false; LFS; submodules; a diverged branch; both transfer modes (bundle, hidden ref) and both landings (checkout, worktree) | a script that round-trips a dirty working tree Mac → Windows → Mac byte-exact, each way |
| S3 Claude Code | resuming on Windows a Mac transcript placed by the pull code; `--append-system-prompt` with `-p` stream-json and `--resume`; an older CLI resuming a newer transcript | written results (like `docs/TEAMS.md`'s spike) |
| S4 UX | pairing, setup and move dialogs, paper prototypes | agreed screens |
| S5 setup | every check of §5.14 on a fresh macOS user and a fresh Windows user, starting from the worst state: Local Network denied, the macOS firewall blocking all, a Public Windows network, a cancelled *Allow access?* prompt (its block rules), Controlled Folder Access on, no git, no Git Bash, a signed-out account. For each: how it is detected without changing anything, what a standard user can fix, what needs an administrator, which settings page opens | a table of results per OS version, and each check either reliable or marked *tried, not read* in the setup |

Exit: a decision record — iroh or the fallback transport — and the S5 table,
in this document.

**Status (2026-11-11): both halves have run**, up to what needs an
administrator or both machines at once — tools and results in
`spikes/machines/` (`RESULTS.md`; the screens in `UX.md`, agreed — §9). The
spikes changed §3.5 (the move note is a message), §5.3 (what *No relay* and
*Your own relay* need), §5.10 (nine fixes, F1–F9) and §5.14 (the app has no
Local Network access on the Mac yet). Left, at the end of `RESULTS.md`: the
two worst-state walks (`setup-walk/macos.sh`, `setup-walk/windows.ps1` —
your password, your clicks), S2's real round trip between the machines,
and a Mac session resumed on Windows.

**Decision (2026-11-11): the transport is iroh** (1.4.x), not the fallback.
It carried the app's HTTP and WebSocket stack unchanged on macOS and
Windows; the Mac found the Windows desktop by its key alone — through n0's
DNS, or by mDNS with no relay — and talked to it directly from the first
packet; its throughput equalled plain TCP on the same Wi-Fi; pairing by key
and a read-only forward of the real API worked; all three relay settings
connect; the binary grows by at most 13 MB. To keep in view: n0's public
relays are slow (1–3 MiB/s from here), macOS needs the Local Network
permission (Phase 1 adds `NSLocalNetworkUsageDescription`), and across two
networks (a phone on mobile data and the home Wi-Fi) hole punching failed,
so everything went through the relay at ~1.1 MiB/s with a 1 s round trip
while a transfer ran (§5.3: bulk transfers yield to the API).

### Phase 1 — set up and pair machines (≈ 4 weeks)

*Built on the `machines` branch (2026-11-11): `src-tauri/src/machines/`,
the Machines page and setup, `tests/machines.rs`; the user guide is
[MACHINES.md](MACHINES.md). **Exit test passed by hand (2026-11-11)** on the
Mac and the Windows desktop: from the worst case (Windows on a *Public*
network with the firewall prompt cancelled, the Mac with Local Network
denied) the setup alone got every check green; paired on the LAN through
*Nearby*, and across networks (the Mac on a phone's hotspot) by typing the
code and the digits.*

- `machines/identity.rs`, `store.rs`, `pairing.rs`, `transport.rs`, `protocol.rs`.
- `server::context`: `Ingress::Peer`, `Principal::Peer`; `server::auth` guards; the allow-list of routes a peer may reach.
- API §7 (pairing, list, grants, revoke, settings, enable); `/peer/hello`, `/peer/settings`.
- The relay setting, all three choices, offered to every machine (§5.3).
  *Your own relay's pinned certificate (`certSha256`) since 2026-11-13*
  (`machines/relay_pin.rs`): iroh 1.4 takes a TLS verifier for every
  connection that is not a peer's (`Builder::ca_tls_config` with
  `CaTlsConfig::custom_server_cert_verifier`), so the relay's host must
  present exactly the pinned certificate — a self-signed one too, its
  handshake signature still checked — and any other host keeps iroh's
  default check (Mozilla's roots); the pin is part of the offered setting
  (`same_as`), left out of the file and the wire when unset, and a
  certificate other than the pinned one is the setup's relay check's
  message (its SHA-256's start, *enter its new SHA-256 in Options*).
- `scripts/release/check-windows.sh`: an override for `blake3`'s C build (it comes with iroh), like ring's; measure the binary's real growth (at most +13 MB in the spike; iroh's `metrics` feature costs nothing).
- `machines/setup.rs`, the part this phase needs: the Mother Claude, network and pairing checks with their fixes (the settings pages through `invoke`, the firewall script after the UAC prompt, the keep-awake assertion); `Info.plist` with `NSLocalNetworkUsageDescription`; `GET /api/machines/setup`, `POST /api/machines/setup/fix`, `POST /api/machines/setup/ask`, `POST /api/machines/test`, `GET /peer/setup`, `POST /peer/setup/ask`; the readiness test's steps 1–2; Machines health and its badge.
- UI: the setup's steps 1–4 and 7 (this machine, pairing with the grants on the confirmation screen, every machine's checks side by side, the grants matrix, the test); the Machines page.
- Tests: pairing with a wrong code or a wrong confirmation fails; a pinned-key mismatch is refused; a peer cannot reach admin routes, `/hooks`, `/ws/browser`, `/mcp`; revoke cuts connections; grants per ingress. Every check's detector against recorded outputs (`Get-NetConnectionProfile`, `Get-NetFirewallApplicationFilter`, `socketfilterfw`, `pmset`, `powercfg`) through a command-runner trait; fixes built as argv, never a shell string; a fix asked from another machine changes nothing until confirmed there; Machines stays off while a blocker is left; a blocker that comes back turns health red.
- Exit: the Mac and Windows pair on the LAN and across networks; each lists the other online; **a fresh Windows user on a Public network with a cancelled firewall prompt, and a Mac with Local Network denied, are both walked to green by the setup alone**.

### Phase 2 — see and drive other machines (≈ 3 weeks)

*Built on the `machines` branch (2026-11-11): the peer allow-list of the
session routes (`machines::routes::peer_route_grant`), `X-MC-Acting`
(`machines::acting`), the proxy and the federation
(`machines::federation`, `{t:"remote"}` frames) and the SPA's per-computer
scopes (`src/app/core/machine-scope.ts`). **Exit test passed by hand
(2026-11-11)**: from the Mac and from a phone paired with it, Windows'
sessions were listed, started, driven, approved (session page and Inbox),
stopped, greyed while Windows was offline and back after. A client on
the polling fallback reads the others from `GET /api/machines/sessions`
(`Authed`; the federation's cache per paired computer: `{enabled,
machines: [{machine, name, online, since, loaded, sessions}]}`, each
session stamped with `machine`, a conditional GET like `/sessions`'),
every poll while one is listed and every tenth otherwise (built
2026-11-12; UX §6.1 had `/api/sessions` carry them, a separate route keeps
that one's answer and ETag this computer's alone).*

- `machines/federation.rs`: subscriptions, merge, offline state; frames tagged `machine`.
- The proxy route; `X-MC-Acting`; the dangerous intersection; audit on the target.
- SPA: machine filter and badges; session pages for remote sessions through the proxy; *New session on …*; federated approvals.
- Phone through its desktop.
- Tests: the proxy's authorization matrix (desktop, device with/without dangerous, peer grants); a peer going offline mid-stream; approvals answered from either machine (first wins); a session id present on two machines kept apart.
- Exit: from the Mac you start, watch, answer and stop a session on Windows; so does a phone paired with the Mac.

### Phase 3a — move a session (≈ 5 weeks)

*Started on the `machines` branch (2026-11-11): `machines/code/`, the
snapshot, both transfers, both landings, undo and the proof, ported from S2
with its cases as tests (F10 above). The backend of the move followed:
`machines/lease.rs` (leases, exchanged on every hello and on `/peer/leases`,
enforced in `ControlRegistry::spawn`), `machines/moves/` (the transaction,
both sides, its records, recovery, take over and undo; the move note in
`write_user`), `machines/move_routes.rs` (§7's routes and the peer's),
`backup::session_pack` (the package and its landing, split out of the
backup's archive and pull code) and `tests/moves.rs`. The screens
followed: the move sheet and its progress
(`shared/move-session-sheet.component.ts`), *Lives on …* with *Bring it
back here* and *Take over…* on a session's page, *Recent moves* on the
Machines page and the session's Info tab (`core/moves.service.ts`). Another
computer does not pass its `move` frames on yet (`federation.rs`), so the
SPA asks about a move it follows there until it ends; `?mock=1&mockmachines=1`
shows them all. The user guide is `docs/MACHINES.md`.*

**Decisions taken for building it (2026-11-11)**, where §5.7–§5.10 left a
choice open:

1. **Files travel as chunks the source pushes**: `PUT /peer/moves/:id/files/:name?offset=N`
   (≤ 8 MiB a request, the target appends), `GET …` answers the size it has,
   so a broken transfer resumes; the sha256 of each file is checked before
   landing. No zstd: the archive is gzipped and a bundle is already packed.
2. **The package** is the session's files in the backup's layout (the same
   file list as `archive_session`, with `.mc/meta.json`), an unencrypted
   tar.gz (the link is encrypted), made by a pack function split out of the
   backup's archive code. **Landing** reuses the pull's staging, relocation,
   placement, fast-forward replace and conflict copy, and its undo folder.
3. **The account there**: the same name (case-insensitive), else Default for
   Default; anything else, or one not signed in, is a preflight blocker.
4. **The folder there**: a root (§5.12), else a folder pair, else the same
   path when it exists there, else preflight asks to choose among the
   target's candidates (its projects).
5. **Freezing**: an owned live session as `claude/switch.rs` does (after this
   turn, or now with an interrupt); an ended one needs none. A foreign live
   session moves only **as a copy** (`asCopy`): nothing frozen, no lease, and
   the target forks it (`--fork-session`, a new id).
6. **The lease** is enforced in `ControlRegistry::spawn` (`409
   session_lives_elsewhere`, naming the machine), so every owned launch path
   refuses; leases are exchanged on every hello and on `/peer/leases` (the
   higher epoch wins, a tie goes to the higher machine id).
7. **The move note** is kept with the lease until delivered and prepended
   once, by `write_user`, to the next message the session gets.
8. **A recipe is not run by Mother Claude**: its commands join the move note
   as a request to Claude, so they run through the session's own tool
   approvals — no new way to run code on another machine (Phase 4 adds that,
   with its grant).
9. **Who starts it**: `POST /api/sessions/:id/move` on the machine the session
   lives on; the other machine's screens call it through the proxy (the
   `move` grant). The target's `/peer/moves/*` need the target's `move` grant
   for the source. Tests record the target's resume instead of starting
   `claude` (as the setup's fixes are recorded).
10. **A move's record** (`machines/moves/<id>.json`) walks `preflight →
    freezing → packaging → sending → landing → landed → committed → resumed`,
    or ends `failed`, `cancelled` or `undone`; after a restart a move before
    *landed* is failed and unfrozen, one after it is finished.
11. **Take over** needs the session's files on this machine (an earlier move
    or a pull; copies are 3b). **Undo move** runs on the target: stop, undo
    the code, put the placed files in the undo folder, lease back to the
    source, which resumes the session if it ran when frozen.
12. **Roots** live in `machines.json`, replicated to peers (last writer wins
    per entry, removals stick); existing folder pairs are imported once as
    two-entry roots. Backup sets keep their pairs as they are; writing roots
    back into them comes later.

    *Built (`machines/roots.rs`, `tests/roots.rs`):* a root is `{id: "rt_…",
    name, paths: {"<machine id>": "<path>"}, updatedAt, by, pair?, removed?}`;
    per root the later `updatedAt` wins (a tie: the higher machine id), a
    removal is a tombstone that wins over any live copy. They are sent to
    every paired machine when they change and when a hello's digest differs;
    a machine takes another's only with the `move` grant it gave it (reading
    needs `view`). A folder pair becomes the root `rt_<the pair's hex>` (the
    same on every machine; its name the last folder name of the side that
    made it), once (`rootsImported`); a pair removed in the backups removes
    its root while nobody edited it. Mapping (`roots::map_between`,
    `map_here`) tries the roots — the longest matching path — then the
    folder pairs; continuing a session from another machine
    (`owned::folder_here_from`) does too.
13. **Machines off blocks use, not pairing or the setup** (built
    2026-11-11): while `enabled` is false, a machine's peer router serves
    only hello, pairing, the setup's routes (checks, asks, accounts,
    origins, the code test, the remote check), settings offers, bulk,
    roots and leases, and answers `403 machines_off` on `/ws`, the proxied
    `/api/*` and `/peer/moves/*` (a move's `DELETE` drop excepted: it only
    takes back what a stopped move left); its own proxy and moves answer
    `409 machines_off`, and its federation dials no link.
14. **The acting person across a second hop** (§5.5): a peer acting for
    another machine — a move that machine started through the proxy —
    sends `X-MC-Acting: machine; dangerous=…` with the right it judged
    (that machine's person ∧ its own `dangerous` grant for it); the
    receiver intersects it with its own grant again. The local-script
    token never changes a move through the proxy.
15. **Ignored files** (§3.6, `moves::ignored`): the plain files under the
    repository's top that git ignores on the source and that the project's
    `carryIgnored` lists (set in Mother Claude, with the dangerous right; a
    committed `.mother-claude/recipe.json` gives commands only), ≤ 1 MiB
    each, ≤ 50, never through a link, travel as `ignored.tar.gz`. Carrying
    them needs the dangerous right on both machines (left unsaid, they
    travel only for a caller holding it); the target writes them once the
    code landed, keeping what they replace for *Undo move*. With code
    through the remote, the preflight runs the setup's remote check on
    both machines.

- `machines/lease.rs`; enforcement in `claude/control.rs` (refuse a session leased elsewhere).
- `machines/moves.rs`: preflight, freeze (reuse `claude/switch.rs`), package (reuse `archive_session`), send/receive (bulk streams), land (reuse the pull's placement through the roots, its conflict rule and undo), commit (lease), resume with the move note.
- `machines/code.rs`: snapshot; both transfers (bundle — the default — and hidden refs on your remote, deleted after the move unless kept, the automatic fallback above 1 GB); both landings (the checkout when clean — the default — and a worktree, with *Remove the worktree*); undo refs; warnings. The choices per project and per move.
- Roots (§5.12) replace folder pairs, existing pairs carried over as two-entry roots.
- The setup's rest: the files, git and Claude Code checks with their fixes; step 5 (accounts matched by name with each machine's sign-in, *Sign in…* there; roots proposed from repos' `origin`; *Clone it there*); step 6 (the code and copies options); the readiness test's steps 3–6; the move's preflight reruns the checks that concern it.
- Recipes; *Bring back*; *Take over*; *Undo move*.
- UI: the move dialog and progress (with the transfer and landing choices, defaulted from the settings); *Lives on …*.
- Docs for the first release: `docs/MACHINES.md` (setup with every check and its fix per OS, pairing, remote control, moves), a "Machines" section in `CLAUDE.md` (D1–D11), `SECURITY.md`, Help topics (en, vi).
- Tests: a failure injected at every step leaves both machines consistent; bring back is a fast-forward; a session continued on both becomes a conflict copy; a lease prevents a second writer; code round trip (from S2) as tests, for each transfer and landing; a target checkout with changes of its own is caught in preflight, which offers a worktree (never a merge); a foreign session moves as a copy; a move whose preflight finds a blocker (a signed-out account there, a missing root, a remote that prompts) stops before anything is frozen.
- Exit: **the NAS round trip is one button**, both ways, with the code — after a setup that left nothing to discover during a move.

### Phase 3b — keep copies current (≈ 2 weeks)

*Built (2026-11-12): `machines/copies/` (`check` — the prefix check: the
size and the SHA-256 of a file's first and last 64 KiB; `source` — the turn
watch and a round: offer, the blob in chunks, apply; `target` — the copy
held, its cap, the move's and the take over's landings; `store`, `wire`,
`routes`), the move's package from the copy (`wire::FromCopy`), *Continue
here from the copy* in the take over, the session menu's *Keep a copy on
…*, the *Copies* card, the project option and the cap, `tests/copies.rs`.
Where §5.9 left a choice: opt-in per session or per project (a session's
own choice wins), the target's `move` grant, after a turn plus five quiet
seconds, the copy keyed `<source>/<account>/<session>`, the cap evicting
the copy refreshed longest ago, never one leased here; the copy's
`index.json` keeps per copy its sizes, its transcript's mark and the last
turn, each file's state in `<copy>/.mc/files.json`. A first copy broken
off restarts whole at the next round (no resume across rounds yet).*

- `machines/copies.rs`: after-turn deltas, prefix hashes, the copies index, size cap.
- *Continue here from the copy of …* (take over from a copy).
- Tests: deltas across many turns equal the source byte for byte; a hash mismatch resends the file; never a mid-turn copy; eviction.
- Exit: moving a marked session is instant; Windows can continue it while the Mac sleeps.

### Phase 4 — work on another machine (≈ 4 weeks)

- `machines/work.rs`; `machine_list`, `machine_run`, `machine_task`; approvals and standing grants; worktrees in the data dir; results as a branch.
- Setup: the `run` grant joins the matrix (off until you turn it on, per pair and per direction); *Claude Code tools* becomes a blocker for the `machine_*` tools; the readiness test gains a `machine_run` echo on each machine.
- Teams: `machine` on a role; the launcher through the peer; `task_*` tools reaching the lead's board.
- Tests: approval gating; worktree isolation (the user's checkout untouched); a task's branch round trip; a role on another machine claiming and finishing a task.
- Exit: a Mac session builds and tests the Windows target, and hands a Windows-only change to a Windows session, without the user switching machines.

**Decisions taken for building it (2026-11-12)**, where §5.11 left a choice
open:

1. **A run happens in a kept work folder** (the user's choice over a fresh
   worktree per run, which rebuilds everything every time): one per project
   and source machine, `<app data>/machines/work/<id>/`, a `git worktree`
   of that computer's own checkout of the project (the folder a root maps
   it to). Before each run it is set to the session's code — the source
   snapshots it (`code::snapshot`, chained, a bundle of what is new) and the
   target resets the work folder to that snapshot, tracked and untracked
   files alike — while the files git ignores (`target/`, `node_modules/`)
   stay between runs, so caches survive. The user's own checkout there is
   never touched. No checkout there: a blocker naming *Clone it there*.
2. **Who agrees**: the target's `run` grant for the source (off until the
   target's desktop turns it on, LAN and internet apart), and an approval
   on the source for every `machine_run` / `machine_task` (a `crate::pending`
   item naming the computer, the folder and the command; the Inbox, the
   session page, a phone) unless a **standing grant** covers it: project +
   machine + command prefix (`cargo test`, `npm run build`), attached from
   the approval card. Attaching one needs the dangerous right (as every
   standing grant does); they are listed and revocable on the Machines page.
   A command `pending::classify_dangerous` flags (config directories, shell
   startup files, …) is a dangerous approval, and the target also requires
   its `dangerous` grant (the `X-MC-Acting` rule).
3. **How it runs there**: macOS through `/bin/zsh -lc` (the login shell's
   `PATH`, as the app resolves it for `claude`), Windows through PowerShell
   7 when present, else Windows PowerShell (`-NoProfile -NonInteractive
   -Command`); `machine_list` names each computer's OS and shell, so Claude
   writes the command for that OS. The environment is scrubbed as for owned
   children (no tokens, no API keys), plus `MOTHER_CLAUDE_RUN=1`. One run
   at a time per work folder (a second waits, up to its timeout); a timeout
   (default 30 min, at most 2 h) or a cancel kills the whole process tree.
4. **What comes back**: the exit code, the duration and the output's last
   64 KiB at once; the whole output kept there for a day
   (`machine_output {id, from}` pages it); files named by a glob listed
   with sizes and fetched on request (≤ 64 MiB each, into the session's
   project under `.mother-claude/artifacts/<run>/`, git-ignored).
5. **Long runs**: `machine_run` waits up to 10 minutes, then answers *still
   running* with the run's id; `machine_wait {id, timeout}` waits again,
   `machine_cancel {id}` stops it. A run's lines also stream to the
   dashboards as a background task of the session.
6. **`machine_task`** starts an owned session there with the task, in its
   own worktree at the snapshot (not the run folder: it edits code); when it
   ends its changes come back as a bundle and become the branch
   `mother-claude/<machine>/<task>` in the asking session's repository
   (never checked out, never merged), with the task session's summary. The
   task session is an ordinary owned session there: listed, driven and
   approved from either computer.
7. **Records**: every run and task is audited on both computers with the
   program's name and a hash of the command line (never the whole line: it
   can carry secrets), the folder, the exit code and the duration.
9. **A team role on another computer** (part 3): a template role may carry
   `machine` — a paired computer's name or id, or `os:windows` / `os:macos`
   (the first online paired computer of that OS that gave this one `run`).
   The lead's launcher starts it *there* through a peer request: that
   computer runs `capacity::admit` itself and starts the role as an owned
   session in a task worktree at the lead's code snapshot (part 2's
   machinery), with the role's charter, settings and team tools exactly as
   a local role gets them, and the ownership guard's path globs. The board
   stays with the lead: the role session's `task_*` tools there reach the
   lead's board through the peer link, accepted only for the team and role
   the lead started there (a binding the lead mints at start). Stop,
   idle-stop and resume come from the lead's launcher. What the role
   commits comes back as the branch `mother-claude/<machine>/<team>/<role>`
   (forward only) when it marks a task done, and the board records it. Only
   owned role sessions run elsewhere; a foreign session never takes a
   remote role. Needs the target's `run` grant, like every run.
8. **Order**: runs first (`machine_list`, `machine_run`, `machine_wait`,
   `machine_output`, `machine_cancel`, the grant, approvals and standing
   grants, the setup's `run` column and readiness echo, the runs list on the
   Machines page); then `machine_task` and its branch; Teams' `machine` on a
   role last.

*Runs built on main (2026-11-12), decision 8's first item:
`machines/work/` (`mod.rs` the runs and their records, `source.rs` the
asking computer, `target.rs` and `shell.rs` the one that runs them,
`output.rs`, `grants.rs` the standing grants, `routes.rs`, `mcp.rs`),
`code::work` (`fetch_snapshot`, `reset_work_tree`),
`Registry::add_with_limit` and `Tool::limit` in `mcp`; the routes
`GET /api/machines/runs`, `GET …/runs/{id}`, `POST …/runs/{id}/cancel`,
`GET /api/machines/run-grants`, `DELETE …/run-grants/{id}` and
`/peer/work/*`; the five tools; the setup's *Run* row, the *Claude Code
tools* check and the readiness echo; the approval card, the *Runs* card and
its *Always allowed runs*; `tests/work.rs`. The user guide is
[MACHINES.md](MACHINES.md) "Running commands on another computer".
Choices made where the decisions left one: results are plain text for the
model (no `structuredContent`; `machine_list` alone answers JSON), so the
output reads as a terminal's would; a listed file is fetched with
`machine_output {id, artifact}` rather than a sixth tool; `cwd` defaults to
the session's own folder inside the project; the target runs the danger
classifier too and a flag on either side counts (the target's verdict comes
back with *where it would run*, before the approval, and the target checks
again when the run starts); a standing grant never covers a dangerous
command, and none is offered for a shell or an interpreter alone; standing
grants are kept in `machines/run-grants.json` (0600); when a command ends,
what it left running in the
background is stopped too (its process group, its job object), so nothing
holds the work folder's files; a work folder's id is derived from the
source computer and the project's path, the same on both; a run is a
background task of its session in the session's detail (`tasks`, kind
`machine`), its output through the existing task routes, live through
`{t:"machines", what:"runs"}` and the task output's 3 s refresh (another
computer's too, since 2026-11-13: the federation passes its `runs` and
`tasks` frames on as `remote` frames, and a remote session's page, in that
computer's scope, refetches on them); turning
Machines off, unpairing or taking back `run` (the column the run came in
by) stops the other computer's runs; the whole output is kept up to 256 MiB; the readiness test runs the
echo both ways (its `run` and `runBack` lines).*

*Tasks built on main (2026-11-12), decision 8's second item:
`machines/work/tasks/` (`mod.rs` the records, the state machine and the
session's hooks, `source.rs` the asking computer, `target.rs` the one that
runs it), `code::task` (`task_result`, `land_task_branch`,
`remove_task_tree`, `drop_task_refs`; `SnapshotRequest::extra_parents` and
`label`); the hooks `tasks::turn_ended` (the control stream's `result`),
`turn_started` (`owned::send`, a cross-session turn) and `session_exited`
(the reaper); `machine_task`, with `machine_wait` and `machine_cancel` taking
a `task_…` id; the routes `GET /api/machines/tasks`, `GET …/tasks/{id}`,
`POST …/tasks/{id}/cancel`, `DELETE …/tasks/{id}` (`ManageData`) and
`/peer/work/tasks/*`; the `tasks` capability and frame; the approval card,
the Runs card's *Tasks*; `tests/tasks.rs`. The user guide is
[MACHINES.md](MACHINES.md) "Handing a task to another computer". Choices
made where the decisions left one: the worktree is
`<app data>/machines/work/tasks/<task>/<project>` (the project's name, so
the session's project reads as it should; the record and the transfers sit
beside it, in folders a project's name never takes); *done* ignores
background work that may resume the session (the answer says so, and a
turn it takes up by itself updates the branch when it ends); the branch's
tip is the session's own HEAD when it committed everything and the last tip
is in its history, else a snapshot commit whose second parent is the last
result — each result descends from every earlier one, so the branch only
fast-forwards; a branch someone committed onto, or that a worktree has
checked out, is not moved (the record names the newer result's commit); a
cancel, a stop there or a process that ends mid-turn still brings back what
the session did; the task session is named *From <computer>: <first line>*
on both lists, and the task is a background task of the asking session too;
the asking computer keeps following a task that ended (its long poll, every
5 minutes once it ended an hour ago) so a later turn's result lands; *Remove*
needs the desktop or a paired device (never the local-script token), is
refused while the task works, and removes the other computer's side when it
can be reached (its 7-day prune otherwise); Machines off, unpairing or
taking back `run` there cancels the tasks that still work, never one whose
turn ended; `machine_list` says `mayTask`; a done task's session idle with
nothing to do for 30 minutes (the launcher's idle-stop rule,
`launcher::idle_stop_due`, applied by its loop through `tasks::stop_idle`)
is stopped there, its worktree and record kept, and a message to it from
either computer resumes it in its worktree (`tasks::target::resume_with`,
ahead of the message route), so a later end still moves the branch
forward.*

*Team roles built on main (2026-11-12), decision 8's third item (decision
9): `RoleDef::machine` (validated; a lead never runs elsewhere; an
unpaired computer a template-time warning, `machine_warnings`, and a
`422 machine_unknown` when a team starts), `RoleOverride::machine` (the
start sheet's *Computer*); `machines/work/roles/` (`mod.rs` the binding —
`check_binding` — the resolution of `machine` — `pick`, `os:` sticky to
the computer a role ran on — the computer's records and guard rule,
`lead.rs` the lead's half, `host.rs` the other's, `routes.rs`);
`launcher::start_role` → `start_remote`, `RoleLaunch`/`launch_options`
shared by both computers, `charter_for`, `claim_for`, `opening`,
`give_back`; `delegation::deliver` and `stop_role` through the peer link;
`teams::RemoteSlot` (the binding's id and the secret's SHA-256, the
worktree there, what came back) and `RemoteStatus` (what was last heard,
`lead::refresh_all` from the loop); the team tools as
`mcp::{Member, run, run_remote, dispatch}`; `Task::remote`
(`board::RemoteWork`); `RunningWorker::for_machine` and
`RoleSession::for_machine`; `code::{role_branch, task_unchanged,
contains}`; the routes `/peer/teams/roles/*`, `POST /peer/teams/board`,
`GET /api/machines/roles`, `POST …/roles/{binding}/stop`, `DELETE
…/roles/{binding}`; the capability `teamRoles`; the frame `{t:"machines",
what:"roles"}`; `tests/teams_remote.rs`. The user guides are
[TEAMS.md](TEAMS.md) "A role on another computer" and [MACHINES.md](MACHINES.md)
"A team role on another computer". Choices made where the decision left
one: the lead keeps the secret's SHA-256 only, the other computer the
secret, and a board call must also name the role's current session; the
other computer runs its admission before the code is sent (`prepare`) and
again at the start, and the role waits on the lead's team page with its
reasons (*on DESKTOP-PC: …*), while its Capacity page shows the queue and
the session (*for MacBook*) — and, since 2026-11-13, the lead's Capacity
page lists the role apart (`remoteRoles`: where it runs, running, waiting
there with its reasons, or stopped; never in the lead's counts); the role's worktree is kept between starts
and moved to the lead's newer code only when nothing in it is unreturned
and the lead's code holds what came back (the next result stays a
descendant of the last, so the branch only moves forward); a result is
taken only at `task_done` (done or failed) and lands before the task
finishes, within 80 s — later, the task records it when it arrives; the
role's packet and notices go to its session through the peer link
(`SendMessage` never crosses computers: the charters say so); the other
computer stops a role whose lead was not heard from for 30 minutes while
idle; a resume continues the conversation there in place, never a fork
— and, since 2026-11-13, a direct message (text only) to its stopped
session resumes it too, through the lead's launcher with the role's
options and that computer's admission, then delivers it
(`lead::resume_with`: from the lead's computer, intercepted in its proxy;
from that computer, `host::wake_with` → `POST /peer/teams/roles/wake`,
accepted with the binding and its secret as a board call is; a role
without approvals needs the sender's dangerous right);
removing the team removes the role there, and *Remove* there makes the
lead mint a new binding (whose results cannot move the old branch: the
task names the commit).*

### Phase 5 — more machines and polish (≈ 3 weeks)

*Built on main (2026-11-12): `machines/intro.rs` (introductions: the
offer, its answer on each side, the door it opens to the key it names) and
the introduced pairing in `pairing.rs` (no code: the joiner dials the key
named, `join` names the introduction, the six digits are seeded by the
introducer and the introduction); `machines/revocation.rs` (*Revoke
everywhere*: revoked here, the others asked through `/peer/revoke`, kept
in `machines.json` until each took it, delivered on its next hello);
`machines/versions.rs` (each peer's `skew` and what it lacks, the setup's
`app` warning when another computer runs a newer Mother Claude, *Update
it* as an asked action that opens *Software updates* there); the hello's
`pairedWith`; the capabilities `sync`, `introductions` and
`revokeEverywhere`; `autostart.rs` and `tray.rs` (*Start Mother Claude when
you sign in, in the tray*, §5.13's first step); the routes `POST
/api/machines/introduce`, `POST /api/machines/introductions/{id}`, `POST
/api/machines/{id}/revoke-everywhere`, `GET|PUT /api/app/autostart`,
`POST /peer/intro`, `/peer/intro/withdraw`, `/peer/pair/intro/decline`,
`/peer/revoke`; `tests/more_machines.rs`. The user guide is
[MACHINES.md](MACHINES.md) "Three computers or more" and "Starting at
sign-in, in the tray". Choices made where this section left one: the
introducer picks who waits (the computer already paired) and who dials (the
new one); an introduction carries no secret — the digits' strength is the
commit-then-reveal over both keys, the introduction only its seed — and
lasts ten minutes; a computer learns whom another is paired with from its
hello (ids only, to a computer it lets see its sessions), so an offer stops
once two are paired; *Revoke everywhere* needs no grant on the computer
asked (it only removes trust) and matches the computer by its id or its
key; the login item is the system's own (the `Run` value, a LaunchAgent),
not `tauri-plugin-autostart` (it writes the program's path unquoted, to
the machine-wide key when the app runs elevated); a tray icon only on a run
the login item started. Since 2026-11-13, one Mother Claude at a time:
`tauri-plugin-single-instance`, registered first, so a second launch exits
before its server, endpoint or window start, and the running one shows its
window (`lib.rs` `second_launch`) — a second launch with `--tray` (the
login item while it already runs) stays quiet. The tray's menu speaks the
window's language (en, vi): the language is chosen per device and kept in
the webview's own storage, so the window tells the desktop through the
`tray_language` invoke at its start and at each switch (an OS concern, as
the architecture allows), and the menu is in English until it has.*

*The dev mock covers every Machines screen (2026-11-13):
`?mock=1&mockmachines=1` serves Machines on with DESKTOP-PC (online) and
BUILD-BOX (offline, an older Mother Claude: its skew, what it lacks, its
*Update Mother Claude* check) paired, DESKTOP-PC's older Claude Code, an
introduction to STUDIO-MINI to answer, a revocation BUILD-BOX has still to
take, code sync (one project in sync, one in conflict), copies, moves (the
preflight's files git ignores, the warnings a landing keeps), runs (one
that runs and ends while you look, a build with an artifact, a failure,
one asked of this computer), a standing grant, tasks (one done with its
branch, one at work, one handed here), a team role run for DESKTOP-PC's
lead, a run and a task waiting for approval on "Build and notarize
v0.5.1", and the startup setting (`core/mock/mock-machines.ts`,
`mock-moves.ts`, `mock-runs.ts`, `mock-copies.ts`; `mock-machines.spec.ts`).*

- Introductions in the setup's step 2 for a third machine and more; *revoke everywhere*.
- Versions page and skew warnings; tray-only start at login.
- Docs: `docs/MACHINES.md` for 3b, 4 and more machines; `BACKUP.md` (how it relates), `REMOTE.md` (devices vs peers).

**First release worth shipping: Phases 0–3a (≈ 13–14 weeks)** — two weeks
more than without the setup, spent so that nothing is found half-way through
a move. 3b and 4 can follow in any order.

## 13. Test plan

- **Unit**: grants, the lease's epoch rules, the move state machine, code
  snapshot / apply with temp repos, delta hashing.
- **Integration, in one process**: two `AppState`s with their own data dirs and
  homes, joined by an in-process transport (iroh endpoints on loopback, no
  relay); the phone path's tests rerun with `Ingress::Peer`; failure injection
  at every move step.
- **On both machines** (like the release legs): the move checklist — dirty
  tree, CRLF repo, LFS repo, a diverged branch, sleep mid-transfer, Wi-Fi
  off/on, across networks via a hotspot, three machines.
- **Setup, on fresh OS users** (like the release legs): from the worst state
  of S5 to green on each OS; then a blocker brought back (the firewall rule
  deleted, the network set to Public, an account signed out) shows red within
  one health run and is named by the next move's preflight.
- **Live, ignored by default** (they spend tokens): a move then a turn on the
  target with the real CLI; `machine_task` end to end.

## 14. Risks and open questions

| Risk | Answer |
|---|---|
| iroh's API or relays change | Spike first; the transport is one module behind a trait; the fallback (LAN HTTPS + Tailscale) stays possible. |
| Claude Code changes its transcript layout or resume rules | All reads stay in `claude/`; the move reuses the pull's placement, already tested against the CLI; S3 re-run on CLI updates. |
| Git edge cases (LFS, submodules, huge repos) | Preflight warnings; the remote mode as an escape; worktree mode when unsure. |
| Machines with different Claude Code versions | Versions page; a warning before moving to an older one. |
| Users expect two-way live editing | The UI says *Lives on …* and offers *Open there* / *Bring back*; D1 explained in Help. |
| n0's public relays limit or change their service | The relay is a setting (D10): your own `iroh-relay` is one field away, and the readiness test shows when the chosen relay stops answering. |
| An OS changes a permission's page or its behaviour (macOS Local Network, Windows Settings URIs, firewall prompts) | Checks try the thing rather than read undocumented state where they can (S5); a check that cannot decide says *Unknown* with the page to open, and does not block on its own; S5 rerun on each new macOS and Windows release. |
| A managed machine (MDM, Group Policy) where the user may not change the firewall or the network profile | The setup says the setting is managed and what to ask its administrator; with a relay those checks are warnings, so Machines still works, relayed. |

**Decisions taken (2026-11-10)**

1. **Relays: both.** n0's public relays by default; your own `iroh-relay`
   (the NAS, a VPS) or no relay, chosen in the setup and the same on every
   machine (D10, §5.3).
2. **How code travels: both.** Machine to machine (a git bundle) by
   default; through your remote (GitHub or another, hidden refs) to choose
   in the setup, per project or per move (D10, §5.10).
3. **Where code lands: both.** The target's checkout when it is clean by
   default; always a separate worktree to choose the same way (D10, §5.10).
4. **Setup before use.** *Set up Machines* checks and fixes every OS
   permission and other blocker on every machine, the grants included
   (`view`, `control`, `move` ticked on each pairing's confirmation; `run` and
   `dangerous` off), and ends with a readiness test between every pair;
   nothing unlocks while a blocker is left (D11, §5.14).
5. **Order as proposed:** remote control (Phase 2) before moving sessions
   (Phase 3a).

## 15. Best practices, starting today

Without any new code:

- **Drive Windows from the Mac now**: pair the Mac's browser with the Windows
  Mother Claude as a *device* (Windows: Remote → Pair a device → Same Wi-Fi;
  on the Mac open `https://<Windows address>:6725` and trust its certificate,
  as for a phone). You get the Windows dashboard on the Mac: its sessions, its
  approvals, sending messages. Across networks, use Tailscale on both and
  *Also listen on Tailscale*.
- **One root folder per machine** (`~/development/projects` ↔
  `D:\development\projects`) and **one folder pair** at that root, not one per
  project.
- **`.gitattributes` with `* text=auto eol=lf`** in every project; the same
  Claude Code version on both machines (`claude update` on each); toolchains
  pinned in the repo (`.nvmrc`, `rust-toolchain.toml`, `global.json`).
- **Commit before switching**, even a WIP commit on a branch, push it, and pull
  on the other machine before *Continue*: the session then sees the code it
  remembers.
- Keep **dependencies and build output per machine** (never in the NAS or in
  git); after switching, run the install step on that machine first.
- Keep the **NAS backup** for when the machines are not online together, and as
  a backup: the Machines feature will not replace it.
