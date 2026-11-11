# Machines — Phase 0 results

Run on the MacBook (macOS 27.2.1, arm64) on 2026-11-10 with iroh 1.4.1,
Claude Code 2.1.322, git 2.54.0, then on the Windows desktop (Windows 11
Pro 26200, x64; Claude Code 2.1.317, Git for Windows 2.45.1 with Git LFS
3.5.1) — see § "Windows half". Raw outputs are written to `out/`
(git-ignored).

## Summary

| Spike | Mac half | What it changes in the plan |
|---|---|---|
| S1 transport | **Works**, and between the machines: the Mac dialled Windows by its key alone (n0's DNS, or mDNS with no relay) and ran direct from the first packet, **as fast as plain TCP** on the same Wi-Fi (26–30 MiB/s, the link's ceiling); across two networks (a phone on mobile data) hole punching failed and the relay carried it at 1.1 MiB/s. On each: Windows: mDNS finds a peer by id alone (0.4–0.8 s) — the Mac's failure was macOS's permission; ~260 MiB/s direct on one PC. Mac: **Works.** The app's axum/hyper stack, WebSocket included, runs unchanged over iroh streams; all three relay settings connect; dialling by id alone works through n0's DNS and moves to a direct path in 0.1 s. mDNS found nothing: macOS refused LAN traffic (S5). | §5.3: what *No relay* and *Your own relay* really need; peers keep each other's last addresses. Binary size to watch. |
| S2 code | **Works** on both: 132 of 132 as designed on each (Windows with Git LFS). Windows found two more fixes (F8, F9). Mac: 132 of 132 matrix runs as designed (8 skipped: no Git LFS on the Mac): both transfers, both landings, undo and bring-back round-trip a dirty tree exactly; every line-ending combination is tree-exact. A 50 MB repo moves as a 2 MB bundle, 1.6 KB the next time. | §5.10 gets nine fixes (F1–F9): F1 and F8 because a snapshot could silently lose an edit or a long path. |
| S3 Claude Code | **A move note given as `--append-system-prompt` is ignored on resume.** Claude Code 2.1.322 records a conversation's first system prompt in the transcript and replays it on every resume. A session resumed from another folder notices the new folder. | §3.5 and §5.8: the move note becomes a message in the conversation. Verify on Windows that the platform and shell update too. |
| S4 UX | Screens drafted: `UX.md`, eight decisions for you at its end. | Four routes to add to §7 (listed in `UX.md`). |
| S5 setup | Every check is readable as a standard user on both. Windows: firewall off on every profile, Private network, never sleeps, Claude Code 2.1.317 (older than the Mac's). Mac: every macOS check is readable as a standard user. **Two real blockers found on this Mac:** Mother Claude has no Local Network access, and the Mac sleeps after 1 minute idle. | §5.14: the Local Network row needs `NSLocalNetworkUsageDescription` in the app first; the check runs inside the app. |

## S1 — transport

`mc-spike local`: both ends on this Mac, one scenario per D10 setting, release
build. Requests open a new QUIC stream each (HTTP/1.1 per stream); *raw* is a
bare stream; transfers were 256 MiB each way locally and 4 MiB through n0's
relays (to be polite to a free service).

| Scenario (setting) | Peer found by | Connect | Path | Request p50 | WebSocket echo p50 | Raw up / down | HTTP down / up |
|---|---|---|---|---|---|---|---|
| direct (*No relay*) | its address (ticket) | 1.8 ms | direct | 0.22 ms | 0.11 ms | 79 / 78 MiB/s | 76 / 78 MiB/s |
| mdns (*No relay*) | id + mDNS | **failed**: "No addressing information available" | — | — | — | — | — |
| own-relay (*Your own relay*) | id + the relay's URL | 2.3 ms | relay (local, every packet) | 0.29 ms | 0.11 ms | 152 / 154 MiB/s | 156 / 153 MiB/s |
| public-relay (*Public relays*, relay forced) | id + relay (ticket) | 79 ms | relay (n0) | 79 ms | 76 ms | 2.6 / 2.3 MiB/s | 1.0 / 2.9 MiB/s |
| n0-dns (*Public relays*, the default) | **id alone** (n0 DNS) | 363 ms | relay, then **direct after 0.1 s** | 0.35 ms | 0.11 ms | 66 / 71 MiB/s | 72 / 75 MiB/s |

Findings:

- **The API rides on it unchanged.** `hyper::server::conn::http1` with
  `.with_upgrades()` over `tokio::io::join(recv, send)` serves the axum router
  as is; axum's `WebSocketUpgrade` works; the client side is
  `hyper::client::conn::http1::handshake` and `tokio_tungstenite::client_async`
  on a fresh stream. 32 concurrent requests on 32 streams: 2.5 ms. HTTP costs
  nothing measurable against a raw stream.
- **Addressing by key works**: `remote_id()` is the authenticated peer; the
  spike's allow-list and one-time pairing code sit on it. Endpoint id = the
  pinned key of §5.4. End to end on this Mac (`serve --pair-code auto
  --forward …`, then `get <id>` by id alone): pairing, then the app's real
  `/api/sessions` through the forward (a local-script request, so the app's
  own auth decided) and `/api/health`; a route not on the forward's list →
  403; another key after the one-time code was used → the connection closed
  with "not paired".
- **Relays.** n0's public relays: ~2–3 MiB/s and a ~78 ms round trip from
  here (Mac → relay → Mac), fine for control, slow for a large first bundle
  (500 MB ≈ 3–4 minutes). An `iroh-relay` of your own is a plain HTTPS
  service; with it a peer is dialled as *id + that relay*, so **no n0 service
  is asked anything** (n0's DNS is needed only with the public relays, to
  find a peer's current home relay).
- ***No relay* is the LAN only.** iroh coordinates hole punching and learns
  public addresses through its relays; without one, two machines on different
  networks meet only if one has a public address or a router port mapping.
  §5.3's table should say so (to confirm across a hotspot in the Windows half).
- **mDNS failed because macOS refused the LAN** (S5), not because of the
  library — to confirm on Windows, where no such permission exists. Peers
  should also **remember each other's last direct addresses** and try them
  first: that works where multicast is filtered (guest Wi-Fi, some routers).
- The direct path's 78 MiB/s with both ends on one Mac is a per-host UDP/CPU
  figure (a local relay over TCP did 150); the LAN figure needs two machines.
- **Binary size** (release, stripped, thin LTO): the same HTTP stack without
  iroh 2.3 MB; with iroh (default features), mDNS and tickets 15.4 MB — **at
  most +13 MB**, against an app binary of 23 MB today (arm64). Part of that
  (rustls, ring, tokio) the app already has; Phase 1 measures the real
  increment. Turning off iroh's default `metrics` feature saves nothing
  (15.38 MB either way).
- **The machine key** is 32 bytes (`SecretKey::to_bytes` / `from_bytes`): it
  goes through `claude/secrets.rs` like every other secret (Keychain,
  Credential Manager, a 0600 file elsewhere), so it needed no spike; the tool
  keeps its throwaway keys in a 0600 file.
- **Windows build**: the spike type-checks for `x86_64-pc-windows-msvc` from
  the Mac. `blake3` (through iroh) compiles C for Windows, so
  `scripts/release/check-windows.sh` will need an override for it, like
  ring's, when iroh enters the app.

## S2 — code

`node code/roundtrip.mjs matrix` (Node, git only), in scratch repos with the
user's git config shut out; every run also proved the source untouched
(HEAD, refs, index bytes, status, every file). The full write-up, the
matrix and the two-machine steps are in [`code/RESULTS-S2.md`](code/RESULTS-S2.md).

| | |
|---|---|
| Matrix | 132 of 132 as designed: staged / unstaged / untracked / deleted / renamed / binary / exec bit / odd names, every `autocrlf` and `eol` combination, behind (fast-forward), diverged and dirty targets (stopped, worktree offered), unborn and detached, submodules, symlinks, case, a shallow target, a round trip and back; 8 LFS runs skipped (not installed here) |
| Small move | snapshot 0.4 s, apply 0.16 s, verify 0.12 s |
| 2,000 files, 50 MB, 77 changes | snapshot ≈ 0.6 s; bundle **2.06 MB** when the target has the base (a full one: 23.2 MB); apply ≈ 0.5 s; the next move **1.6 KB** with chained snapshots |

What §5.10 needed (now folded into the plan):

- **F1** keep the index copy's mtime, and cross-check the snapshot against
  `git status` (re-hash on a mismatch): a plain copy silently missed an edit
  in 6 of 60 snapshots.
- **F2** the staged tree travels as a commit of its own (`…/{snap,index}`).
- **F3** plumbing still runs hooks (`post-index-change`,
  `reference-transaction`, `pre-push`): every git call gets an empty, private
  `core.hooksPath`.
- **F4** `write-tree` and `status` rewrite the index they read: only copies,
  and `GIT_OPTIONAL_LOCKS=0`.
- **F5** fetches must not follow submodules, or one missing submodule commit
  fails the whole fetch.
- **F6** chain a session's snapshots, or every move resends everything.
- **F7** bring-back needs a *parked* rule: the source still holds what it
  sent, so "clean only" refused every return.

Known limits, warned in the preflight: +x lost on Windows for unstaged mode
changes, untracked symlinks arrive as files without Developer Mode, case-only
renames on disk, names Windows cannot hold, submodule contents. Not covered:
Windows itself, Git LFS, GitHub accepting `refs/mother-claude/*`, repos over
1 GB.

## S3 — Claude Code

Haiku, in scratch folders under `/tmp` (about 6 cents). The questions were
how a moved session learns that it moved, and what a resumed session
believes about its machine.

| Run | Result |
|---|---|
| Fresh session, `--append-system-prompt`, text input | the model sees it |
| Fresh session, `--append-system-prompt`, stream-json input (owned sessions' mode) | the model sees it |
| Resume, new `--append-system-prompt`, stream-json | **ignored** ("I have no move reference") |
| Resume, `--system-prompt-snapshot off` + new note, stream-json | **ignored** |
| Resume, `--system-prompt-snapshot off` + new note, text input | seen |
| Resume after that, default snapshot, another note | the **recorded** prompt wins; the model calls its previous, note-based answer wrong |
| Resume with `--fork-session` + note, stream-json | ignored (the fork keeps the record) |
| Resume from **another folder** (same id) | found; the model reports the new folder, "changed from …" |

Why: `claude --help` (2.1.322) documents `--system-prompt-snapshot on|off`,
on by default: the system prompt is rendered on a conversation's first
request — an `--append-system-prompt` included — recorded, and every later
request **and resume** sends the record as-is "even when a later launch
passes different text, until the conversation is compacted". The transcript
now carries it: `api-request-shape`, `api-request-blob` and `api-request`
records and `prompt_snapshot` attachments; the environment (`Platform:
darwin`, `Shell: zsh`, the working directory) is an `environment`
attachment, and a changed folder on resume is reported again.

Consequences:

- **The move note must be a message**, recorded in the conversation: the
  first message after a move carries *"Moved from MacBook to DESKTOP-PC …;
  paths in earlier messages refer to the MacBook; the project is now
  D:\…"* — prepended to your next message, so it costs no extra turn. It
  works for owned and foreign continuation alike and survives every later
  resume.
- **To verify on Windows**: a Mac transcript resumed there reports
  `Platform: win32` and the Windows shell, the way it reported the new
  folder here. If the platform stays `darwin`, a move must also end the
  recorded prompt (a compaction) — the plan's fallback.
- Not run: an older CLI resuming a newer transcript. It would run an old
  Claude Code against your real `~/.claude.json`; the setup's "same version
  everywhere" check covers the risk until a safe way exists (a directory
  account kept for tests).
- Aside, outside Machines: a team role's charter is passed with
  `--append-system-prompt` (`orchestration/launcher.rs`). It is recorded on
  the role session's first launch, so a charter changed later reaches a
  resumed role session only after a compaction.

## S4 — UX

`UX.md`: the Machines page, pairing on both computers, the seven setup steps,
what the other computer shows when asked to fix something (with Windows'
administrator prompt), Machines health, sessions across machines, the move
sheet with progress / undo / bring back / take over / conflict copy, and the
phone variants. It ends with the routes it adds to §7 and eight decisions for
you, each with a recommendation.

## S5 — setup checks (macOS)

`mc-spike checks`, as a standard user, nothing changed:

| Check (§5.14) | Detected by | Here, now |
|---|---|---|
| Mother Claude runs | `GET /api/health` | ok, 0.10.2 |
| **Local Network access** | a datagram to the gateway and an mDNS query | **refused: `No route to host` (os error 65)** — from a process Mother Claude launched (this session runs under it), and from a launchd job alike |
| Application firewall | `socketfilterfw --getglobalstate`, `--getblockall`, `--getappblocked` | off; Mother Claude permitted |
| Stays awake | `pmset -g` | **`sleep 1`**: the Mac sleeps after 1 minute idle |
| Roots | listing `~/development/projects` | listable (not a protected folder) |
| git | `git --version`, `git lfs version`, config | 2.54.0; **no Git LFS**; `core.autocrlf` unset |
| Claude Code | `claude --version`, `claude auth status --json` | 2.1.322, signed in (only *signed in* and *how* are kept) |

Findings:

- **Mother Claude has no Local Network access on this Mac today.** macOS
  attributes the permission to the *responsible process*: the app for
  everything it launches (its sessions included), the binary itself for a
  launchd job. Both were refused without a prompt. The app has no
  `NSLocalNetworkUsageDescription` in its `Info.plist`, so Phase 1 adds it
  and the check must run **inside the app**, never from a helper. Until it is
  allowed, every connection from this Mac to another machine goes through the
  relay, even at home. Nothing the app does today needed it (phones connect
  *to* the Mac).
- **The Mac sleeps after a minute idle**, so the keep-awake assertion (§5.14)
  is not optional for a machine others use.
- Each macOS check was readable without an administrator; none needed a
  prompt except Local Network, which cannot be read, only tried.
- Not run: the *worst state* walk on a fresh macOS user (firewall blocking
  all, Local Network denied then allowed), which needs your administrator
  password and your OK. Same for Windows.

## Windows half (2026-11-10)

Run on the Windows desktop by a Claude session there, as a standard user
(not elevated).

### S5 — setup checks (Windows)

`mc-spike checks`: **every check was readable without an administrator.**

| Check (§5.14) | Here, now |
|---|---|
| Mother Claude runs | ok, 0.10.2; its script token found in `%APPDATA%\motherclaude\Mother Claude\data` |
| Network profile | **Private** (Wi-Fi), same subnet as the Mac |
| Defender Firewall | **off on all three profiles** (Domain, Private, Public); no rule for Mother Claude or the spike — so nothing blocks, and no *Allow access?* prompt appeared when the spike first listened |
| LAN sends | allowed: a datagram to the gateway and an mDNS query both left (Windows has no Local Network permission) |
| Controlled Folder Access | off |
| Long paths (registry) | on; git `core.longpaths=true` in your global config |
| Developer Mode | on, but git `core.symlinks` is false (Git for Windows' default) |
| Sleep | never (standby 0 on AC and DC) |
| Git Bash | found (`C:\Program Files\Git\bin\bash.exe`) |
| git | 2.45.1, **Git LFS 3.5.1** |
| Claude Code | **2.1.317 — older than the Mac's 2.1.322** (the setup's "same version everywhere" warning would fire); signed in |
| Roots | `D:\development\projects` listable |

The firewall being off everywhere is not a blocker for Machines, but it is
the state most installs are *not* in: the walk (`setup-walk/windows.ps1`,
below) turns it on to see the blocked case. A signed-out account was shown
safely: `claude auth status --json` with `CLAUDE_CONFIG_DIR` set to an empty
scratch folder reads `loggedIn: false`.

### S1 — transport (Windows, both ends on this PC)

| Scenario | Connect | Path | Request p50 | WebSocket p50 | Raw up / down | HTTP down / up |
|---|---|---|---|---|---|---|
| direct | 2.4 ms | direct | 0.13 ms | 0.08 ms | 266 / 258 MiB/s | 223 / 230 MiB/s |
| **mdns** | 767 ms (**id alone, found on the LAN**) | direct | 0.13 ms | 0.08 ms | 247 / 242 MiB/s | 231 / 237 MiB/s |
| own-relay | 14 ms | relay (local) | 0.18 ms | 0.15 ms | 112 / 110 MiB/s | 108 / 111 MiB/s |
| public-relay | 94 ms | relay (n0) | 94 ms | 90 ms | 1 / 1 MiB/s | 1.2 / 1.2 MiB/s |
| n0-dns | 275 ms | relay, then direct after 0.2 s | 0.13 ms | 0.08 ms | 288 / 293 MiB/s | 246 / 235 MiB/s |

- **mDNS works on Windows**, in one process and between two processes
  (`serve --relay none --lookup mdns` / `get <id>`: found in 0.36 s). This
  confirms the Mac's mDNS failure was macOS refusing the LAN, not the library.
- Direct throughput on one PC is ~260 MiB/s (the Mac did 78): the per-host
  ceiling is far above a LAN's. n0's public relays gave ~1 MiB/s from here.
- The spike builds natively on Windows (3 min, release) — the cross-check
  from the Mac was right.
- **Between the machines (Mac → Windows, the same Wi-Fi)** — the Mac side in
  the probe app (`setup-walk/macos.sh run …`), the Windows side `serve
  --pair-code auto --forward http://127.0.0.1:6725`, dialled by **id alone**:

  | | |
  |---|---|
  | Pairing, then Windows' `/api/sessions` through the forward | 200, its 4 sessions (counted by status) |
  | Connect | 7.4 ms, **direct from the first packet** (the Windows desktop's LAN address); the n0 relay kept as a standby path (76 ms) |
  | Round trip | 6 ms idle, 16 ms under load |
  | Request p50 (new stream) / WebSocket echo p50 / 32 concurrent | 5.4 ms / 4.8 ms / 9.9 ms |
  | Raw up / down | **29.6 / 26.3 MiB/s** (248 / 221 Mbit/s), 256 MiB each |
  | HTTP down / up | 25.4 / 28.4 MiB/s |

  So the probe app's Local Network access works, and with it the fix
  planned for Mother Claude (`NSLocalNetworkUsageDescription`). The plan's
  ≥ 50 MB/s was **not** reached here — but HTTP equals raw, one PC alone
  does ~260 MiB/s, and both machines are on Wi-Fi (every packet crosses the
  air twice), so the link is the likely ceiling — and a plain TCP transfer on the
  same Wi-Fi settled it:
- **The link is the ceiling.** Plain TCP on the same Wi-Fi, no iroh (curl
  against a Node server streaming zeros): **25.9 MiB/s down, 30.2 MiB/s
  up**; iroh's raw streams did 26.3 / 29.6 — **within 2% of plain TCP**.
  The ≥ 50 MB/s target belongs to the network (wired, or faster Wi-Fi), not
  to the transport; the plan now asks for the link's own speed.
- ***No relay* works between the machines.** `--relay none --lookup mdns`
  on both ends: Windows **found by mDNS from its id alone in 1.3 s**, paired,
  direct (7 ms); request p50 5.4 ms, WebSocket echo p50 4.9 ms; 64 MiB at
  26.7 / 20.9 MiB/s raw up / down, 29.0 / 20.1 MiB/s HTTP up / down (a
  busier minute on the air: a 44 ms round trip under load).
- **A phone hotspot that still reached the home network** (the Mac on the
  phone's hotspot; the direct path went to the Windows desktop's *home*
  address, which mobile data alone cannot reach — the phone was routing
  through its own connection to the home Wi-Fi): relay first (n0, 145 ms),
  **direct after 0.3 s** through the phone's extra router (89 ms), 13–15
  MiB/s raw, 6–10 MiB/s HTTP (16 MiB). The Mac's endpoint, started just
  after the network change, had **no home relay for 15 s**; the connection
  itself then took 0.34 s. In the app the endpoint lives on, but a laptop
  that changes networks meets the same wait — Machines health should say
  "reconnecting", not "offline".
- **Across two separate networks** (the phone on mobile data only, the
  Windows desktop on the home Wi-Fi): found by id alone and connected in
  0.32 s, but **hole punching failed** — the carrier's NAT — and the whole
  run stayed on n0's relay (Singapore, 113 ms): request p50 112 ms,
  WebSocket echo p50 103 ms, 32 concurrent in 0.2 s — fine for driving
  sessions — but **1.1 MiB/s** each way, and the round trip rose to **1 s**
  while the transfer filled the relay path. So: bulk transfers must run at a
  lower stream priority than the API, a move says how long it will take,
  and a large first move across networks suggests your own relay or your
  remote (D10).

### S2 — code (Windows)

`roundtrip.mjs matrix` on Windows with Git for Windows and Git LFS. Two new
findings, fixed in the spike and to be folded into §5.10:

- **F8 — long paths are dropped silently.** Without `core.longpaths=true`,
  Git for Windows meets a path over 260 characters with a *warning*
  ("Filename too long"), **exits 0**, and leaves the file out of `add -A` —
  and out of `git status` too, so F1's cross-check agrees with the wrong
  answer. A 255-character untracked file vanished from the snapshot while
  every check passed. Fix: every git call on Windows runs with `-c
  core.longpaths=true` (your global config already has it; a fresh Git for
  Windows does not), and any "Filename too long" / "could not open
  directory" warning fails the snapshot.
- **F9 — Git LFS writes hooks into the overriding hooks folder.** On every
  LFS case, `post-checkout`, `post-commit`, `post-merge` and `pre-push`
  appeared in the "empty" `core.hooksPath` folder (LFS installs its hooks
  through `git rev-parse --git-path hooks`, which returns the override). A
  shared folder would then run them on every later git call. Fix: a fresh,
  private, empty hooks folder per operation, checked empty before each git
  call and deleted after.
- LFS itself: **`lfs-carried` passes** (the objects travel in the move
  folder); `lfs-not-carried` fails as designed ("remote missing object") —
  carrying them is required.
- Symlinks: with `core.symlinks=false` (Git for Windows' default even with
  Developer Mode) both sides hold symlinks as files — a limit, as expected.

Result after the fixes: **132 of 132 as designed** (90 pass, 22 known limits, 4 stopped by design, 4 expected errors — lfs-not-carried; 12 skipped: exec bits and names Windows cannot hold, which only a Mac can produce), 23 minutes. The 2,000-file / 50 MB repo moved as the same 2.06 MB bundle and 1.6 KB the next time; its snapshot took 42 s here against about 1 s on the Mac, but that includes the spike's own byte-by-byte proof of every file on both sides with Defender's real-time scanning on — Phase 3a times the product's path alone. The real round trip between the Mac and Windows is still to run (it needs the Mac).

### S3 — Claude Code (Windows)

Claude Code 2.1.317 here already has `--system-prompt-snapshot` (on by
default), so the move note must be a message on both machines. **Still to
run**: a Mac session resumed here — no Mac session has been pulled to this
machine yet (Backup › pull is your action).

## Still to run

### On the Mac, with you at it (about 5 minutes)

`bash spikes/machines/setup-walk/macos.sh` in Terminal, as your own user.
It asks before every change and puts every setting back (on Ctrl-C too):

1. **Local Network as the app will meet it**: the spike inside "MC Spike
   Probe.app", whose `Info.plist` has `NSLocalNetworkUsageDescription`. Does
   macOS ask now (the fix Phase 1 makes to Mother Claude)? Then allowed,
   switched off in System Settings, switched on again — each read by the
   check.
2. **mDNS** between two endpoints inside the probe app (the positive case
   this Mac could not show).
3. **The firewall** on with *Block all incoming connections* (sudo, about a
   minute), read by the check as a standard user, then put back.
4. **Sleep**: the timers, and a keep-awake assertion as `pmset` reports it.

The probe app keeps its Local Network permission, so it is also the Mac's
side of the LAN test below: `bash spikes/machines/setup-walk/macos.sh run
<mc-spike arguments>`.

### On the Windows desktop, with you at it (about 5 minutes)

Done here already: the build, S5's checks, S1 on one PC (mDNS included), and
S2's matrix. Left for you, because it needs an administrator:
`powershell -NoProfile -ExecutionPolicy Bypass -File
spikes\machines\setup-walk\windows.ps1` from an **elevated** PowerShell. It
asks before each change and puts each back: the network set Public, the
firewall on with block rules as a cancelled prompt leaves them, then the
setup's fix; Controlled Folder Access on; a signed-out account (an empty
scratch config folder); long paths, Developer Mode and Git Bash read.

### Between the two machines

1. **S1** — done: the LAN, *No relay*, plain TCP, and across two networks.
2. **S2, a real round trip**: `code/RESULTS-S2.md` § "Windows half", step 2
   (Mac `demo` and `dirty`, Windows `haves`, Mac `snapshot`, Windows `apply`
   and `verify`, then back).
3. **S3**: a scratch session made on the Mac, backed up, pulled on Windows
   (Backup › pull), resumed there; ask it for its platform, shell and folder.
   If it still says `darwin`, the move must also end the recorded prompt.
4. Then the decision record in docs/PLAN-MACHINES.md §12 (iroh or the
   fallback).