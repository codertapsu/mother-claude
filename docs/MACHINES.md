# Machines — one Mother Claude across your computers

Pair your computers once, then see and drive every computer's Claude Code
sessions from any of them — and from a phone paired with any of them — and
move a session, with its code, to another computer and back. The design and
the plan for what comes next (copies kept up to date, work on another
computer) are in [PLAN-MACHINES.md](PLAN-MACHINES.md).

> **Status**: Phases 1 (pairing and setup), 2 (seeing and driving other
> computers' sessions) and 3a (moving a session) are built. Keeping a copy
> on another computer comes next.

## Turning it on

Machines is off until you set it up: **Machines › Set up Machines** on the
desktop. The setup runs on each computer — only a computer can check its
own operating system — and once two are paired, each shows every computer's
checklist side by side. Machines turns on only when no *required* check is
left; the checks keep running afterwards as **Machines health** (a red dot
on the sidebar entry when something breaks again).

The seven steps:

1. **This computer** — its checks (below), each with what fixes it.
2. **Your machines** — pair the others (next section).
3. **Every machine, side by side** — a fix that needs a person at another
   computer reads *On DESKTOP-PC:*; *Ask DESKTOP-PC to fix* shows it there as
   a notice. Nothing on a computer changes because another asked.
4. **What each machine may do** — the permissions, per computer.
5. **Accounts and folders** — comes with moving sessions.
6. **Options** — the relay and keeping awake.
7. **Test, then turn on** — a connection and a 20 MB transfer between this
   computer and each other one, then *Turn on Machines*.

### What is checked

*Required* blocks Machines; *recommended* works, but worse.

| Check | Why | Fix |
|---|---|---|
| Mother Claude | every computer must speak the same Machines protocol | update it there |
| Stays awake while used | a computer asleep cannot answer | *Keep awake while used*: the app holds a power assertion while another computer is connected — no system setting changes |
| **macOS:** Local Network access | without it macOS refuses Mother Claude your local network, and the Mac reaches the others only through the relay, even at home | allow it when macOS asks, or *System Settings › Privacy & Security › Local Network* |
| **macOS:** the firewall | if it blocks Mother Claude, the others reach this Mac only through the relay | *System Settings › Network › Firewall* |
| **Windows:** the network type | on a *Public* network Windows blocks incoming connections | set your home or office network to *Private* |
| **Windows:** the firewall | without a rule, the others reach this PC only through the relay | *Allow through the firewall*: one rule for Mother Claude, after Windows' administrator prompt |
| The relay | across networks your computers meet through it | check its address, or pick the public relays |
| Paired computers | Machines works between computers you paired | *Pair a machine* |
| Each paired computer | the same protocol and relay setting on both | update it / *Use this setting everywhere* |

The network checks are *required* only with the **No relay** setting: with a
relay, two computers still meet — through it, slower.

## Pairing

On the first computer, **Machines › Pair a machine** shows an 8-character
code (it works once, for five minutes). On the other, **Machines › Join**:
pick it from *Nearby* (same network) or type the code — it works across the
internet too. Both screens then show the **same six digits**: check they
match, choose on each computer what the other may do on it, and confirm on
both. When the other computer joined over the internet, the first one asks
you to *type* the number the other shows, as phone pairing does.

What the code and the digits protect against: the code finds the other
computer, and only someone holding it can start a pairing; the digits are
computed from both computers' keys, so a computer in the middle — even one
that saw the code — would make the two screens show different numbers.
Nothing is stored until both sides confirm.

A paired computer is recognised by its key alone, kept in your Keychain or
Windows Credential Manager. Reinstalling Mother Claude makes a new key: pair
that computer again.

### What another computer may do here

| Permission | Lets it | Default |
|---|---|---|
| **See** | list sessions, read transcripts, see approvals and questions | on |
| **Drive** | send messages, answer ordinary approvals, interrupt, stop, set model, effort and mode | on |
| **Move** | move sessions here and away ([below](#moving-a-session-to-another-computer)) | on |
| **Run** | run commands and tasks here (later) | off |
| **Dangerous** | bypass permissions, approve dangerous tool calls, send raw keys | off |

Each is set separately for the **same network** (a direct path on your
local network) and the **internet** (everything else). Change them on the
Machines page; *Revoke* forgets a computer and cuts its connections at once.

## Seeing and driving another computer

Once paired, every computer's sessions appear in **Sessions** with a chip
naming their computer (the *Machine* filter picks one computer); open one
and use it as if it ran here — the message box, approvals, stop, model,
effort. Its page says *Runs on DESKTOP-PC*. A few things stay on the
computer a session runs on, and its page says so: switching its account,
moving or copying it, deleting it, showing it in Finder or File Explorer,
and opening it in VS Code.

- **Inbox** lists what waits on you everywhere, each request with its
  computer's chip; answer it here and the first answer wins, as with a
  phone.
- **New session** has a *Where* choice: start it on another computer, in
  one of that computer's folders. Claude Code must already trust the folder
  there (open it there with Claude Code once).
- The **Machines** page shows how many sessions each computer has and how
  many run, with *Open its sessions*.

A phone paired with any of your computers sees and drives them all through
it. (A phone whose live connection fails and falls back to refreshing every
few seconds sees only that computer's own sessions for now.)

A dangerous action on another computer needs both its **Dangerous**
permission for this computer *and* your own right here: a phone without the
dangerous permission on the Mac cannot reach for it through the Mac. Every
action another computer takes is in the audit log of the computer it acted
on, with who acted; a dangerous one also shows a notice there.

When a computer goes offline, its sessions stay listed, greyed, until it is
back.

## Moving a session to another computer

A session can go on on another of your computers: the same conversation,
with its code, continuing where it stands. A session lives on one computer
at a time — the one that runs it — and only that one may run it; the
computer it left keeps its copy, read only, and says where it lives now.

### The button

On a session's page, the ⋮ menu has **Continue on another computer…**. On
another computer's session it has **Bring back to this Mac** (*Continue on
this Mac…* for a session that never lived here) and, when a third computer
is paired, *Continue on another computer…* for the others. A move always
starts on the computer the session lives on: from here, Mother Claude asks
that computer to do it.

The sheet asks the other computer first and shows what it found — nothing is
written anywhere until you press **Continue on DESKTOP-PC**:

- **Computer** — the paired computers that are online and let this one
  move sessions to them (an offline one says since when).
- **What goes** — the conversation's size (with its subagents) and the
  code: its branch, how many files changed and how many are new.
- **Where it goes** — the project's folder there: found through your folders
  (the same project on each computer), a folder pair, or the same path.
  *Change…* picks another; when it found none, *Choose its folder on
  DESKTOP-PC* lists that computer's projects.
- **Code travels** — *Machine to machine* (the default: straight between
  the two, nothing pushed anywhere), *Through your git remote* (hidden refs
  on your remote, removed afterwards), or *Only the conversation*.
- **Code lands** — *In the project folder* (the default, when it has no
  changes of its own there) or *In a separate folder (worktree)*, beside it:
  the project folder is then never touched, and nothing is ever merged into
  it.
- **After the move** — when the project has a recipe (a
  `.mother-claude/recipe.json` with commands per system under `afterMove`,
  such as `npm ci`), *ask Claude to run* them there. They are a request in
  Claude's first message there, and Claude runs them through its own
  approvals: Mother Claude itself never runs a command on the other
  computer.
- **When** — while a turn runs: *After this turn* (the default: it finishes
  first) or *Now* (it interrupts the turn; what you had queued goes along).
- **Keep driving it from here** — on by default: when you close the sheet
  after the move, the page becomes the session where it runs now, driven
  from here as any session of that computer.
- **Carry this folder's trust** — when Claude Code on the other computer
  never opened that folder. A trusted folder runs its project hooks and MCP
  servers without asking, so this is a dangerous action: the option shows
  only on the desktop, or on a device allowed dangerous actions (and, for a
  session on another computer, with that computer's *Dangerous* permission
  for this one).

**Checks** lists what is fine, what blocks the move (with its fix: *Sign
Work in on DESKTOP-PC*, *Update Mother Claude on DESKTOP-PC*…) and what only
makes it worse (*Claude Code there is older*, line endings, Git LFS,
submodules). A blocker keeps the button from starting — *Fix the checks
first* — and the sheet offers what it can instead: a project folder there
with changes of its own, or a branch that went on there, offers *Use a
separate folder (worktree)*. A session another app runs (VS Code, a
terminal) cannot be stopped by Mother Claude: it goes on there **as a
copy**, a new session, while this one goes on here (or end it there first).

### While it moves

The sheet then shows its steps with their times: checked, stopped here,
packed, sent (with how much and about how long is left), placed there,
started there. *Cancel the move* works until it is being placed there;
*Close (it goes on)* leaves it running, and the session's page says what it
does meanwhile, its composer waiting. A connection that drops for a moment
while it sends picks up where it stopped.

The first message the session gets there tells Claude it moved: from which
computer and folder to which, that paths in earlier messages are the old
computer's, which shell it has now, and the recipe when you asked for it.
The conversation itself is never rewritten.

### If something fails

- **Before it is placed there**, nothing is lost: the session goes on here
  as it was (and starts again if it ran), and the other computer drops what
  it received. The sheet says why — "Couldn't move Fix tray icon:
  DESKTOP-PC went offline while it was sent. It stays on this Mac, as it
  was." — and *Try again* goes back to the choices.
- **When the code can't land in the project folder there**, the sheet offers
  a separate folder (worktree) and asks again.
- **Placed but not started there** (its account is not signed in there, for
  instance): it lives there now. *Try again* starts it there without sending
  anything twice; *Undo move* takes it back.

### Lives on …

The computer a session left keeps its copy, greyed in the lists with a chip
*Lives on DESKTOP-PC*. Its page says "Lives on DESKTOP-PC since 14:05 —
moved from this Mac.", with:

- **Open there** — the session where it runs.
- **Bring it back here** — a move back to this computer, started on the one
  it lives on (while it is online). Only what happened there is added: this
  computer's copy is the start of the same conversation, and the code lands
  on top of this computer's folder, which still holds what it sent.
- **Take over…** — only while the computer it lives on is offline: the
  session goes on here from this computer's copy, which has the
  conversation only up to when it moved, with this computer's project
  folder as it is. If the other computer went on with it meanwhile, both
  versions are kept side by side when it next moves there.

Messages can't be sent to the copy here; its page says why.

### Undo

**Undo move** — in the sheet when it is done, in *Recent moves* on the
Machines page and on the session's Info tab — is offered for seven days. It
puts the other computer's project folder and branch back as they were,
takes the conversation's files there back out, and the session lives here
again (it starts again here if it ran). It asks for a second tap. An undo is
refused once the session went on there (bring it back instead, so nothing
it did there is dropped), and a copy is never undone: stop or delete it
there.

### What never moves

- **Files git ignores** — `.env` files, `node_modules`, build output: they
  stay where they are. Set the project up there with a recipe.
- **Sign-ins**: tokens, keys and `.credentials.json` stay on each computer;
  the account there is the one of the same name, signed in there.
- **The worktree** a move made on the other computer stays there when the
  session leaves it (its work went with the session); remove it there.

### From a phone

A phone paired with any of your computers moves sessions the same way, one
column. Carrying a folder's trust needs the phone's dangerous permission
(set on the desktop); without it the option does not show.

## How it connects

Your computers talk over QUIC between their keys, end to end encrypted:
directly on the same network, hole-punched across networks when the
networks allow it, otherwise through a **relay** that only ever sees
ciphertext. In *Options*:

- **Public relays** (default) — n0's, anywhere, no setup. Measured in the
  spikes: fine for driving sessions, slower for large transfers.
- **Your own relay** — the address of an `iroh-relay` you run (on a NAS, a
  small server). Your computers then ask no outside service anything.
- **No relay** — the same network only.

The setting must be the same on every computer; changing it on one offers
it to the others, and each applies it when you accept there. Nothing runs on
the network until you use Machines: a computer that never pairs makes no new
connection.

## Troubleshooting

- **The other computer is not in *Nearby*.** Type the code instead (it works
  across networks too). On a Mac, check Local Network access for Mother
  Claude.
- **"No computer showing this code was found."** Check the code, and that
  both computers have the internet or share a network; the code lasts five
  minutes.
- **A computer shows *Through the relay* at home.** On the Mac, Local
  Network access; on Windows, the network type and the firewall check.
- **"This computer has not given yours the … permission."** Change it on
  the other computer's Machines page.
- **"DESKTOP-PC has no account named Work."** Add the account there with the
  same name (Accounts) and sign it in; a session of the Default account
  goes on under that computer's Default.
- **"DESKTOP-PC doesn't trust … yet."** Open that folder once with Claude
  Code there, or carry the folder's trust from the desktop.
- **"… has changes of its own."** Use a separate folder (worktree), or commit
  or stash the changes there first.
