# Machines — one Mother Claude across your computers

Pair your computers once, then see and drive every computer's Claude Code
sessions from any of them — and from a phone paired with any of them —
move a session, with its code, to another computer and back, and keep a
project's code the same on two computers. The design and
the plan for what comes next (copies kept up to date, work on another
computer) are in [PLAN-MACHINES.md](PLAN-MACHINES.md).

> **Status**: Phases 1 (pairing and setup), 2 (seeing and driving other
> computers' sessions) and 3a (moving a session, with the setup's checks for
> it) are built, and so is code sync (a project's code kept the same on two
> computers). Keeping a copy on another computer comes next.

## Turning it on

Machines is off until you set it up: **Machines › Set up Machines** on the
desktop. The setup runs on each computer — only a computer can check its
own operating system — and once two are paired, each shows every computer's
checklist side by side. Machines turns on only when no *required* check is
left; the checks keep running afterwards as **Machines health** (a red dot
on the sidebar entry when something breaks again).

While Machines is off on a computer, the others can pair with it and run
the setup and its readiness test with it — nothing else: they don't see,
drive or move its sessions ("Machines is off on DESKTOP-PC."), and it
doesn't use theirs. *Turn Machines off* doesn't unpair; *Revoke* does.

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
| Where it lands | **In the project folder** when it has no changes of its own (default) · **Always a separate folder** (a worktree, in Mother Claude's data folder on the computer it goes to) |

They are the same on every computer: a change is offered to the others,
each taking it when someone there accepts. Each project can change them
(*Each project*), with its **recipe**: commands to ask Claude to run after a
move, per system (they go through the session's own approvals), and the
files git ignores that travel with it (such as `.env.local`) — those
usually hold secrets, so adding one needs the dangerous right, and so does
each move that carries them (*Not in git*, below).

### The readiness test

For each other computer: the connection, a 20 MB transfer, git and Claude
Code on both, a **code round trip** — a scratch repository in each
computer's data folder (never a project) snapshotted, sent machine to
machine, landed and compared file by file, both ways — whether each account
here is signed in there, and whether your shared folders exist there and map
the projects both computers have.

## Pairing

On the first computer, **Machines › Pair a machine** shows an 8-character
code (it works once, for five minutes). On the other, **Machines › Join**
and type the code — it works across the internet too. On the same network
the Join sheet lists the computers showing a code under *Nearby*, so you can
check which one you join; you still type its code (joining is proven with
it). Both screens then show the **same six digits**: check they match,
choose on each computer what the other may do on it, and confirm on both.
When the other computer joined over the internet, the first one asks you to
*type* the number the other shows, as phone pairing does.

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
| **Dangerous** | bypass permissions, approve dangerous tool calls, send raw keys, carry a folder's trust and files git ignores with a move | off |

Each is set separately for the **same network** (a direct path on your
local network) and the **internet** (everything else). Change them on the
Machines page; *Revoke* forgets a computer and cuts its connections at once.

## Seeing and driving another computer

Once paired, with Machines on on both, every computer's sessions appear in
**Sessions** with a chip naming their computer (the *Machine* filter picks
one computer); open one
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
dangerous permission on the Mac cannot reach for it through the Mac. A move
you start from here for another computer's session comes back to this one
from there, so carrying a folder's trust or files git ignores then needs
each computer's **Dangerous** permission for the other. Every action another
computer takes is in the audit log of the computer it acted on, with who
acted; a dangerous one also shows a notice there.

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
- **Not in git** — the files git ignores that the project lists to carry
  (*Options › Each project*), such as `.env.local`: plain files inside the
  project that git ignores there, up to 1 MB each and 50 in all, never
  through a symbolic link. They usually hold secrets, so they go only with
  the dangerous right here and the other computer's *Dangerous* permission
  for this one; untick *Carry them* to leave them. A file of the same name
  there is kept, and *Undo move* puts it back.
- **Where it goes** — the project's folder there: found through your folders
  (the same project on each computer), a folder pair, or the same path.
  *Change…* picks another; when it found none, *Choose its folder on
  DESKTOP-PC* lists that computer's projects.
- **Code travels** — *Machine to machine* (the default: straight between
  the two, nothing pushed anywhere), *Through your git remote* (hidden refs
  on your remote, removed afterwards: the remote must answer without a
  password prompt on both computers, checked before anything moves), or
  *Only the conversation*.
- **Code lands** — *In the project folder* (the default, when it has no
  changes of its own there) or *In a separate folder (worktree)*: a worktree
  of the project in Mother Claude's data folder there
  (`machines/worktrees/<session>`); the project folder is then never
  touched, and nothing is ever merged into it.
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
submodules). Machines must be on on both computers. A blocker keeps the button from starting — *Fix the checks
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
  stay where they are, but for the few the project lists to carry (*Not in
  git*, above). Set the project up there with a recipe.
- **Sign-ins**: tokens, keys and `.credentials.json` stay on each computer;
  the account there is the one of the same name, signed in there.
- **The worktree** a move made on the other computer (in Mother Claude's
  data folder there) stays when the session leaves it (its work went with
  the session); remove it there.

### From a phone

A phone paired with any of your computers moves sessions the same way, one
column. Carrying a folder's trust or files git ignores needs the phone's
dangerous permission (set on the desktop); without it the trust option does
not show, and those files stay where they are.

## Keeping code in sync

Working on the same project from two computers? **Code sync** keeps its
code the same on both: what you change on one — an edit, a new file, a
commit, a branch switch — shows up on the other a few seconds later, both
ways. It is off until you turn it on, for one project at a time, with one
other computer.

### Turning it on

1. On the desktop, open **Machines › Set up › Options**, and under **Each
   project** choose the project (*Set a project's own options…*, or
   **Change** on one already listed).
2. Tick **Keep this project in sync with DESKTOP-PC** and press **Save**.
3. DESKTOP-PC shows **Keep app in sync** on its Machines page (and as a
   notice). Someone at DESKTOP-PC presses **Keep it in sync** — or *Not
   now*.

It is on only once both computers said yes. Then the first sync runs at
once: the computer whose checkout has changes, or newer commits, sends them
to the one that is clean and behind. When both have changes of their own,
or each has commits the other lacks, you choose which to keep first
(below).

What it needs, checked when you turn it on:

- **A shared folder** that maps the project to its folder on the other
  computer (*Set up › Accounts and folders*); the folder there must be the
  same project — the same `origin`, or a commit in common. If it is not
  there yet, *Clone it on …* first.
- **The Move permission both ways**: code sync writes into each computer's
  project folder, so each must have given the other *Move* (Machines › the
  computer › Change).
- **Machines on** on both computers.

### What it keeps the same, and what it never touches

What travels is what a move carries: the branch's commits and the staged,
unstaged and untracked changes, exactly as they are. Files git ignores —
`.env` files, `node_modules`, build output — stay on each computer, but for
the ones the project lists to carry (*Files git ignores that travel with
it*, which needs the dangerous right to set) and only when the other
computer gave this one its *Dangerous* permission; the copy such a file
replaces there is kept in Mother Claude's data folder there (the last ten).
Nothing is ever merged, and nothing is pushed anywhere: code goes straight
between the two computers.

### When it syncs

A few quiet seconds after a change — a saved file, a commit, a branch
switch — the computer that changed sends it; a check every minute catches
anything missed. **Sync now** on the Machines page runs one at once.

### The Code sync card

**Machines › Code sync** lists the projects kept in sync and how each
stands:

- **In sync** — when it last synced, and which way.
- **Syncing…** — a step runs.
- **Waiting for DESKTOP-PC** — it was asked; nothing syncs until someone
  there confirms.
- **Paused** — with the reason: the other computer is offline, Machines is
  off on one of them, a Move permission is missing, or a session working in
  the project is moving between the two computers. It goes on by itself
  once that ends — from the checkouts as they are then; nothing is queued.
- **Changed on both** — see below.
- **Off** — turned off on the other computer (or it said *Not now*):
  **Turn on again** asks again; **Remove** drops it here.

**Turn off** stops it at once on this computer and tells the other.

### When both computers changed it

If both computers changed the project since it was last synced, code sync
stops on both — never overwriting either — and the card shows both sides:
how many files each changed, and whether it committed or switched branch.
Choose **Keep this Mac's** or **Keep DESKTOP-PC's** (a second tap
confirms). The side you don't keep is first saved in its own repository,
under `refs/mother-claude/sync/<id>/kept-<time>` — its working tree with the
untracked files, what was staged, and any commits it made — and its project
folder goes back to the last synced state; then the kept side's changes
arrive there as usual. The card lists each saved state. To look at one, in
that computer's project folder:

```sh
git show --stat refs/mother-claude/sync/<id>/kept-<time>    # what it held
git checkout refs/mother-claude/sync/<id>/kept-<time> -- .  # take its files back
```

The same change made on both sides (say, the same `git pull`) is not a
conflict. When setting a side aside could lose something — a file git
ignores where the other side has one, a submodule changed inside, a merge
under way — nothing is changed and the card says why: sort it out there,
then choose again.

### From a phone

A phone sees the Code sync card with each project's state, read only:
turning it on or off, *Sync now* and choosing a side happen on the
desktop.

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

- **The other computer is not in *Nearby*.** *Nearby* only shows which
  computer shows a code: you always type the code, and it works across
  networks too. On a Mac, check Local Network access for Mother Claude.
- **"Machines is off on DESKTOP-PC."** Turn it on there (its Machines page;
  or here, when it names this computer). While Machines is off on a
  computer, the others can only pair and run the setup with it.
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
- **"The remote origin doesn't answer … without a prompt."** Sign in to the
  remote on that computer (Git Credential Manager, `gh auth login`), check
  its address, or move the code machine to machine.
- **"Carrying files git ignores needs …"** They need the dangerous right
  where the session lives and the *Dangerous* permission of the computer
  they go to — or untick *Carry them*, and they stay where they are.
