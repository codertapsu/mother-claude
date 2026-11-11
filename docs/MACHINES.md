# Machines — one Mother Claude across your computers

Pair your computers once, then see and drive every computer's Claude Code
sessions from any of them — and from a phone paired with any of them. The
design and the plan for what comes next (moving a session with its code,
copies, work on another computer) are in
[PLAN-MACHINES.md](PLAN-MACHINES.md).

> **Status**: Phases 1 (pairing and setup) and 2 (seeing and driving other
> computers' sessions) are built, and the setup's checks for moving sessions
> (accounts, folders, git, Claude Code, the code options). Moving sessions
> comes next.

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
5. **Accounts and folders** — each account matched by name on every
   computer, with its sign-in there (*Sign in on DESKTOP-PC…*, *Add it on
   DESKTOP-PC…*); your shared folders (*roots*: one folder per computer),
   with the ones your projects' git remotes propose; and the projects one
   computer lacks (*Clone it on DESKTOP-PC*, *Clone it here*).
6. **Options** — the relay, how a moving session's code travels and where
   it lands, keeping awake; and each project's own options and recipe.
7. **Test, then turn on** — with each other computer: a connection, a 20 MB
   transfer, git and Claude Code on both, a code round trip both ways, the
   accounts and the folders; then *Turn on Machines*.

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
| Each paired computer | the same protocol and relay setting on both (and, recommended, the same code options) | update it / *Use this setting everywhere* |
| git | a session's code moves through git: installed, 2.30 or newer | install it (*Xcode command line tools*, *Git for Windows*) |
| **Windows:** long paths | Git for Windows leaves out paths over 260 characters unless `core.longpaths` is on (recommended) | *Turn on for git* (for you only); Windows' own setting: an administrator |
| Claude Code | sessions run in it; recommended: not older than the other computers' | install it / *Update Claude Code* (`claude update`, when you press it) |
| Each account | signed in here; a session moves to the account of the same name | *Sign in…* — required for **that account's** sessions only: Machines turns on without it |
| Free space | moves and copies land in Mother Claude's data folder first (recommended: 5 GB) | free some space |

The network checks are *required* only with the **No relay** setting: with a
relay, two computers still meet — through it, slower.

*Through your git remote* needs each project's remote to answer without a
password prompt on both computers. That is checked per project, when you ask
(*Options › Each project › Check the remote*), and again before a move —
never in the background.

### Asked of another computer

A fix, a sign-in, adding an account or cloning a project that needs someone
at another computer is **asked** there: it shows as a notice and in
*Asked by your other computers* on that computer's Machines page, and runs
only when someone there confirms it (*Not now* tells you so). A sign-in
opens that computer's browser; a clone goes only into a folder of one of its
shared folders, from a git remote without credentials, and never prompts for
a password.

### Accounts and folders

- An account is matched by its **name** on each computer (without regard to
  case), the Default to the Default. Another computer sees only your
  accounts' names, kinds and whether they are signed in — never a token, an
  email or a folder.
- A **shared folder** (a *root*) is one folder on each computer:
  `~/development/projects` on the Mac and `D:\development\projects` on
  Windows. A project under it on one is the same project, at the same
  place, on the other. Your folder pairs from *Backup & sync* are kept, as
  roots.
- The setup looks at your projects and the folders right under your shared
  folders, reads each git repository's `origin`, and where the same remote
  is under folders of the same name on both computers, proposes their parent
  folders as a shared folder (*Use*). A project with no remote can't move
  with its code.

### Options for code

| Option | Choices |
|---|---|
| How code travels | **Machine to machine** (default): straight to the other computer, nothing pushed anywhere · **Through your git remote**: hidden refs on your remote, deleted after the move unless you keep them |
| Where it lands | **In the project folder** when it has no changes of its own (default) · **Always a separate folder** (a worktree) |

They are the same on every computer: a change is offered to the others,
each taking it when someone there accepts. Each project can change them
(*Each project*), with its **recipe**: commands to ask Claude to run after a
move, per system (they go through the session's own approvals), and the
files git ignores that travel with it — those usually hold secrets, so
adding one needs the dangerous right.

### The readiness test

For each other computer: the connection, a 20 MB transfer, git and Claude
Code on both, a **code round trip** — a scratch repository in each
computer's data folder (never a project) snapshotted, sent machine to
machine, landed and compared file by file, both ways — whether each account
here is signed in there, and whether your shared folders exist there and map
the projects both computers have.

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
| **Move** | move sessions here and away (coming) | on |
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
