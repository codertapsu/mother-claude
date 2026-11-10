# Machines — screens to agree on (Phase 0 · S4)

**Draft for review, 2026-11-10.** Paper prototypes for
[docs/PLAN-MACHINES.md](../../docs/PLAN-MACHINES.md) (§5.4, §5.6–§5.10, §5.12,
§5.14, §7, §9). Nothing here is built. Once the points in
[Decisions for you](#decisions-for-you) are settled, Phase 1 builds from this.

The screens reuse what the app already has: the sidebar's groups, the Remote
page's pairing (code, "Works once · expires in", "Number shown on the
device"), the permissions list (`required` / `optional`, *Open System
Settings*, *Re-check*), the session menu's sheets (*Continue on another
account…*, *Switch account…* with "After this turn — it finishes first"),
the data-job card with *Undo* ("Possible until …"), approval cards ("Waiting
at the {computer} ({app})"), toasts with one action, and the pull's words for
conflicts ("Changed on both — kept side by side").

**How to read**

- `[Button]` a button · `[Choice ▾]` a menu · `( )` `(•)` one of · `[ ]` `[x]`
  a tick box · `‹text›` a field.
- `✓` fine · `✕` blocks (required) · `!` works, but worse (recommended) ·
  `…` checking · `?` cannot tell yet · `–` does not apply.
- `●` online · `○` offline · `◌` greyed (its machine is offline).
- Desktop wireframes are about 100 columns wide, phone ones 40.
- Example machines: **MacBook** (this Mac), **DESKTOP-PC** (the Windows
  desktop), **BUILD-BOX** (a third, sometimes offline). Accounts: **Default**,
  **Work**, **Client X**.
- Each screen says where it shows (**Shown on**), what it calls (**Calls**,
  the routes of plan §7) and its **States** besides the one drawn.

**Words (app ↔ plan)** — the UI never says peer, lease, epoch, bundle or
ref.

| In the app | In the plan |
|---|---|
| *Machines* (the page); *this Mac* / *this PC* for itself (the app's `{computer}`); *computer* in sentences | machine, peer |
| *Runs on DESKTOP-PC* · *Lives on DESKTOP-PC* | the lease's home |
| *Continue on DESKTOP-PC* · *Bring back to this Mac* · *Take over* | a move · a move back · a take-over |
| *a copy on DESKTOP-PC* | a copy (§5.9) |
| *Changed on both — both kept side by side* | a conflict copy |
| *Machine to machine* · *Through your git remote* | bundle · hidden refs |
| *In the project folder* · *In a separate folder (worktree)* | checkout · worktree (the app already says "worktree" for Teams) |
| *What DESKTOP-PC may do on this Mac*: See · Drive · Move · Run · Dangerous | grants `view` · `control` · `move` · `run` · `dangerous` |
| *Direct (same network)* · *Direct (internet)* · *Through the relay* | LAN direct · internet direct · relayed |
| *required* · *recommended* | B · W (§5.14) |
| *Re-check* | Check again |

---

## 0. Where it shows

**Shown on**: every desktop; the phone sees Machines under *More* (read
only) and the badges and filter on Sessions.

The sidebar gets **Machines** first in the *Connect* group. A red dot means
Machines health has a problem; nothing shows while Machines is off.

```
 ┌──────────────────────┐
 │ Mother Claude        │
 │                      │
 │ Sessions             │
 │ Inbox             2  │
 │ Agents            1  │
 │ New session          │
 │ Teams                │
 │ YOUR DATA            │
 │ Accounts             │
 │ Backup & sync        │
 │ CONNECT              │
 │ Machines          ●  │   ← red dot: Machines health has a problem (§5)
 │ Remote               │
 │ Browser              │
 │ API                  │
 │ SYSTEM               │
 │ Capacity             │
 │ Settings             │
 └──────────────────────┘
```

The session menu (⋮ on a session's page) gets two entries, beside the
account ones they resemble. They show once Machines is on and a machine is
paired:

```
 Rename…
 Continue here
 Switch account…
 Continue on another account…
 Continue on another computer…        ← new (§7)
 Keep a copy on another computer…     ← new (§7.10)
 Move or copy…
 Delete…
 ──────────
 Terminal view
 Notifications
 Open in VS Code
 Session info
```

For a session that runs on another machine, the same menu shows *Bring back
to this Mac* first, and *Continue on another computer…* lists the other
machines only.

---

## 1. Machines page

### 1.1 Off — the first visit

**Shown on**: the desktop. **Calls**: `GET /api/machines` (`enabled: false`).

```
 Machines
 Use your other computers from here: see and drive their sessions, and move
 a session — with its code — to another computer and back.

 ┌──────────────────────────────────────────────────────────────────────────┐
 │ Machines is off                                                          │
 │                                                                          │
 │ Set it up once. It checks this Mac and every computer you add, fixes     │
 │ what it can, and tests each pair before anything moves. About 10         │
 │ minutes, with each computer at hand.                                     │
 │                                                                          │
 │ You need:                                                                │
 │   • Mother Claude 0.11 or newer on each computer                         │
 │   • each computer switched on, with the internet or the same network    │
 │   • on Windows, an administrator's OK once, for the firewall            │
 │                                                                          │
 │ [Set up Machines]     How it works (Help)                                │
 └──────────────────────────────────────────────────────────────────────────┘
```

**States**

- Setup started and left: the card reads *Setup: step 3 of 7 — 2 required
  left* with *[Go on with the setup]*.
- Phone: "Machines are set up from the desktop app." (the Remote page's
  sentence), and nothing else.

### 1.2 On, with machines

**Shown on**: the desktop (the phone sees it without the buttons).
**Calls**: `GET /api/machines`, `GET /api/machines/setup` (the health card),
`GET /api/machines/settings`, `GET /api/roots`, `GET /api/moves/:id`.

```
 Machines                                           [Pair a machine]  [Join…]

 ┌ Machines health ─────────────────────────────────────────── ✓ All good ┐
 │ Checked 3 min ago on every machine.                         [Re-check] │
 └────────────────────────────────────────────────────────────────────────┘

 ┌ This Mac ──────────────────────────────────────────────────────────────┐
 │ MacBook                                                       [Rename] │
 │ macOS 27.2 · Mother Claude 0.11.0 · Claude Code 2.1.330                │
 │ Key 3F9A C1D2 77E0 9B41                                                │
 └────────────────────────────────────────────────────────────────────────┘

 ┌ DESKTOP-PC ───────────────────────────── ● Online · Direct (same network) ┐
 │ Windows 11 · Mother Claude 0.11.0 · Claude Code 2.1.330 · seen just now   │
 │ 4 sessions, 1 running                              [Open its sessions]    │
 │                                                                           │
 │ On this Mac, DESKTOP-PC may    See ✓  Drive ✓  Move ✓  Run –  Dangerous – │
 │                                                              [Change]     │
 │ On DESKTOP-PC, this Mac may    See ✓  Drive ✓  Move ✓  Run –  Dangerous – │
 │                                      (changed on DESKTOP-PC)              │
 │ Key 8C21 0E5F A9B3 4D67 · paired 10 Nov 2026                  [Revoke]    │
 └───────────────────────────────────────────────────────────────────────────┘

 ┌ BUILD-BOX ─────────────────────────────────────── ○ Offline since 14:32 ┐
 │ Windows 11 · Mother Claude 0.10.9                                        │
 │ ! Older than this Mac's: its sessions show, but nothing moves to or      │
 │   from it until it is updated.                           [How to update] │
 └──────────────────────────────────────────────────────────────────────────┘

 ┌ Folders ──────────────────────────────────────────────────────────────────┐
 │ projects   This Mac     ~/development/projects                            │
 │            DESKTOP-PC   D:\development\projects                           │
 │            BUILD-BOX    C:\src                                            │
 │                                                  [Add a folder]  [Edit]   │
 └───────────────────────────────────────────────────────────────────────────┘

 ┌ Options ──────────────────────────────────────────────────────── [Change] ┐
 │ Relay            Public relays (default)                                  │
 │ Code travels     Machine to machine (default)                             │
 │ Code lands       In the project folder when it has no changes (default)   │
 │ Copies           up to 5 GB on this Mac · 1.2 GB used                     │
 │ Keep awake       while another machine uses this Mac                      │
 └───────────────────────────────────────────────────────────────────────────┘

 ┌ Recent moves ─────────────────────────────────────────────────────────────┐
 │ Fix tray icon   This Mac → DESKTOP-PC   14:05      ✓ Done          [Undo] │
 │ Release notes   DESKTOP-PC → This Mac   yesterday  ✓ Done                 │
 │ Sync folders    This Mac → DESKTOP-PC   yesterday  ✕ DESKTOP-PC went      │
 │                                                      offline; stayed here │
 └───────────────────────────────────────────────────────────────────────────┘

 [Run the setup again]                                   [Turn Machines off]
```

*[Change]* on the grants opens the same switches as the pairing
confirmation (§2.3); a change is made on this Mac only. *[Revoke]* arms
first ("Tap again to revoke", as for devices), then closes the connections
and forgets the key; a second line offers *Revoke on every machine* when
others are paired. *[Open its sessions]* opens Sessions filtered to that
machine.

**States**

- Loading: "Loading…" in each card.
- A machine through the relay: `● Online · Through the relay (slower)`.
- A machine offline: `○ Offline since 14:32`; its card keeps the last
  versions and counts in grey.
- **Its key changed** (reinstalled, or not the same computer):

  ```
  ┌ DESKTOP-PC ────────────────────────────────────────────── ✕ New key ┐
  │ DESKTOP-PC has a new key: it was reinstalled — or it isn't           │
  │ DESKTOP-PC. Nothing goes to it until you pair it again.              │
  │                                           [Pair again]  [Revoke]     │
  └──────────────────────────────────────────────────────────────────────┘
  ```

- Machines turned off: the page shows §1.1 with "Your 2 machines stay paired
  while Machines is off."

---

## 2. Pairing

Pairing works the same on the same network and across the internet. The
other computer is found by the code, nearby or through the relay; the six
digits are compared on both screens; across the internet they are also
typed on the first machine, as phones do today.

### 2.1 Pair a machine — on the first computer

**Shown on**: the desktop that starts it (MacBook here). **Calls**:
`POST /api/machines/pair` (code, expiry), then waits for a join.

```
 ┌ Pair a machine ─────────────────────────────────────────────────── ✕ ┐
 │ On the other computer, open Mother Claude › Machines › Join, and     │
 │ type this code — or pick “MacBook” among the computers nearby.       │
 │                                                                      │
 │      ▄▄▄▄▄▄▄  ▄ ▄▄ ▄▄▄▄▄▄▄          K7QM-4R2X                        │
 │      █ ▄▄▄ █ ▀█▄▀  █ ▄▄▄ █          Works once · expires in 4:52     │
 │      █ ███ █  ▀▄█▄ █ ███ █          [Copy code]                      │
 │      █▄▄▄▄▄█ ▄▀▄▀█ █▄▄▄▄▄█                                           │
 │                                                                      │
 │ Waiting for the other computer…                                      │
 │                                                [New code]  [Cancel]  │
 └──────────────────────────────────────────────────────────────────────┘
```

**States**: "Creating…"; expired — "Expired — create a new code." with
*[New code]*; a computer joined — the confirmation (§2.3) replaces this;
two computers used the code — "This code was used by two computers, so
neither can pair. Make a new code." (the Remote page's rule).

### 2.2 Join — on the other computer

**Shown on**: the desktop that joins (DESKTOP-PC). **Calls**:
`POST /api/machines/join`.

```
 ┌ Join another machine ───────────────────────────────────────────── ✕ ┐
 │ Nearby                                                               │
 │   (•) MacBook     macOS · showing a code                             │
 │                                                                      │
 │ Code               ‹K7QM-4R2X›                                       │
 │ Name of this PC    ‹DESKTOP-PC›                                      │
 │                                                                      │
 │                                                    [Join]  [Cancel]  │
 └──────────────────────────────────────────────────────────────────────┘
```

**States**

- "Looking for computers nearby…"; none — "None nearby. Type the code: it
  works across the internet too."
- "Joining…"
- Wrong code: "That code doesn't match. Check it on the other computer, or
  make a new one there."
- Expired: "That code has expired. Make a new one on MacBook."
- Can't reach: "Can't reach MacBook. The two computers need the internet, or
  the same network. [Run the checks]" (opens the setup's step 1).
- Older app on one side: "MacBook needs Mother Claude 0.11 or newer to pair."

### 2.3 Confirm — on both computers

**Shown on**: both, at the same time. **Calls**: `POST /api/machines/confirm`
(with the grants this machine gives).

On MacBook:

```
 ┌ Pair with DESKTOP-PC? ─────────────────────────────────────────────────┐
 │ DESKTOP-PC · Windows 11 · Direct (same network)                        │
 │                                                                        │
 │ Check that DESKTOP-PC shows the same number:                           │
 │                                                                        │
 │                          4 8 1   2 7 0                                 │
 │                                                                        │
 │ What DESKTOP-PC may do on this Mac         Same network    Internet    │
 │   See sessions, transcripts, requests          [x]            [x]      │
 │   Drive them: send, answer, stop, model        [x]            [x]      │
 │   Move sessions here and away                  [x]            [x]      │
 │   Run commands and tasks here  (later)         [ ]            [ ]      │
 │   Dangerous actions  [risky]                   [ ]            [ ]      │
 │   You can change these later on Machines.                              │
 │                                                                        │
 │                          [The numbers match — Pair]   [Cancel]         │
 └────────────────────────────────────────────────────────────────────────┘
```

On DESKTOP-PC, the mirror: *Pair with MacBook?*, the same number, and *What
MacBook may do on this PC*. Each computer decides only what the other may do
on it.

When DESKTOP-PC joined **over the internet**, MacBook asks for the number
instead of a tap (the phone pairing's rule):

```
 │ DESKTOP-PC joined over the internet. Type the number it shows:          │
 │                         ‹_ _ _  _ _ _›                                  │
 │                                         [Pair]   [Cancel]               │
```

*Dangerous actions* explained under its row when ticked: "Lets DESKTOP-PC
bypass permissions, approve dangerous tool calls and delete conversations on
this Mac."

**States**: "Waiting for DESKTOP-PC to confirm…" (after one side confirmed);
"DESKTOP-PC cancelled."; a wrong typed number — "That's not the number on
DESKTOP-PC. Nothing was paired." and the code is used up; success —
"Paired with DESKTOP-PC." (a toast; inside the setup, the step's list).

### 2.4 Introductions — a third computer

**Shown on**: the computer that just paired BUILD-BOX (MacBook), then on
DESKTOP-PC and BUILD-BOX. **Calls**: `POST /api/machines/pair` with an
introduction, `POST /api/machines/confirm` on each of the two.

```
 ┌ Paired with BUILD-BOX ───────────────────────────────────────────────┐
 │ Also pair BUILD-BOX with your other machines?                        │
 │   [x] DESKTOP-PC   online                                            │
 │ DESKTOP-PC and BUILD-BOX will each show a number to compare.         │
 │                                              [Pair them]   [Skip]    │
 └──────────────────────────────────────────────────────────────────────┘
```

Then on DESKTOP-PC (and the mirror on BUILD-BOX): "MacBook introduces
BUILD-BOX. Pair with it?" with the six digits and the grants, exactly §2.3.
Nothing is trusted because MacBook said so.

---

## 3. Set up Machines (the wizard)

**Shown on**: the desktop, `#/machines/setup`, a full page. It can be left
at any step and resumed; each machine runs its own checks (only a machine can
check its own system), and from step 3 every machine's checks show side by
side. **Calls**: `GET /api/machines/setup` (this machine and, through
`/peer/setup`, the others), `POST /api/machines/setup/fix`,
`POST /api/machines/setup/ask`, `POST /api/machines/test`,
`PUT /api/machines/settings`, `PUT /api/roots`, `POST /api/machines/enable`.

```
 Set up Machines                                  [Leave — your progress is kept]
 ┌────────────────────────┬─────────────────────────────────────────────────────┐
 │ ✓ 1  This Mac          │                                                     │
 │ ● 2  Your machines     │              (the step, below)                      │
 │   3  Every machine     │                                                     │
 │   4  What each may do  │                                                     │
 │   5  Accounts, folders │                                                     │
 │   6  Options           │                                                     │
 │   7  Test              │                                                     │
 │                        │                                                     │
 │ 2 required left        │                                   [Back]  [Next]    │
 └────────────────────────┴─────────────────────────────────────────────────────┘
```

The rail marks a step `✓` when nothing required is left in it, `!` when only
recommended ones are, `✕` when a required one is. *Next* is never disabled —
you may look ahead — but *Turn on Machines* (step 7) is.

### 3.1 Step 1 — This Mac

Required rows block Machines; recommended ones work, but worse, and say why.
Problems first. Shown here from the worst state:

```
 1 · This Mac
 Checked on this Mac. Required ones block Machines until they are fixed;
 recommended ones work without, but worse — each says why.

 NEEDS YOU
 ✕ git                          required     git isn't installed: code can't
                                             move without it.
                                             [Install the command line tools]
                                             [Re-check]
 ? Local Network                recommended  macOS is asking you now: “Mother
                                             Claude would like to find and connect
                                             to devices on your local network.”
                                             Click Allow.              [Re-check]
 ! Firewall                     recommended  Incoming connections to Mother
                                             Claude are blocked, so your computers
                                             meet only through the relay: slower,
                                             and only with the internet.
                                             [Open System Settings]  [Re-check]
                                             In Network › Firewall › Options,
                                             allow Mother Claude.
 ! Start at login               recommended  After a restart, your other
                                             computers can't reach this Mac until
                                             you open Mother Claude.   [Turn on]
 ! Stay awake while used        recommended  This Mac sleeps after 10 minutes: a
                                             move, or a session driven from
                                             another computer, would stop.
                                             [Keep awake while used]
                                             [Open Battery settings]

 FINE
 ✓ Mother Claude 0.11.0 · Machines version 1
 ✓ Relay answers — Public relays, 38 ms
 ✓ Claude Code 2.1.330
 ✓ ~/development/projects can be read and written

                                                       [Re-check all]  [Next]
```

**How a row reads its states**

- `…` "Checking…".
- `?` macOS is showing its own question (Local Network, a folder): the row
  quotes it and says which button to press; *Re-check* after.
- `✕` / `!` with the fix: *[Fix]* when Mother Claude can do it itself
  (start at login, keep awake), *[Open System Settings]* / *[Open
  Settings]* with the exact page and one line saying where to click when
  only you can.
- "Set by your organization" when a profile manages it: "Ask your
  administrator to allow Mother Claude." — still counted.
- **Required with *No relay* only** (Local Network, the firewalls, a Public
  network): with a relay these rows are *recommended* and say: "Without it,
  your computers meet only through the relay: slower, and only with the
  internet." With *No relay* chosen (step 6) they become *required* and the
  rail updates.

### 3.2 Step 2 — Your machines

```
 2 · Your machines
 Pair every computer you want to use. On each one: install or update
 Mother Claude, open Machines › Join, type the code.

 ✓ DESKTOP-PC     Windows 11 · 0.11.0 · Direct (same network)
 ! BUILD-BOX      Windows 11 · 0.10.9 — needs 0.11 or newer: on BUILD-BOX,
                  Settings › Check for updates, or download it:
                  github.com/codertapsu/mother-claude/releases/latest  [Copy link]

                                            [Pair a machine]  [Join…]
```

Pairing opens §2.1 / §2.2 over the step; a third computer gets §2.4.

### 3.3 Step 3 — Every machine, side by side

```
 3 · Every machine
                               This Mac (MacBook)        DESKTOP-PC
 Mother Claude                 ✓ 0.11.0                  ✓ 0.11.0
 Start at login                ✓                         ✓
 Stay awake while used         ✓                         ! sleeps after 30 min
                                                           [Ask DESKTOP-PC to fix]
 Network                       ✓ Local Network allowed   ! the network is Public
                                                           On DESKTOP-PC: Settings ›
                                                           Network › this network ›
                                                           Private.
                                                           [Ask DESKTOP-PC to fix]
 Firewall                      ✓                         ! blocks Mother Claude
                                                           [Ask DESKTOP-PC to fix]
 git                           ✓ 2.54                    ✕ git isn't installed
                                                           On DESKTOP-PC: install
                                                           Git for Windows.
                                                           [Ask DESKTOP-PC to fix]
 Relay                         ✓ Public relays · 38 ms   ✓ Public relays · 41 ms
 Folders                       ✓                         ✓ Controlled folder access off
 Long paths                    –                         ! off
                                                           [Ask DESKTOP-PC to fix]
 Claude Code                   ✓ 2.1.330                 ! 2.1.322 — older
                                                           [Ask DESKTOP-PC to update]
 Git Bash (Claude Code's)      –                         ✕ not found — comes with
                                                           Git for Windows (above)
 Claude Code tools             ✓ on                      ! off  [Ask DESKTOP-PC to fix]

 DESKTOP-PC answered 20 s ago.                                  [Re-check both]
```

Rows that need someone at the other computer read **On DESKTOP-PC:** and
say where to click. *Ask DESKTOP-PC to fix* sends the request (§4); nothing
changes there until someone there confirms. With three machines, a third
column; past four, a machine picker above the table.

**States**

- Asked: "Asked 1 min ago — waiting for someone at DESKTOP-PC."
- Fixed there: "✓ Fixed on DESKTOP-PC at 14:40."
- Offline: the column is greyed, "Offline since 14:32 — these are its
  checks of 13:58."
- An older app there that has no setup checks: the column reads "Update
  DESKTOP-PC to 0.11 to see its checks."

### 3.4 Step 4 — What each machine may do

```
 4 · What each machine may do
 Each machine decides what the others may do on it. This Mac's are changed
 here; DESKTOP-PC's on DESKTOP-PC (you can ask from here).

                                     See   Drive   Move   Run   Dangerous
 On this Mac,    same network        [x]    [x]    [x]    [ ]     [ ]
 DESKTOP-PC may  internet            [x]    [x]    [x]    [ ]     [ ]

 On DESKTOP-PC,  same network         ✓      ✓      ✓      –       –
 this Mac may    internet             ✓      ✓      ✓      –       –
                                             [Ask DESKTOP-PC to change]

 ✓ Sessions can move both ways (Move is on, on both).
 Run: for working on another machine without moving, later; off until then.
 Dangerous [risky]: bypassing permissions, dangerous approvals, deleting.
```

**States**: a missing *Move* in one direction — "! Sessions can't move from
this Mac to DESKTOP-PC: DESKTOP-PC doesn't allow Move." (required only for
that direction); an asked change — "Asked; DESKTOP-PC shows it now."

### 3.5 Step 5 — Accounts and folders

```
 5 · Accounts and folders

 ACCOUNTS — a session moves to the account of the same name
                       This Mac          DESKTOP-PC
 Default               ✓ signed in       ✓ signed in
 Work                  ✓ signed in       ✕ not signed in    [Sign in on DESKTOP-PC…]
 Client X              ✓ signed in       ✕ not there        [Add it on DESKTOP-PC…]

 FOLDERS
 Found 14 projects on both computers, by their git remote:
   projects   ~/development/projects   ↔   D:\development\projects   [Use]  [Edit]
              mother-claude, codex-http-bridge, tray-tool and 11 more
 Only on this Mac
   side-project   github.com/you/side-project      [Clone it on DESKTOP-PC]
   notes          no git remote — can't move with its code
 Only on DESKTOP-PC
   win-tools      github.com/you/win-tools         [Clone it here]

 Your folder pairs from Backup & sync are kept as they are.
```

*Sign in on DESKTOP-PC…* starts the existing sign-in there; its browser
opens on DESKTOP-PC, so the row says "Someone at DESKTOP-PC finishes it in
its browser." *Add it on DESKTOP-PC…* asks DESKTOP-PC to add the account
(confirmed there). *Clone it on …* runs `git clone` into the matched folder
there, as an asked action.

**States**: "Looking for projects on both computers…"; nothing matched — "No
project is on both yet. Pick a folder on each: [Add a folder]"; a clone
running — a data-job line; a clone that needs a sign-in to the remote — "The
remote asked for a sign-in on DESKTOP-PC: sign in to GitHub there, then
[Try again]."

### 3.6 Step 6 — Options

```
 6 · Options — the same on every machine

 Relay — how your computers meet when they can't connect directly
   (•) Public relays (default) — run by the library's maker, n0; they only
       ever see encrypted traffic
   ( ) Your own relay   ‹https://relay.home.lan›   [Test]   ✓ answers · 12 ms
   ( ) No relay — only on the same network, or when they reach each other directly

 How code travels when a session moves
   (•) Machine to machine (default)
   ( ) Through your git remote (GitHub or another) — both computers need access

 Where the code lands
   (•) In the project folder, when it has no changes of its own (default)
   ( ) Always in a separate folder (a worktree), never the project folder

 Copies of other computers' sessions on this Mac      up to ‹5› GB
 [x] Keep this Mac awake while another computer uses it

 Each project can change these (in its settings), and each move too.
 DESKTOP-PC takes a change when someone there accepts it (§4.3).
```

**States**: *[Test]* "Testing…", "✕ No answer from relay.home.lan — check
the address, or a proxy."; *No relay* chosen — "The network rows of steps 1
and 3 are now required."; a setting not yet taken by DESKTOP-PC — "!
DESKTOP-PC still uses Public relays: it was asked at 14:41."

### 3.7 Step 7 — Test, then turn on

```
 7 · Test every pair
                          Connect          Speed        Code         Claude Code   Folders    A session
 This Mac → DESKTOP-PC    ✓ direct · 3 ms  ✓ 112 MB/s   ✓ 1.4 s      ✓ 2 accounts  ✓ 1 of 1   ✓
 DESKTOP-PC → This Mac    ✓ direct · 3 ms  ✓ 98 MB/s    … running    …             …          …

                                                                       [Run again]
 ──────────────────────────────────────────────────────────────────────────────
 [Turn on Machines]   (disabled)
 1 required left — DESKTOP-PC: git isn't installed.               [Go to it]
 3 warnings — DESKTOP-PC's firewall and Public network (relay only) · it
 sleeps after 30 min · Claude Code there is older.
```

The columns are the readiness test of plan §5.14: connect, move 20 MB, a git
round trip on a scratch repo, Claude Code and each account's sign-in, the
folders, a session-shaped transfer into a scratch folder. Nothing touches a
project or a config directory.

**States**: a slow result — "! 0.9 MB/s through the relay: on the same
network? See step 3."; a failure — "✕ Code: git on DESKTOP-PC is missing" with
a link to the row; a machine offline — "Skipped: DESKTOP-PC is offline";
all good — *[Turn on Machines]* enabled; after it — "Machines is on." and the
Machines page (§1.2).

---

## 4. On the other computer: being asked

### 4.1 The request

**Shown on**: DESKTOP-PC, when MacBook asks. **Calls**: arrives through
`POST /peer/setup/ask`; the fix is `POST /api/machines/setup/fix` on
DESKTOP-PC.

A toast, and a card at the top of DESKTOP-PC's Machines page until done or
dismissed:

```
 ┌───────────────────────────────────────────────────────────┐
 │ MacBook asks for a fix                                    │
 │ Windows Firewall blocks Mother Claude.        [Show]  ✕   │
 └───────────────────────────────────────────────────────────┘

 ┌ Asked from MacBook ────────────────────────────────────────────────────┐
 │ ! Windows Firewall blocks Mother Claude                                │
 │   Your other computers reach this PC only through the relay: slower.   │
 │                                              [Fix…]   [Not now]        │
 │ ! This PC sleeps after 30 minutes                                      │
 │   A session driven from MacBook would stop.                            │
 │        [Keep awake while used]   [Open power settings]   [Not now]     │
 └────────────────────────────────────────────────────────────────────────┘
```

Nothing runs because MacBook asked: every fix waits for someone at this PC.
*Not now* tells MacBook "Not now (at DESKTOP-PC)".

### 4.2 The firewall fix, with Windows' administrator prompt

**Shown on**: DESKTOP-PC.

```
 ┌ Allow Mother Claude through Windows Firewall ─────────────────────────────┐
 │ Windows asks for an administrator's OK next (a User Account Control      │
 │ window). Then Mother Claude:                                              │
 │   • adds one rule: allow Mother Claude in, on Private networks only       │
 │   • removes Mother Claude's own block rules (Windows made them when its   │
 │     first “Allow access?” window was closed)                              │
 │ Nothing else changes, and the firewall stays on.                          │
 │                                                                           │
 │ What it runs                                                       [Copy] │
 │   New-NetFirewallRule -DisplayName "Mother Claude" -Direction Inbound     │
 │     -Program "C:\Program Files\Mother Claude\Mother Claude.exe"           │
 │     -Action Allow -Profile Private -Protocol UDP                          │
 │                                                                           │
 │                                                  [Continue]   [Cancel]    │
 └───────────────────────────────────────────────────────────────────────────┘
```

**States**: "Waiting for Windows…" while the UAC window is open; done — "✓
Allowed. MacBook re-checks by itself."; refused — "Windows didn't allow it:
the window was closed, or this Windows account isn't an administrator. [Try
again]"; managed — "Set by your organization: ask your administrator to
allow Mother Claude on Private networks."

The **Public network** fix is not Mother Claude's to do: "This network is
Public, so Windows blocks incoming connections. Make it Private only if you
trust it (home, office). [Open network settings]" — then *Re-check*.

### 4.3 A changed option arrives

**Shown on**: DESKTOP-PC, after an option changed on MacBook. **Calls**:
`POST /peer/settings`, then `PUT /api/machines/settings` when accepted.

```
 ┌ MacBook changed an option ─────────────────────────────────────────────┐
 │ Relay: Your own relay (https://relay.home.lan) instead of Public       │
 │ relays. Use it on this PC too? Until you do, the two may not meet      │
 │ across networks.                                                        │
 │                                    [Use it]   [Keep Public relays]      │
 └─────────────────────────────────────────────────────────────────────────┘
```

---

## 5. Machines health

**Shown on**: every desktop (the phone: the card on *More › Machines*).
**Calls**: `GET /api/machines/setup`; checks rerun at start, every 30
minutes, on a network change, and before every move.

The sidebar's red dot (§0), and the card at the top of Machines:

```
 ┌ Machines health ──────────────────────────────── ✕ 1 problem · 1 warning ┐
 │ ✕ DESKTOP-PC: Work isn't signed in any more     [Sign in on DESKTOP-PC…] │
 │   Moving Work's sessions to DESKTOP-PC waits for it.                     │
 │ ! DESKTOP-PC: the network is Public now          [Ask DESKTOP-PC to fix] │
 │   Your computers meet through the relay: slower.                         │
 │ Checked 2 min ago.                                           [Re-check]  │
 └──────────────────────────────────────────────────────────────────────────┘
```

When one comes back (it was fine before), a toast once:

```
 ┌──────────────────────────────────────────────────────────┐
 │ Machines                                                 │
 │ Work signed out on DESKTOP-PC.             [Show]  ✕     │
 └──────────────────────────────────────────────────────────┘
```

A move's checks (§7.1) name the same problem before anything stops, so a
move never finds it half-way.

**States**: all good — "✓ All good"; a machine offline is not a problem by
itself ("DESKTOP-PC is offline since 14:32" in grey); checking — "Checking…".

---

## 6. Sessions across machines

### 6.1 The list

**Shown on**: every desktop and phone paired with one. **Calls**: the
existing `/ws` and `/api/sessions`, now with `machine` on each session.

A *Machine* filter beside *Surface*, *Project* and *Account*, and a machine
chip on other computers' sessions (this Mac's have none):

```
 Sessions                                    ‹Search sessions›      [New session]
 [Live] [Needs you] [All]   Surface ▾   Project ▾   Account ▾   Machine: All ▾
 ┌──────────────────────────────────────────────────────────────────────────────┐
 │ ● running   Fix tray icon                                         2 min ago  │
 │ Editing src-tauri/src/tray.rs                                                │
 │ [DESKTOP-PC] [CLI · owned] [Work] [Opus] [high]                              │
 ├──────────────────────────────────────────────────────────────────────────────┤
 │ ○ idle      Release notes                                        14 min ago  │
 │ The notes are ready for review.                                              │
 │ [CLI · owned] [Default] [Sonnet]                                             │
 ├──────────────────────────────────────────────────────────────────────────────┤
 │ ◌           Build installer                                         13:58    │
 │ BUILD-BOX is offline since 14:32                                             │
 │ [BUILD-BOX] [VS Code] [Work]                                                 │
 └──────────────────────────────────────────────────────────────────────────────┘
```

*Machine* menu: All machines · This Mac · DESKTOP-PC · BUILD-BOX (offline).
An offline machine's sessions stay, greyed, with when it was last seen; they
open read-only from the last state received.

**States**: a machine connecting — its sessions show "Loading DESKTOP-PC's
sessions…" as one line; *Needs you* counts other machines' approvals too.

### 6.2 A session on another computer

**Shown on**: any desktop or phone that may see DESKTOP-PC. **Calls**:
`ANY /api/machines/:id/proxy/<session routes>`.

```
 ‹ Sessions   Fix tray icon     [DESKTOP-PC] [Work] [Opus]          ● Live   ⋮
 ┌──────────────────────────────────────────────────────────────────────────┐
 │ Runs on DESKTOP-PC — you drive it from here.                             │
 └──────────────────────────────────────────────────────────────────────────┘
   … the transcript, as for a session here …
 ┌ Waiting at DESKTOP-PC (CLI) ──────────────────────────────────────────────┐
 │ Bash   cargo build --release                             [Allow]  [Deny]  │
 └───────────────────────────────────────────────────────────────────────────┘
 ‹Message›                                                              [Send]
```

Approvals read "Waiting at DESKTOP-PC (CLI)" and, once answered elsewhere,
"Answered at DESKTOP-PC" (the existing card's words). A dangerous approval
from a phone needs both the phone's dangerous permission and DESKTOP-PC's
*Dangerous* for this Mac; otherwise "Answer it at DESKTOP-PC or on this Mac"
in place of *Allow*.

**States**: DESKTOP-PC goes offline — "DESKTOP-PC went offline at 14:32.
What you see is from then; sending waits until it's back." and the composer
disabled; *Drive* not allowed — the composer reads "DESKTOP-PC lets this Mac
watch its sessions, not drive them." (the foreign-session sentence's form).

### 6.3 A session that moved away, seen where it was

**Shown on**: the computer it left (MacBook). **Calls**: the lease in the
session summary; actions are §7.

```
 ‹ Sessions   Fix tray icon     [Work] [Opus]                                ⋮
 ┌──────────────────────────────────────────────────────────────────────────┐
 │ Lives on DESKTOP-PC since 14:05 — moved from this Mac.                   │
 │ [Open there]     [Bring back here]     [Take over…]                      │
 └──────────────────────────────────────────────────────────────────────────┘
   … the transcript up to the move, read only …
```

*Open there* opens §6.2. *Take over…* shows only while DESKTOP-PC is
offline. Sending from this page is refused with the same three buttons.

### 6.4 New session on another computer

**Shown on**: the desktop and phone. **Calls**: the existing new-session
route (`POST /api/sessions`) through the proxy, `/api/machines/:id/proxy/…`.

```
 New session
 Where      (•) This Mac   ( ) DESKTOP-PC   ( ) BUILD-BOX  (offline)
 Project    ‹mother-claude ▾›
            on DESKTOP-PC: D:\development\projects\mother-claude
 Account    ‹Work ▾›   ✓ signed in on DESKTOP-PC
 Options    …
 First prompt
            ‹Describe what Claude should do… (optional)›
                                                    [Start on DESKTOP-PC]
```

**States**: the project isn't on DESKTOP-PC — "Not on DESKTOP-PC. [Clone it
there] or pick one of its folders: [Browse DESKTOP-PC's projects]"; the
account isn't there or signed out — the same rows as step 5; a folder there
not trusted — "DESKTOP-PC doesn't trust this folder yet." with the existing
trust question.

---

## 7. Moving a session

### 7.1 Continue on another computer

**Shown on**: the desktop and the phone (§8.2). **Calls**: the checks
through a preflight call (new, see [Routes this draft adds](#routes-this-draft-adds);
nothing is written), then `POST /api/sessions/:id/move`.

```
 ┌ Continue on another computer ─────────────────────────────────────────── ✕ ┐
 │ The session goes on there — the same conversation, with its code. This     │
 │ Mac keeps its copy, and you can keep driving it from here.                 │
 │                                                                            │
 │ Computer                                                                   │
 │   (•) DESKTOP-PC   ● online · direct · Work signed in                      │
 │   ( ) BUILD-BOX    ○ offline since 14:32 — can't now                       │
 │                                                                            │
 │ What goes                                                                  │
 │   Conversation    18 MB, with 3 subagents                                  │
 │   Code            feat/windows-tray · 6 changed files, 2 new               │
 │   Not in git      [ ] .env.local  — carried only if you tick it [dangerous]│
 │                                                                            │
 │ Where it goes      D:\development\projects\app                [Change…]    │
 │ Code travels       [Machine to machine ▾]                                  │
 │ Code lands         [In the project folder ▾]   ✓ it has no changes there   │
 │ After the move     [ ] On DESKTOP-PC, run: npm ci                          │
 │ When               (•) After this turn — it finishes first                 │
 │                    ( ) Now — interrupts the turn                           │
 │ [x] Keep driving it from here                                              │
 │                                                                            │
 │ Checks                                                                     │
 │   ✓ Work is signed in on DESKTOP-PC                                        │
 │   ✓ The folder there is the same repo (same remote)                        │
 │   ✓ DESKTOP-PC trusts that folder                                          │
 │   ! Claude Code there is older (2.1.322, here 2.1.330)                     │
 │   ! Line endings: DESKTOP-PC writes CRLF, so 6 files will look changed      │
 │     there. A .gitattributes in the project avoids it.     [Show the lines] │
 │                                                                            │
 │                               [Continue on DESKTOP-PC]     [Cancel]        │
 └────────────────────────────────────────────────────────────────────────────┘
```

**States**

- "Checking with DESKTOP-PC…" while the checks run; the button waits.
- **The project folder there has changes of its own** — the landing choice
  turns into the question, the button stays usable once answered:

  ```
  │   ✕ D:\development\projects\app has changes of its own (3 files).        │
  │     The code can't land in the project folder.                           │
  │     [Use a separate folder (worktree)]      [See the 3 files]            │
  ```

- **The branch went on there**: "✕ feat/windows-tray on DESKTOP-PC has 2
  commits this Mac doesn't. [Use a separate folder] [Replace it with this
  Mac's — kept for undo]".
- **A required check fails** (an account signed out, no folder, the remote
  asks for a sign-in): the row is `✕` with its fix, and the button reads
  "Fix the checks first".
- **A foreign session** (VS Code, a terminal): the button is
  *[Continue as a copy on DESKTOP-PC]* and the text says: "Fix tray icon runs
  in VS Code — Mother Claude can't stop it. A copy goes on there as a new
  session; this one goes on here. Or end it in VS Code first."
- From the phone, ticking a file not in git (or carrying a folder's trust)
  needs the phone's dangerous permission: the tick box reads "Needs the
  dangerous permission (set on the desktop)".

### 7.2 Progress

**Shown on**: the sheet, then the session page and *Recent moves* (the
data-job card). **Calls**: `GET /api/moves/:id`, `POST /api/moves/:id/cancel`.

```
 ┌ Continue on DESKTOP-PC: progress ─────────────────────────────────────┐
 │ ✓ Checked                                                             │
 │ ✓ The turn ended                                                      │
 │ ✓ Stopped here                                                        │
 │ ✓ Packed — conversation 18 MB, code 2.1 MB                            │
 │ … Sending   12.4 of 20.1 MB   ██████████████░░░░░░░░   about 4 s      │
 │   Placing it on DESKTOP-PC                                            │
 │   Starting it there                                                   │
 │                                       [Cancel]   [Close (it goes on)] │
 └───────────────────────────────────────────────────────────────────────┘
```

While the turn runs: "Moving to DESKTOP-PC once this turn ends. [Cancel the
move]" (the switch-account sheet's form). *Cancel* is possible until
*Placing it on DESKTOP-PC*; after that the way back is *Undo move*.

### 7.3 Done, failed, undone

- **Done** — a toast: "Fix tray icon now runs on DESKTOP-PC. [Open]"; the
  session page turns into §6.2 (or §6.3 without *Keep driving it from here*);
  *Recent moves*: "✓ Done [Undo]", "Possible until 17 Nov." (7 days).
- **Failed before it was placed** — "Couldn't move Fix tray icon: DESKTOP-PC
  went offline while sending. It goes on here, as it was. [Try again]".
- **Placed but not started** — "Fix tray icon is on DESKTOP-PC but didn't
  start there: Work isn't signed in on DESKTOP-PC. [Try again] [Undo
  move]". Trying again sends nothing twice.
- **Undo move** — arms first ("Tap again to undo"), then: the session lives
  here again, DESKTOP-PC's project folder and branch are put back as they
  were, "Undone."

### 7.4 Bring back

**Shown on**: the computer it left (or any, through *Continue on another
computer…*). **Calls**: `POST /api/sessions/:id/move` with `to` = this
machine.

```
 ┌ Bring back to this Mac ──────────────────────────────────────────────── ✕ ┐
 │ What comes back                                                          │
 │   Conversation   what happened on DESKTOP-PC since 14:05 (4 turns)        │
 │   Code           feat/windows-tray: 3 new commits, 1 changed file         │
 │ Nothing here is lost: only what happened there is added.                  │
 │                                                                           │
 │ Checks                                                                    │
 │   ✓ This Mac's project folder has no changes of its own                   │
 │                                                                           │
 │                                    [Bring back here]     [Cancel]         │
 └───────────────────────────────────────────────────────────────────────────┘
```

### 7.5 Take over (the computer it lives on is offline)

**Shown on**: any other computer with the session, while its home is offline.
**Calls**: `POST /api/sessions/:id/move` to this machine, as a take-over (a
new field, see [Routes this draft adds](#routes-this-draft-adds)).

```
 ┌ Take over Fix tray icon? ──────────────────────────────────────────────┐
 │ DESKTOP-PC is offline since 14:32. The session goes on here from this  │
 │ Mac's last copy (14:31, after “Add the tray menu”).                    │
 │ The code is this Mac's project folder as it is: what DESKTOP-PC        │
 │ changed after 14:05 isn't here.                                        │
 │ If DESKTOP-PC went on after 14:31, both versions are kept side by side │
 │ when it's back.                                                         │
 │                                           [Take over]     [Cancel]     │
 └────────────────────────────────────────────────────────────────────────┘
```

Without a copy here: "This Mac has the conversation only up to 14:05, when
it moved. [Take over from 14:05]".

### 7.6 Changed on both — when the old home comes back

**Shown on**: both computers. A toast and a line on both sessions' pages:

```
 ┌──────────────────────────────────────────────────────────────────┐
 │ Fix tray icon changed on both computers                          │
 │ DESKTOP-PC went on with it after you took it over. Both are kept │
 │ side by side: “Fix tray icon” and “Fix tray icon (DESKTOP-PC)”.  │
 │                                               [Open both]   ✕    │
 └──────────────────────────────────────────────────────────────────┘
```

### 7.7 Keep a copy on another computer

**Shown on**: the desktop and phone. **Calls**: `PUT /api/sessions/:id/copies`.

```
 ┌ Keep a copy on another computer ────────────────────────────────────── ✕ ┐
 │ The copy is brought up to date after every turn. Moving the session      │
 │ there is then instant — and works while this Mac sleeps.                 │
 │   [x] DESKTOP-PC   18 MB of its 5 GB for copies                           │
 │   [ ] BUILD-BOX    offline — starts when it's back                        │
 │ The code isn't in the copy: it moves with the session.                    │
 │                                                   [Save]    [Cancel]      │
 └───────────────────────────────────────────────────────────────────────────┘
```

On DESKTOP-PC, while MacBook sleeps, the session page offers "Continue here
from the copy of 14:32" — a take-over (§7.5).

---

## 8. Phone

The same badges, filter, session pages and move sheet, one column. Setup,
pairing, the grants and options stay on the desktop ("Machines are set up
from the desktop app."); a phone sees health and machines read only.

### 8.1 Sessions

```
 ┌──────────────────────────────────────┐
 │ Sessions                          ●  │
 │ ‹Search sessions›                    │
 │ [Live] [Needs you] [All]  Filters ▾  │
 │ ┌──────────────────────────────────┐ │
 │ │ ● Fix tray icon       2 min ago  │ │
 │ │ Editing src-tauri/src/tray.rs    │ │
 │ │ [DESKTOP-PC] [Work] [Opus]       │ │
 │ └──────────────────────────────────┘ │
 │ ┌──────────────────────────────────┐ │
 │ │ ○ Release notes      14 min ago  │ │
 │ │ [Default] [Sonnet]               │ │
 │ └──────────────────────────────────┘ │
 │ ┌──────────────────────────────────┐ │
 │ │ ◌ Build installer         13:58  │ │
 │ │ BUILD-BOX offline since 14:32    │ │
 │ └──────────────────────────────────┘ │
 ├──────────────────────────────────────┤
 │ Sessions   Inbox   Agents   More     │
 └──────────────────────────────────────┘
```

*Filters ▾* opens a sheet with Surface, Project, Account and **Machine**.

### 8.2 Continue on another computer

```
 ┌──────────────────────────────────────┐
 │ Continue on another computer      ✕  │
 │ The session goes on there, with its  │
 │ code. This Mac keeps its copy.       │
 │                                      │
 │ Computer                             │
 │ (•) DESKTOP-PC  ● online             │
 │ ( ) BUILD-BOX   ○ offline            │
 │                                      │
 │ What goes                            │
 │ Conversation 18 MB                   │
 │ Code: 6 changed files, 2 new         │
 │ Not in git: .env.local               │
 │   needs the dangerous permission     │
 │                                      │
 │ Code lands  In the project folder ▾  │
 │ When  (•) After this turn            │
 │       ( ) Now — interrupts it        │
 │ [x] Keep driving it from here        │
 │                                      │
 │ Checks                               │
 │ ✓ Work signed in there               │
 │ ✓ Same repo there                    │
 │ ! Claude Code there is older         │
 │                                      │
 │ [   Continue on DESKTOP-PC   ]       │
 └──────────────────────────────────────┘
```

**States**: as §7.1, one per line; the progress is the §7.2 list, full width.

---

## Routes this draft adds

The screens need a few things plan §7 does not list yet; Phase 1–3a adds
them there:

- `POST /api/sessions/:id/move/preflight` — the move sheet's checks (§7.1,
  §7.4), the target's `/peer/moves/preflight` asked through the source;
  nothing written.
- `takeOver: true` on `POST /api/sessions/:id/move` (§7.5), and a `from`
  copy time when taking over from a copy (§7.7).
- Asked actions besides fixes — *Sign in on DESKTOP-PC…*, *Add it on
  DESKTOP-PC…*, *Clone it on DESKTOP-PC*, *Ask DESKTOP-PC to change* the
  grants (§3.4, §3.5) — as more `check` kinds of `POST
  /api/machines/setup/ask`, each confirmed on that machine.
- `GET /api/machines/nearby` — the computers nearby showing a code (§2.2).

## Decisions for you

1. **Sidebar** — *Machines* first in the *Connect* group, a red dot for
   health. *Recommended.*
2. **The move's name** — *Continue on another computer…* in the session menu
   (button *Continue on DESKTOP-PC*, and *Bring back to this Mac*), beside
   *Continue on another account…*; not "Move to…". *Recommended*: it says what
   you want ("go on there"), and *Move or copy…* already means moving data
   between accounts.
3. **Page name** — *Machines* for the page, "computer" in sentences, as the
   app already mixes them (Folder pairs' "Other machine", "this computer").
   *Recommended*; the alternative is *Computers* everywhere.
4. **Machine chips** — only on other computers' sessions, none on this Mac's.
   *Recommended*: most of the list is local; the *Machine* filter covers the
   rest.
5. **Pairing two desktops** — the code and *Nearby*; the QR code kept small
   and secondary (a desktop rarely has a camera facing the other screen).
   *Recommended*; dropping the QR entirely is fine too.
6. **Keep driving it from here** — on by default in the move sheet.
   *Recommended.*
7. **Moves from a phone** — allowed when the phone is paired with the
   session's computer or one that may *Move*; files not in git and a folder's
   trust need the phone's dangerous permission; setup, pairing, grants and
   options stay desktop-only. *Recommended.*
8. **A busy project folder on the target** — offer *Use a separate folder
   (worktree)* in the sheet as the answer, never wait or merge. *Recommended.*
