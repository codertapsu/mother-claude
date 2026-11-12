# Machines — one Mother Claude across your computers

Pair your computers once, then see and drive every computer's Claude Code
sessions from any of them — and from a phone paired with any of them —
move a session, with its code, to another computer and back, keep a copy
of a session current on another computer, keep a project's code the same
on two computers, let a session build and test on another computer, hand a
task to a session there, and run a team's role there. The design is in
[PLAN-MACHINES.md](PLAN-MACHINES.md).

> **Status**: every phase of the plan is built: 1 (pairing and setup), 2
> (seeing and driving other computers' sessions), 3a (moving a session,
> with the setup's checks for it), 3b (keeping a copy current, and
> continuing from it), 4 (running commands on another computer, handing it
> a task, a team's role there) and 5 (three computers and more:
> introductions, *Revoke everywhere*; each computer's versions; starting at
> sign-in in the tray), and code sync (a project's code kept the same on
> two computers).

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
   accounts and the folders, and a command each way *Run* is given; then
   *Turn on Machines*.

### What is checked

*Required* blocks Machines; *recommended* works, but worse.

| Check | Why | Fix |
|---|---|---|
| Mother Claude | every computer must speak the same Machines protocol; recommended: not older than another computer's ([Versions](#versions)) | update it there / *Update Mother Claude* (the *Software updates* page) |
| Stays awake while used | a computer asleep cannot answer | *Keep awake while used*: the app holds a power assertion while another computer is connected — no system setting changes |
| Starts when you sign in | after a restart, the others reach this computer only once Mother Claude runs again (recommended) | *Turn on* ([Starting at sign-in](#starting-at-sign-in-in-the-tray)) |
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
| Claude Code tools | sessions here run commands on your other computers through Mother Claude's tools (`machine_run`) — recommended; required, for those tools only, once another computer gave this one *Run* | *Turn on* (*Services › Claude Code tools*) |
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
the projects both computers have. Where *Run* is given, a fixed `echo
machine-run-ok` too: **Run there** (this computer runs it on the other, in a
scratch folder) and **Run here** (the other runs it here).

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
| **Run** | run commands here for its sessions, in a work folder of its own ([below](#running-commands-on-another-computer)) | off |
| **Dangerous** | bypass permissions, approve dangerous tool calls, send raw keys, carry a folder's trust and files git ignores with a move, run commands flagged dangerous | off |

Each is set separately for the **same network** (a direct path on your
local network) and the **internet** (everything else). Change them on the
Machines page; *Revoke* forgets a computer and cuts its connections at once
— on this computer; *Revoke everywhere* does it on all of them
([below](#revoke-everywhere)).

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
it. A phone whose live connection fails and falls back to refreshing every
few seconds (*Limited*) still sees the other computers' sessions: it reads
them, as that computer last heard them, on the same refresh — every few
seconds while a computer is paired, about every half minute while none is.

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
  *Only the conversation*. Machine to machine, code that would be more than
  1 GB goes through your remote on its own when it answers without a prompt
  on both computers (a large first move over a slow link) — and the move
  says so under its steps; otherwise it goes as it is.
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
separate folder (worktree)*. A branch that went on there (commits there
this computer doesn't have) also offers **Replace feat/login on
DESKTOP-PC with this Mac's**: the branch there becomes this computer's —
only the tip the checks saw; if it moves on meanwhile, the move stops —
and its own commits there are kept under
`refs/mother-claude/undo/<time>/branch`, which *Undo move* puts back. The
sheet says it is in force under *Code lands*, where it can be unticked. A
session another app runs (VS Code, a
terminal) cannot be stopped by Mother Claude: it goes on there **as a
copy**, a new session, while this one goes on here (or end it there first).

### While it moves

The sheet then shows its steps with their times: checked, stopped here,
packed, sent (with how much and about how long is left), placed there,
started there. *Cancel the move* works until it is being placed there;
*Close (it goes on)* leaves it running, and the session's page says what it
does meanwhile, its composer waiting. A connection that drops for a moment
while it sends picks up where it stopped.

What is worth knowing about the move is listed under its steps, in *Recent
moves* and (as a count that opens the steps) on the session's line while it
moves: a file git ignores that the other computer had of its own and kept
for *Undo move*, turns it had taken there kept beside the conversation, the
checks' warnings (*Claude Code there is older*…) and the code's (Git LFS,
submodules, names Windows can't hold, executable bits Windows drops).

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
  a separate folder (worktree) and asks again — or, when the branch there
  went on there, replacing it with this computer's.
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

## Keeping a copy on another computer

A session you will likely continue elsewhere can **keep a copy** there:
after each of its turns, what grew is copied to the other computer. Moving
it there is then instant — the conversation does not travel again — and if
this computer is asleep or away, the other one can **continue from the
copy**. It is off until you turn it on.

### Turning it on

- **One session**: its ⋯ menu › **Keep a copy on…** lists your computers,
  each with a switch and how its copy stands. Save.
- **Every session of a project**: Machines › Options › the project's
  options › *Keep a copy of its sessions on*, one switch per computer. A
  session's own choice in its menu wins over its project's; *As its project
  says* goes back to the project's.

The other computer must have given this one its **Move** permission (a copy
is the first half of a move), and Machines must be on on both. Otherwise —
or while the other computer is offline — the copy is **paused** and says
why; nothing is queued, and once that clears, the next copy sends
everything that grew meanwhile.

### When, and what travels

- **After a turn**, never in the middle of one: once the session is idle (or
  only waits for a scheduled wakeup) and has stayed quiet for five seconds.
  Its files are read twice around each copy: if anything moved meanwhile,
  that copy is dropped and goes again once the session is quiet.
- **What grew**: the conversation's new lines, its subagents' and workflows'
  new lines, new file-history snapshots, its todo list and its session
  environment — the files a move would carry. A conversation file only ever
  grows, so only its new bytes travel, once both computers agree that what
  the other holds is the start of the file here (its size and a fingerprint
  of its first and last 64 KB). A file that was cut short or rewritten goes
  again whole, and so does any file whose bytes did not arrive as sent.
- **Never**: the code (it moves with a move, as a snapshot), sign-ins,
  settings, files outside the session's own.

### Where a copy is kept

In Mother Claude's own data folder on the other computer
(`machines/copies/<this computer>/<account>/<session>/`, laid out like
Claude Code's folder), readable by your user only — **never** in Claude
Code's folders: the other computer's Claude Code doesn't see it until you
move or continue the session there. Each computer keeps at most **2 GB** of
another computer's copies (Machines › Options › *Copies of other computers'
sessions*); past that, the copy refreshed longest ago goes first — never the
copy of a session that lives on that computer now.

### What it says

- **Copied at 14:32** — the copy there is as the session was after its last
  turn.
- **Copies once its turn ends** — a turn runs; it goes when it ends.
- **Paused: …** — the other computer can't be reached, Machines is off on
  one of them, its Move permission is missing, its Mother Claude is older,
  its cap made room for newer copies (it goes again after the next turn), or
  the session lives on another computer now.

### Moving a session that has a copy

A move to a computer that keeps a copy brings the copy up to date first,
then sends only the list of the session's files with their SHA-256: the
other computer checks its copy against it, every file, and lands from it
(as a pull lands a session). The move sheet says the copy is there; *Recent
moves* shows **from its copy** and almost nothing sent. If the copy can't be
used, the move sends the conversation as usual.

### Continue here from the copy

While the session's own computer is **offline**, the other one offers
**Continue here from the copy of 14:32**: on the session's page (that
computer's sessions are greyed in the list) and in the *Copies* card. It
takes the session over (it lives here from now on), lands the copy in this
computer's Claude Code folder — under the account of the same name, in the
folder your roots map it to, kept for undo like a pull — and starts it as a
session Mother Claude runs, with the model, effort and mode it had. Its
first message tells Claude where it was and where it is now. The code is
this computer's project folder as it is.

If the first computer went on with the session while it was away, nothing
is lost: when the session next moves there, that computer's own turns are
kept beside it as a conflict copy, never merged.

### The Copies card

Machines › **Copies** lists the copies **held on this computer** (the
session, its computer, its size, when it was last copied — *Continue here*
while its computer is offline, *Remove*) and the ones **kept on other
computers** (how each stands — *Stop keeping it on …*, which also removes
it there). A copy removed here comes back after the session's next turn
until its own computer stops keeping it.

### From a phone

A phone paired with the session's computer chooses where it keeps copies
from the session's menu, and sees and removes copies on the Machines page.
The cap is set on the desktop.

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

## Running commands on another computer

A Claude Code session on one computer can build and test on another: a
session on the Mac runs `cargo test` on the Windows desktop, at the code it
has right now, and reads what it printed — without moving, and without you
switching computers. The computer that runs the commands decides whether
it may at all; you approve each run on the computer that asks.

### Turning it on

1. On the computer that will run the commands (DESKTOP-PC), open
   **Machines**; on the card of the computer whose sessions will ask, press
   **Change** next to *On this PC, … may*, tick **Run** — for the same
   network, the internet or both — and press **Save**. It is off for every
   computer until you tick it, and it needs *Drive* (which needs *See*).
2. On the computer whose sessions ask (the Mac), keep **Services › Claude
   Code tools** on (it is by default): sessions reach the other computer
   through Mother Claude's tools. The setup checks it — *Claude Code tools*,
   with *Turn on* — and once another computer gave this one *Run*, it is
   required for those tools (Machines itself works without it).
3. Machines on, on both.

The readiness test then runs a fixed `echo` each way *Run* is given
(*Run there*, *Run here*).

### How a session uses it

Every session sees five tools, as `mcp__plugin_mother-claude_mc__machine_…`
(the reference is [MCP.md](MCP.md#machines)):

| Tool | Does |
|---|---|
| `machine_list` | your other computers: system, shell, online, and whether this computer may run commands there (if not, why) |
| `machine_run` | runs one command line on one of them, at the session's code, and waits up to 10 minutes for it |
| `machine_wait` | waits again for a run that is still running |
| `machine_output` | reads a run's whole output, page by page — or brings a file it made into the project |
| `machine_cancel` | stops a run, with everything it started |

Ask in your own words — *run the tests on DESKTOP-PC* — and Claude looks
the computer up (it runs PowerShell there), then calls `machine_run`:

```json
{ "machine": "DESKTOP-PC", "command": "cargo test --lib", "cwd": "src-tauri" }
```

and reads back where it ran, how it ended and what it printed:

```text
run_5c0f9e2a7d41b3e8 on DESKTOP-PC (PowerShell 7), in src-tauri of a work folder at this session's code (C:\Users\me\AppData\…\machines\work\wk_8d2e…).
It ended with exit code 0 after 2 min 14 s.
Output (38.2 KiB):
…
test result: ok. 412 passed; 0 failed; 0 ignored
```

- **`cwd`** is a folder inside the project, relative to its top; without
  it, the command runs in the folder the session works in.
- **`timeout`** is in seconds: 30 minutes by default, 2 hours at most.
- **`artifacts`** names files the run makes, such as
  `target/release/*.exe` (below).

The session must work in a git repository: its code goes to the other
computer as a git snapshot. In Claude Code's default permission mode the
session first asks to use the tool, as for any MCP tool; that only lets it
ask — each run still needs the approval below. A session in
`bypassPermissions` needs it too.

### Who agrees

- **The computer that runs it**, through its *Run* permission for the one
  that asks, on the network in use. Without it nothing is asked or sent:
  Claude is told that DESKTOP-PC has not given this computer the Run
  permission, and where to turn it on. With it, each run is approved on the
  computer that asks, never on the one that runs it: give *Run* only to a
  computer you would let type into a terminal on the other.
- **You, for each run, on the computer that asks.** The run waits for an
  approval — in the Inbox, on the session's page and on your phones —
  showing the **Computer**, the **Folder** (that computer's checkout of the
  project, and the folder in it), the **Command**, the **Shell**, the
  **Time limit** and the **Files to list**. *Allow once* runs it; *Deny*
  tells Claude not to try again (*Deny with feedback…* says what to do
  instead); unanswered for 10 minutes, it does not run.
- **Or a standing grant.** *Always…* on that card offers *Always allow
  `cargo test` on DESKTOP-PC for this project*: from then on, a command of
  this project that starts with those words runs on DESKTOP-PC without
  asking. It is saved in Mother Claude — not in Claude Code's settings —
  and listed under *Always allowed runs* on the Machines page, where
  *Revoke* (desktop) takes it back. Attaching one needs the dangerous
  right (the desktop, or a device allowed dangerous actions), as every
  standing grant does; without it the card offers *Allow once* only.

A standing grant is narrow on purpose:

- **Whole words.** The command's first words must be the grant's words,
  exactly: `cargo test` covers `cargo test --lib`, never `cargo test-foo`
  or `cargo testing`.
- **One command.** A command holding `;`, `&`, `|`, a backtick, `$`,
  parentheses, braces, `<`, `>` or a line break always asks, whatever it
  starts with: no grant lets a second command ride along.
- **Never a dangerous command** (next).
- **What it offers** is the program and what it runs: `cargo test`, `npm
  run build`, `python -m pytest`, `./gradlew build`. Nothing is offered for
  a shell or a program that runs whatever it is given (`bash`, `pwsh`,
  `cmd`, `sudo`, `env`, `xargs`, …), nor for an interpreter alone
  (`python`, `node`).

On the computer that runs the commands, taking back the other's *Run* (in
the column its run came in by: the same network, or the internet),
revoking that computer or turning Machines off stops what it runs for that
computer at once (turning Machines off: for every computer).

### Dangerous commands

A command that names what Mother Claude guards — Claude Code's
configuration (`.claude/`, `.claude.json`, `.mcp.json`, an account's config
folder), Mother Claude's own data, `~/.ssh`, a shell startup file, git
hooks or `.git/config`, launch agents, `claude plugin|mcp|config`, or a
permission bypass — is a **dangerous** run:

- its approval is a dangerous one: only the desktop, or a device allowed
  dangerous actions, can allow it, and it never offers *Always…*;
- the computer that runs it must also have given the one that asks its
  **Dangerous** permission. It checks the command with its own rules as
  well, before you are asked: a command only its rules flag (a Windows path
  into `.claude\` that the Mac's rules don't read as one, say) is a
  dangerous run all the same, and checked again when it starts there.

### Where it runs: a work folder

A run never happens in your project folder on the other computer. Each
project gets a **work folder** there, one per computer that asks, kept
between runs: `machines/work/wk_…` in Mother Claude's data folder there, a
git worktree of that computer's own checkout of the project — the folder
your shared folders map the project to (else a folder pair, else the same
path).

Before each run:

1. the computer that asks snapshots the session's working tree — commits,
   staged, unstaged and untracked files, not the files git ignores — and
   sends only what the other one lacks;
2. the other computer resets the work folder to that snapshot: every file
   as the snapshot holds it, untracked files it doesn't hold removed;
3. the files git ignores stay — `target/`, `node_modules/`, build caches —
   so a build picks up where the last one stopped.

Your checkout there is never touched: its files, index, branch and `HEAD`
stay as they are. Its `.git` gains only what a worktree and a fetch add:
the worktree's entry, the snapshots' objects and refs under
`refs/mother-claude/moves/wk_…/` (the last three are kept); the session's
own repository keeps its last three snapshots under the same refs.
Submodules are not checked out in the work folder.

No checkout there, and the run is refused: clone the project there first
(*Set up › Accounts and folders*, *Clone it on DESKTOP-PC*), or add the
shared folder it is under.

### How it runs

- **The shell.** On a Mac, `zsh` as a login shell (`/bin/zsh -lc`: what
  your `.zprofile` puts on the `PATH` is there; `.zshrc` is not read). On
  Windows, PowerShell 7 (`pwsh`) when it is installed, else Windows
  PowerShell 5.1, without your profile (`-NoProfile -NonInteractive`); the
  run's exit code is the program's own (a failed cmdlet: 1). On Linux,
  `bash -lc`. `machine_list` names each computer's shell, so Claude writes
  the command for it.
- **The environment** is Mother Claude's own, without what would let a
  command act as you elsewhere: the asking session's identity, account
  tokens, API keys and provider switches are removed, as for the sessions
  Mother Claude starts. `MOTHER_CLAUDE_RUN=1` tells a script it runs this
  way, and `GIT_TERMINAL_PROMPT=0` keeps git from waiting for a password
  nobody sees. Nothing can be typed into it: a command that asks a
  question fails instead of waiting.
- **One at a time.** A second run in the same work folder waits for the
  first; its time limit counts meanwhile (it starts when the run reaches
  that computer).
- **The end.** When the time limit runs out, or on a cancel, the command
  ends with everything it started: on a Mac or Linux its own process group
  (asked to stop, killed 3 seconds later), on Windows its job object. When a
  command ends by itself, whatever it left running in the background is
  stopped too, so nothing keeps the work folder's files open.

### What comes back

- `machine_run` waits up to 10 minutes. A run that ends by then answers at
  once: the computer, shell and work folder; how it ended — its exit code
  and how long it took, cancelled, out of time, or why it could not run;
  the output's last 64 KiB; and the files it made.
- A longer run answers *still running*, with its id and its latest output;
  Claude waits again with `machine_wait`, or stops it with
  `machine_cancel`.
- **The whole output** — both streams as they came, up to 256 MiB — stays a
  day on the computer that ran it: `machine_output` reads it page by page
  (64 KiB at a time unless asked for up to 256 KiB), from the start or from
  the end.
- **Files it made.** `artifacts` is a glob relative to the project's top
  (one without a `/` matches at any depth). When the command ends by
  itself, up to 100 matching files are listed with their sizes;
  `machine_output` with `"artifact": "target/release/app.exe"` brings one,
  up to 64 MiB, into the session's project under
  `.mother-claude/artifacts/<run>/`, where a `.gitignore` keeps git from
  seeing it. A file can be fetched until the next run resets the work
  folder.

On the session's page, a run is a background task of the session that
asked (**Agents** › **Background tasks**), labelled with its command and
computer: **Output** shows the end of what it printed (read again every few
seconds while it runs), **Cancel** stops it there.

### The Runs card

**Machines › Runs** lists the recent runs both ways — the ones this
computer's sessions ran elsewhere and the ones other computers ran here —
with the computer and which way, the project, the command, how it stands
(*Running*, *Done*, *Failed*, *Ran out of time*, …), its exit code, time
and output size. *Output* opens what it printed last, with the files it
made; *Cancel* stops a run that has not ended, from either computer. Each
computer keeps its runs a day after they end.

Under it, **Always allowed runs** lists the standing grants: each one's
command prefix, computer and project, who added it and how often it was
used. *Revoke* (desktop) takes one back; the next such run asks again.

### From a phone

A phone paired with the computer that asks answers a run's approval like
any other (Inbox, the session's page), follows runs and cancels them.
*Always…* and dangerous runs need the phone's dangerous permission (set on
the desktop). It sees *Always allowed runs* read only: *Revoke* is the
desktop's.

## Handing a task to another computer

A session can also hand a whole task to a Claude Code session on another
computer: the Mac's session asks the Windows desktop to *make the installer
build on Windows*; a session there does it in its own copy of the project,
at the Mac session's code; and what it did comes back to the Mac as a
branch, with its last message as the summary. The session there is an
ordinary session of that computer: you see it, drive it and answer its
approvals from either computer.

### Turning it on

As for runs: the computer that will take tasks gives the one that asks its
**Run** permission (a session that works there runs code there), and
*Claude Code tools* stays on on the computer that asks. Both need this
version of Mother Claude: an older one answers *Update Mother Claude on
DESKTOP-PC: it does not take tasks from other computers yet.*
`machine_list` says for each computer whether this one may hand it tasks
(`mayTask`).

### How a session uses it

`machine_task` (the reference is [MCP.md](MCP.md#machines)):

```json
{ "machine": "DESKTOP-PC", "task": "Make the installer build on Windows. …" }
```

- **`task`** is the new session's whole prompt (64 KiB at most). That
  session knows nothing of the conversation that hands it over, so Claude
  writes in it what it needs to know.
- **`permissionMode`** is the session's mode there: `default` unless asked
  — it asks before it edits files or runs commands, and you answer from
  either computer. `acceptEdits`, `plan` and `auto` may be asked as well;
  `bypassPermissions` and `dontAsk` need the dangerous right on both
  computers (below).
- **`model`** and **`effort`** choose its model and reasoning effort there
  (that computer's defaults otherwise).

`machine_task` waits up to 10 minutes for the session's turn to end:

```text
task_3f1c0a9e2d7b4c11 on DESKTOP-PC: "Make the installer build on Windows." — in its own worktree of D:\dev\app there, at this session's code.
It is done: its session's turn ended after 6 min 2 s.
Its work is the branch mother-claude/DESKTOP-PC/task_3f1c0a9e2d7b4c11 in this repository: 2 commits and changes it left uncommitted, 4 files changed. It is never checked out or merged here; look with
  git log --oneline 8d2e41c0b7a9..mother-claude/DESKTOP-PC/task_3f1c0a9e2d7b4c11
  git diff 8d2e41c0b7a9...mother-claude/DESKTOP-PC/task_3f1c0a9e2d7b4c11
(8d2e41c0b7a9 is this session's code as it was handed over, uncommitted changes included.)
Its summary (its session's last message):
…
```

A longer task answers *still running* with its id (`task_…`):
`machine_wait` waits again and `machine_cancel` stops it — both take a
task's id as well as a run's.

### Who agrees

- **The computer that takes it**, through its *Run* permission for the one
  that asks, on the network in use. Without it nothing is asked or sent.
- **You, for every task, on the computer that asks.** The approval (Inbox,
  the session's page, your phones) shows the **Computer**, the **Folder**
  (that computer's checkout of the project: the task gets its own worktree
  of it), the whole **Task**, and the session's **Permission mode**, model,
  effort and **Account** there. There is no *Always…*: a task is a
  free-form prompt, so each one is approved.
- **A session without approvals** (`bypassPermissions`, `dontAsk`) is a
  dangerous approval — the desktop, or a device allowed dangerous actions —
  and the computer that takes it must also have given the one that asks its
  **Dangerous** permission; without it Claude is told so before you are
  asked.

On the computer that runs a task, taking back *Run* (in the column the task
came in by), revoking the computer that asked or turning Machines off
cancels the task while its session works; a task that ended is left as it
is.

### Where it works: its own worktree

The computer that asks snapshots the session's working tree (commits,
staged, unstaged and untracked files, not ignored ones) and sends what the
other lacks, as for a run. There the task gets **its own worktree**,
`machines/work/tasks/task_…/<project>` in Mother Claude's data folder: a
git worktree of that computer's checkout of the project, detached at the
snapshot, so the asking session's uncommitted changes are the worktree's
first commit. Your checkout there is never touched (its files, index,
branch and `HEAD`), and neither is the run work folder.

The session starts there as one Mother Claude runs (an *owned* session),
with the task as its first message after a short note: that the task was
handed over from the Mac, and that it finishes by committing its work or by
leaving its changes in the working tree. It runs under the account with the
same name as the asking session's account, else the Default, which must be
signed in there. Both computers' lists call it *From MacBook: <the task's
first line>*.

### When it ends, and what comes back

A task is **done** when its session's turn ends with no approval waiting
and no message queued. Then:

1. that computer snapshots the worktree — the session's commits and what it
   left uncommitted — and sends a bundle of what this computer lacks;
2. this computer makes it the branch **`mother-claude/<computer>/<task>`**
   in the asking session's repository. It is never checked out and never
   merged: review it with `git log` and `git diff` (the answer names the
   commit it starts from), merge or cherry-pick what you want, or delete
   it;
3. the session's last message is the task's summary.

A task that changed nothing makes no branch, and says so.

The session stays there after its turn: send it another message, from
either computer, and the task **runs again**; when that turn ends, the
branch **moves forward** with what it did since — never rewritten: each
result builds on the last one. A branch you committed onto here, or one a
worktree here has checked out, is not moved: the task names the newer
result's commit instead (`git branch <name> <commit>` makes a branch of
it).

A done task's session that sits idle with nothing to do — no approval
waiting, no message queued, no background work — for 30 minutes (the idle
stop team roles get) is **stopped** there, so a finished task does not keep
a `claude` running; the task stays done and its worktree stays. *Its
session stopped after sitting idle* shows on its card, and a message to it
from either computer **resumes it** in its worktree (`--resume`, the same
conversation), so the task runs again and the branch moves forward as
above. A session that runs without approvals is resumed only by someone
with the dangerous right.

### Following tasks

**Machines › Runs › Tasks** lists the tasks both ways — those this
computer's sessions handed elsewhere (*To DESKTOP-PC*) and those other
computers handed this one (*From MacBook*) — with the project, the task's
first line, how it stands (*Working*, *Waiting for an answer*, *Bringing
its work back*, *Done*, *Stopped*, *Cancelled*, …), the branch and what it
holds, and:

- **Open its session** — its page on the computer it runs on (another
  computer's session page for a task handed over: drive it and answer its
  approvals from here);
- **Summary** — its session's last message;
- **Cancel** — while it works: its session is stopped (what it did so far
  still comes back);
- **Remove** — once it ended (tap twice): its session and worktree there
  go, and its records on both computers; **its branch stays**.

A task is kept 7 days after it last ended, with its worktree, then removed
by itself. On the asking session's page it is a background task (**Agents**
› **Background tasks**): **Output** shows how it stands, its branch and
summary; **Cancel** stops it.

Both computers write each task in the audit log with its id, the computer,
the folder and the SHA-256 of the task's first line — never the task's
text.

### From a phone

A phone paired with the computer that asks answers a task's approval like
any other, follows tasks, opens their sessions, cancels and removes them. A
task whose session runs without approvals needs the phone's dangerous
permission.

## A team role on another computer

A team ([TEAMS.md](TEAMS.md)) can run one of its roles on another computer:
the lead plans on the Mac, an `ios` role works on the Mac, and a `windows`
role works on the desktop PC — each on the same board, the Windows role's
work coming back to the Mac as a branch.

### Turning it on

- The computer that will run the role gives the lead's computer its
  **Run** permission (as for runs and tasks), and keeps *Claude Code tools*
  on (Settings › Services): the role reaches the board through them. Both
  need this version of Mother Claude: an older one answers *Update Mother
  Claude on DESKTOP-PC: it does not run team roles yet.*
- The project is a git repository, cloned on both computers (a shared
  folder maps it there, as for runs).
- In the template, the role names the computer: `"machine": "DESKTOP-PC"`,
  or `"os:windows"` for the first online Windows computer that lets this
  one run (the role stays on it afterwards). A name no paired computer has
  is a warning on the template, and the team does not start with it. When
  you start a team, each role's **Computer** can also be chosen there: the
  template's, this computer, a paired one, or any of an OS. The lead always
  runs on the computer that starts the team.

### What happens

- **It starts there**, when its work arrives (or with the team, when it
  starts with it): the lead's computer sends the project's code as it is
  (uncommitted and untracked files included), and the other computer
  decides whether it may start now — with its own limits on role sessions,
  its memory and its plan's 5-hour window. It then starts an ordinary owned
  Claude Code session there, named `<project>/<role>`, in a worktree of its
  own (`machines/work/roles/rb_…/<project>` in Mother Claude's data folder;
  your checkout there is never touched), with the role's charter, settings,
  team tools and file rules, under the account of the same name as the
  team's, else the Default.
- **It works on the lead's board.** Its `task_*` tools there act on the
  board on the lead's computer, which accepts them only from that computer,
  for that role (a binding the lead made for it). The lead's packets and the
  board's notices reach it there. It cannot `SendMessage` the lead (that
  stays on one computer): it writes on its task instead.
- **Its work comes back as a branch.** When it finishes a task, what it
  committed and left uncommitted becomes the branch
  **`mother-claude/DESKTOP-PC/<team>/<role>`** in the lead's project —
  never checked out or merged, moved only forward — and the task names it.
  The lead reviews and merges it. The next task starts from the lead's
  newer code once the lead merged that branch; until then the role keeps
  its own work in its worktree.
- **The lead stops and resumes it**: *Stop* on the team page, its idle stop
  and stopping the team stop its session there; its next work resumes it
  (the same conversation, in the same worktree). Removing the team removes
  its worktree there.
- **It counts where it runs.** The other computer's Capacity page lists its
  session *for MacBook*; the lead's computer does not count it. The team
  page shows the role *on DESKTOP-PC*, why it waits (*on DESKTOP-PC: …*) or
  why it could not start, its worktree there and its branch.

### On the computer that runs it

**Machines › Runs › Team roles** lists the roles this computer runs for
another's lead — the team, whose lead, how its session stands, its
results — with **Open its session**, **Stop** (the lead starts it again
when it has work) and **Remove** (its session, worktree and record here).
Turning Machines off here, revoking the lead's computer or taking back its
**Run** stops them; so does a lead not heard from for 30 minutes while the
session sits idle.

Both computers audit starts, stops, results and board calls with the team,
the role and the computer — never a task's text.

## Three computers or more

Trust is **pairwise**: each pair of your computers is paired on its own,
with its own six digits and its own permissions each way. Nothing is
trusted transitively — being paired with the Mac gives a computer nothing
on the computers the Mac is paired with.

### Introductions

Pairing a third computer with all the others would mean a code and six
digits per pair. An **introduction** spares the codes, never the digits:

1. Pair the new computer (C) with one of yours (A), by code.
2. A offers **Also pair C with B?** for each of your other computers that
   isn't paired with C — right after pairing (in the pair and join sheets),
   in the setup's *Your machines* step, and on the Machines page (*Pair
   your computers with each other*). A knows whom each is paired with
   because each computer's hello lists the ids of the computers it is paired
   with (ids only), to a computer it lets see its sessions.
3. **Introduce them** tells B who C is and C who B is — their names, ids,
   public keys (their endpoint ids) and last addresses, over A's own
   end-to-end encrypted links to each. Never a secret, never a permission.
   Nothing is paired yet.
4. B and C each show the introduction: a notice, and the *Introductions*
   card on the Machines page (*A introduces C: pair this computer with
   it?*). **Pair with C…** on each starts this side of the pairing; **Not
   now** declines it there and tells the other, and nothing is paired.
5. C dials B **by the key A named** (the connection proves it), and the two
   run the normal pairing exchange directly: the commit-then-reveal over
   both keys, so both screens show the **same six digits**; when C came
   over the internet, B asks you to type the digits C shows. Each desktop
   chooses what the other may do on it and confirms — only then does each
   store the other.

B admits C's key, for this introduction only, while it is open (ten
minutes): to the pairing routes, nothing else — a computer that comes for
it with any other key is refused (an unknown key at the door, another paired
computer at the join), and one that names an introduction B never got is
told it is gone. An introduction from a
computer that isn't paired never arrives; one naming the computer it is
sent to, or its own sender, is refused. A computer too old to be introduced
says so ("Update Mother Claude on … first").

### Revoke everywhere

For a lost or stolen computer: on **any** of your computers, its card's
**Revoke everywhere** (two taps). That computer is revoked there at once —
its key forgotten, its connections cut — and every other paired computer
is asked to do the same. Each applies it at once (it only ever removes
trust), audits it, and shows a notice: *MacBook revoked Lost laptop on all
your computers.*

A computer that is off or away takes it when it is next heard (its hello,
either way): the request is kept in `machines.json` until every computer
took it, and *Revoked everywhere* on the Machines page says which ones still
have to — offline, or too old to take it until it updates. The request names
a third computer only: never the computer asked, nor the one asking, and it
never adds anything — a computer the asked one doesn't know stays unknown.

### Versions

Each computer's card shows its system, Mother Claude, Machines protocol and
Claude Code versions. When they differ:

- **Another Machines protocol** — required: the two don't work together
  until one updates (the setup's check, and *Older than this computer's* on
  the card).
- **An older Mother Claude** — recommended: the card says so with **Ask
  DESKTOP-PC to update**, which shows that computer's *Software updates*
  page once someone there confirms it (an asked action: nothing installs on
  its own). When this computer is the older one, **Update it** opens
  *Software updates* here, and the setup's *Mother Claude* check says the
  same.
- **What it can't do until it updates** — the capabilities its hello lacks,
  named: *DESKTOP-PC can't take tasks and keep code in sync until it
  updates.* A feature is used only when both computers list it.
- **An older Claude Code** — recommended (a move to it already warns):
  **Ask DESKTOP-PC to update** asks it to run `claude update` there; here,
  **Update it** runs it.

## Starting at sign-in, in the tray

A computer nobody opens Mother Claude on after a restart — a build box, a
desktop at home — can start it by itself: **Settings › Startup › Start
Mother Claude when you sign in, in the tray** (the desktop app; off by
default). At the next sign-in Mother Claude starts with **no window**, only
its icon in the tray (the menu bar on a Mac), and serves your phone and
your other computers as usual. *Open Mother Claude* in the icon's menu (on
Windows, a click on the icon) shows the window; closing the window hides it
again; *Quit Mother Claude* ends it.

The switch writes the system's own login item and reads it back: on
Windows the per-user `Run` key (`HKCU\Software\Microsoft\Windows\CurrentVersion\Run`,
value `Mother Claude`: the app, quoted, with `--tray`) — when *Task Manager ›
Startup apps* switched it off, *Settings* says so and switching it on here
starts it again; on a Mac a LaunchAgent,
`~/Library/LaunchAgents/dev.motherclaude.desktop.plist`, that starts the app
with `--tray` (macOS lists it under *Login Items › Allow in the
Background*). Nothing is written but on your switch, or the setup's
*Starts when you sign in* check's *Turn on* — here, or asked from another
computer and confirmed here. Only the desktop can change it.

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

Big transfers give way: the files of a move, a copy, a code sync step, a
run's or a task's code, an artifact coming back and the readiness test's
transfer travel at a lower priority than the dashboard and its requests, so
a session on another computer stays quick to drive while a move fills a
slow relay.

## Troubleshooting

- **"… has not accepted the introduction yet."** The introduced computer
  waits (*Waiting for … to accept the introduction there*): press **Pair
  with …** in the *Introductions* card on the other one, within ten minutes.
- **"Update Mother Claude on … first: it can't be introduced yet."** Its
  Mother Claude is older: update it there (**Ask … to update**), or pair the
  two by code.
- **"Revoked here; still to be told: …"** Those computers were off or away:
  each revokes it when it is next connected. *(once it updates)*: its
  Mother Claude is too old to take the request; update it.
- **"DESKTOP-PC can't … until it updates."** Its Mother Claude is older
  than this computer's: **Ask DESKTOP-PC to update**.
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
- **"DESKTOP-PC has not given this computer the Run permission."** Tick
  *Run* for this computer on DESKTOP-PC's Machines page, in the column of
  the network you use now.
- **"app isn't on DESKTOP-PC: … Clone it on DESKTOP-PC first."** That computer has
  no checkout of the project, or no shared folder maps it there: *Set up ›
  Accounts and folders*, *Clone it on DESKTOP-PC* (or add the shared folder
  it is under), then run it again.
- **"Update Mother Claude on DESKTOP-PC: it does not run commands for other
  computers yet."** Its Mother Claude is older: update it there.
- **`npm` fails on Windows: "running scripts is disabled on this
  system".** Without PowerShell 7 a run uses Windows PowerShell 5.1, which
  follows your execution policy, and `npm` is the `npm.ps1` script there.
  Install PowerShell 7 (then restart Mother Claude on that computer), write
  `npm.cmd`, or allow your own scripts there:
  `Set-ExecutionPolicy -Scope CurrentUser RemoteSigned`.
- **A command is not found.** On a Mac, a run's login shell reads
  `.zprofile`, not `.zshrc`: put the `PATH` lines your tools need there
  (nvm, pyenv, …). On Windows, a run gets the `PATH` Mother Claude started
  with: restart it after installing a tool.
- **"Another run used the work folder since: run it again to get the
  file."** The next run reset the work folder. Run the command again and
  fetch the file before another run starts.
- **"Update Mother Claude on DESKTOP-PC: it does not take tasks from other
  computers yet."** Its Mother Claude is older: update it there.
- **"A session that runs in bypassPermissions on DESKTOP-PC needs
  DESKTOP-PC's Dangerous permission for this computer."** Tick *Dangerous*
  for this computer on DESKTOP-PC's Machines page — or hand the task over in
  the default mode, and answer its approvals.
- **"… isn't signed in on DESKTOP-PC."** The task's session runs under the
  account with the asking session's account's name there, else the Default:
  sign that one in on DESKTOP-PC (Accounts there).
- **A task's branch did not move: "… is checked out in …" or "… has commits
  the task's result does not have".** A worktree here has the branch
  checked out, or you committed onto it: Mother Claude never moves it under
  you. The task names the newer result's commit; make a branch of it with
  `git branch <name> <commit>`.
- **A copy says *Paused: the other computer has not given this one the Move
  permission*.** Tick *Move* for this computer on the other computer's
  Machines page, in the column of the network you use now; the copy goes at
  the session's next turn.
- **A copy says *Paused: the other computer keeps less of this one's
  copies*.** It is larger than that computer's cap for this one, or the cap
  made room for newer copies: raise *Copies of other computers' sessions* in
  its Options, or remove copies it keeps.
- **"… doesn't trust … yet" when continuing from a copy.** Open the project
  folder once with Claude Code on this computer, then continue again.
