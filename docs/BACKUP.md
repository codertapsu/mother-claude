# Backups and carrying sessions between laptops

Mother Claude can back up each Claude account's data to a folder you choose —
iCloud Drive, Google Drive, any synced or local folder — encrypted with a
passphrase only you hold. The same folder carries your conversations to another
laptop: push on one, pull on the other.

This is a backup *set* per account, not a live sync of `~/.claude`. Claude Code
rewrites its files constantly, under a lock, and cloud folders evict, re-upload
and duplicate files on their own; syncing the directory itself would corrupt
it. Pushing and pulling are explicit steps (or a schedule while nothing runs),
and every pull can be undone for seven days.

---

## Setting it up

**Settings → Backup & sync**, on the desktop or a paired phone (the passphrase
only on the desktop):

1. **Folder.** Pick one of the suggestions (iCloud Drive, Google Drive, OneDrive
   or Dropbox when the Mac has them; iCloud Drive on Windows) or type any
   folder's full path the way the desktop's system writes it — on a Mac
   `/Volumes/NAS/Mother Claude` or `~/Backups`, on Windows a drive
   (`D:\Backups\Mother Claude`, or `D:/Backups/Mother Claude`), a share
   (`\\nas\backups\Mother Claude`) or `~\Backups`. The field takes either
   system's full path, since a phone may set up a Windows PC; the desktop says
   so when its own system cannot use one, and its hints and examples follow
   its system. Each account with a directory of its own gets a sub-folder
   named after it, e.g. `Mother Claude/Work/`, the same on every machine: a
   name Windows cannot keep as a folder's — one ending in a dot, or a device
   name such as `CON` or `NUL` — gets a `_` (`Acme Inc.` → `Acme Inc._`), and
   a set an older version made under the name itself is still found there. A
   token account uses the Default's directory, so its sessions are backed up
   in the Default's set.
2. **Passphrase.** *Generate* one — eight groups of four characters, 160 bits —
   and write it down: it is shown **once**, then kept in the macOS Keychain
   (`dev.motherclaude.desktop`). *Use my own* takes any one-line passphrase of
   at least twelve characters. Without it no backup can be read, by you or
   anyone; Mother Claude cannot recover it.

   Changing it later re-keys every set in the folder, all or nothing as far as
   it can be: every set is opened first, and if a set of one of your accounts
   opens with neither the old nor the new passphrase, nothing changes. While
   the sets are re-keyed, the new passphrase is kept beside the old one, so an
   interruption leaves every set readable; the page then says the change did
   not finish, and setting the passphrase again finishes it. If a generated
   passphrase was already applied to a set when something failed, the error
   shows it, so it is never lost. A set in the folder that belongs to none of
   your accounts and opens with neither passphrase is left as it is.
3. **Schedule.** *By hand only*, or every *N* hours: a set is backed up on
   schedule only after none of the sessions in its directory was active for
   ten minutes. A set that is still being backed up is never queued again.
4. **Back up now** runs every set at once.

A paired phone, on the LAN or through a tunnel, may do all of this but set the
passphrase: change the folder and the schedule, back up, restore, pull and
undo. Bringing back a folder's trust (below) and forcing a backup, pull or
restore need the dangerous right on the phone, and the desktop shows a notice
naming the phone and what it did.

## The page at a glance

**Backup & sync** opens with a short guide while no backup exists yet —
choose a folder, set a passphrase, back up for the first time — and then
shows **Your backups**, one lane per account:

- **This Mac** (or PC): the account, what kind it is and how many of its
  sessions run;
- the line between them says how it stands — *Backed up 2h ago* (green),
  *Not backed up yet* or a backup older than twice the schedule (amber), a
  failed backup or a missing key (red), *Working…* while a backup, pull or
  restore runs;
- **Backups**: the account's backup, with *Back up*, *Pull…*, *Restore…*,
  *Force backup…*, *Restore settings* and **Conversations**;
- **Other computers** that back up to it, and when they last did.

On the desktop you can do the same by dragging (the buttons stay, and are
what a phone uses):

- an **account onto the backups** backs it up at once;
- a **backup onto its own account** opens *Pull…* for it; onto **another
  account**, *Restore…* into that account;
- **Conversations** lists what the backup holds, grouped by project, each
  saying how it compares with the account here (*Not on this computer*,
  *Newer in the backup*, *Changed on both*, *Up to date*). Drag one — or every
  one you ticked — onto its own account to bring it here at once (a pull of
  just those: newer copies replace the ones here, kept for undo, and both
  sides of a conflict are kept), or onto another account to open *Restore…*
  into it with them selected. One Claude Code would delete again at its next
  cleanup opens *Pull…* first, which says so;
- an **account onto a storage's set** (under Storage, below) opens *Move
  here…* from that account, and a **set onto an account** opens *Bring
  back…* into it.

The folder, the passphrase and the schedule are under **Settings** on the
same page.

## Storages

The backup folder is the first **storage**. Under **Storage** you can add more:
another cloud's folder, a NAS share, an external disk (**Add storage…** offers
the same suggestions as the folder). A storage is any full path of the
desktop's system (on Windows also a drive or a share: `E:\Mother Claude`,
`\\nas\backups\Mother Claude`) that is not inside an account's directory, the
backup folder or another storage — compared by whole folders, on Windows in
any case; its folder is made when only it is missing, and a folder on a disk
that is not plugged in is refused.

Every set in every storage opens with the one passphrase. Changing it re-keys
every set in every storage, opening them all first — and never while a storage
is not there (an unplugged disk): its sets would keep only the old passphrase.
Plug it in, or remove it from the list, and change the passphrase again.

A set named after an account in the **backup folder** is that account's backup:
"Back up now", the schedule and pull on launch use the backup folder alone. Any
other set — "Shared", "Archive 2026", or a set in another storage — is read and
written only when you move or copy conversations there (*Move data*).
Removing a storage forgets it; its files stay where they are.

## Moving conversations into and out of storage

*Move data* — on the desktop or a paired phone — takes conversations, projects
or everything of an account to a set of any storage, and back:

- **Copy into a set** backs up just those conversations, into the account's
  own set or one you name ("Shared", "Archive 2026"). It is a backup: nothing
  changes in the account, so there is nothing to undo.
- **Move into a set** archives them. Each is written to the set anew, the
  set's copy is checked against the files here byte for byte, it is marked
  *in storage only* in the set's index, and only then is it removed from the
  account (into the undo folder, for seven days). A conversation that starts
  or changes meanwhile stays where it is. Its project's prompt history and
  folder settings stay in the account.
- A conversation *in storage only* is never brought by a pull to a laptop that
  does not have it; Restore and Move data list it and bring it back when you
  ask. A laptop that still has it keeps it, and if it continues the
  conversation and backs up, its copy replaces the set's and the mark goes.
- **Copy out of a set** restores into any account, from any set of any
  storage. **Move out of a set** then takes the conversations out of the set;
  their archives stay there for seven days, so that the move can be undone.
- A conversation is moved into a set only once it is closed: an idle session
  whose process is still open is backed up, but it stays in the account until
  it is stopped.
- **From one storage to another**, a copy re-encrypts each conversation to the
  target set's key and checks it before the target lists it; a move then takes
  it out of the source set, kept there seven days for the undo.
- **Undo** (seven days, from the job): a move into a set puts the files back
  and clears the marks; a move out of a set puts its entries back into the
  set first — not over a newer copy another laptop pushed since — and then
  removes the restored files. If the set cannot take them back (an archive is
  gone, the passphrase is not on this Mac), nothing is undone.

## What is backed up

Per session, one archive: its transcript, its subagents' and workflows'
transcripts, its file-history snapshots (what `/rewind` restores), its session
environment and its todo lists. A session is archived again only when it
changed here, so after the first run a backup of a large directory writes only
what changed. A session Claude is working in (a turn running, waiting for an
answer, starting) is skipped until it is idle; an idle one is backed up even
while its process stays open, as the sessions Mother Claude started keep
theirs between turns. If Claude starts working in a session while it is being
read, that archive is dropped and the session is listed as skipped, to back up
again once it is idle: a backup never holds a copy taken mid-turn. A backup's
progress counts the bytes as they are packed, against the total of everything
it will archive, so a long first backup (a conversation of several GB takes a
minute or two) shows how far it is rather than standing still.

Archives are never rewritten: each push of a session writes a new file, the
set's index then names it, and the file it replaced is removed a day later. A
push that is interrupted — the Mac sleeps, the app quits, the disk fills up —
leaves the set as its last index describes it.

Per machine, the **configuration bundle** (each machine keeps its last five,
and writes a new one only when its configuration changed): settings,
`CLAUDE.md`, your skills, agents and commands, plugin registrations, prompt
history and `.claude.json` (whole — it holds your MCP servers and per-project
choices, which is why the bundle is encrypted). A file that cannot be read is
left out of the bundle with a warning rather than failing the backup.

**Never backed up:** sign-in credentials (`.credentials.json`; on macOS they are
in the Keychain anyway), the live session list, background jobs, caches, logs,
telemetry, plugin caches and downloaded marketplaces, synced skills, lock files.

## Restoring

**Restore…** lists the sessions of a set by their title, each marked *new
here*, *same*, *newer here* or *newer in the backup* — and *in storage only*
when it was moved into the set. Pick the ones to bring back and the account to
restore them into. Restoring writes only new files — a
session the account already has is skipped — and can be undone for seven days.

> **Pruning.** Claude Code deletes transcripts older than its
> `cleanupPeriodDays` (30 days unless you changed it). A restored session that
> old is marked *Claude Code will delete this again at its next cleanup unless
> `cleanupPeriodDays` is raised*, and the dialog says so before it starts. Raise
> it in `settings.json` first if you want to keep it.

A folder's **trust** — whether Claude Code may run its project hooks and MCP
servers without asking — is restored only when you tick *Trust this folder for
the target account too*. Without it the conversation comes back and the folder
asks again the first time it is opened. It goes under the name Claude Code on
this machine gives the folder (on Windows `D:/dev/app` for `D:\dev\app`), and
never over a folder's trust the account has already.

The configuration bundle is never restored over your live settings: *Restore
settings* unpacks this laptop's newest bundle (or, on a laptop that has none
yet, the newest of any) into a folder for you to look through and copy from.

## Carrying sessions to another laptop

On the first laptop, back up. On the second, set the **same folder and the same
passphrase**, then **Pull…**:

- sessions the set has and this laptop does not are restored;
- a session that is newer in the set, and unchanged here since this laptop last
  pushed or pulled it, is replaced — and only when the set's transcript
  continues this laptop's, checked byte for byte (the older copy is kept for
  undo);
- a session that changed **on both** laptops is never merged: the set's
  transcript lands beside the local one as
  `<id>.conflict-<laptop>-<time it was pushed>.jsonl` — the time in UTC
  without a `:` (`20261106T134759`), the laptop's name in letters, digits and
  `-`, so any system can hold it — and the pull lists it under *Conflicts*. Pulling again adds no second copy of the same version, and
  neither laptop replaces the other's copy until one of them continues the
  conversation;
- a session running here is never touched;
- the prompt history of every other laptop is merged, without duplicates.

A push works the same way round: it never replaces a copy another laptop pushed
since this one last had it. Such a session is listed as *newer in the set* (a
pull brings it) or, when it changed here too, as a *conflict*. A pull that
fails half-way puts the local copy back.

*Pull on launch* (off by default) pulls every set when the app starts — only
when no session is running.

## Folder pairs: the same project on a Mac and on Windows

Claude Code keeps a project's sessions in a folder named after the project's
path (`/Users/marcus/dev/app` → `projects/-Users-marcus-dev-app`,
`D:\dev\app` → `projects/D--dev-app`), and its session lists — `/resume`,
VS Code's — show the folder of the project you are in. So a session the Mac
backed up comes to Windows under the Mac's path, and Claude Code on Windows
does not list it in `D:\dev\app`.

A **folder pair** fixes that. Pair a folder on one machine with the folder
that holds the same projects on another — `/Users/marcus/dev` on the MacBook
with `D:\dev` on the Windows desktop — and every project under it maps both
ways:

- a pull or a restore (or a copy out of a set) puts a session the other
  machine backed up into the matching project here, `D:\dev\app`; a session
  an earlier pull put under the other machine's path moves there on the next
  pull (kept for undo);
- Mother Claude lists it in that project and *Continue* runs it there; back
  on the Mac it is in `/Users/marcus/dev/app` again, also after it was
  continued on Windows (a backup records where each session lives on the
  machine that backs it up);
- a folder's trust (when you tick it) and your prompt history go with it.

The transcripts are never rewritten: their older messages keep the path they
were written in. Only the folder Claude Code keeps them in is the one of the
project here.

Set it up once, on either machine, under **Folder pairs** on the Backup &
sync page: this machine's folder (`~` for your home folder works, as in the
other folder fields), the other machine (it shows once it has backed up to
or pulled from these backups) and its folder, written the way that machine
writes paths — its examples and checks follow that machine's system. Pairs are kept inside the backups (encrypted), so
every machine uses them and either may add or remove one; two machines
editing at once keep both, and a removed pair stays removed. A pair matches
whole folders only (`/Users/marcus/dev2` is not under `/Users/marcus/dev`);
Windows folders match whatever their letter case and separators; with
several pairs the longest match wins (`D:\dev\work` beside `D:\dev`).

A project no pair maps keeps the other machine's path, as before, and the
pull's report names it: *Pair a folder…* there fills the form.

### Continuing a conversation from another computer

A conversation the other machine started can be continued here with or
without a pair. When a pair maps its folder, *Continue* (or a message sent
to it) runs it in the folder here at once. When none does — or its folder is
gone from this machine — Mother Claude asks **Choose its folder on this
PC** (or Mac) instead of refusing:

- it offers the folders here that look like it: the same path below your
  home folder (`C:\Users\you\development\projects\app` for
  `/Users/you/development/projects/app`), the same path on a drive
  (`D:\development\projects\app`), and folders of other sessions and
  trusted folders with the same name; **Another folder** takes any folder
  typed in full (`~` works);
- **Pair …**, ticked by default, pairs the folders as you answered them:
  the folders above both projects when the two have the same name
  (`D:\development\projects` with `/Users/you/development/projects`), else
  the two project folders. The other conversations from there then continue
  without the question, and pulls put them there. It shows once the other
  machine is one of the backups' machines (it has backed up or pulled);
- the conversation itself is not moved: Claude Code finds it by its id
  wherever the pull or restore put it, and goes on writing it there. From
  then on Mother Claude lists it in the folder it continued in here.

A paired phone gets the same question; the folder it picks must be one
Claude Code already trusts on the desktop.

### Mac ↔ Windows, step by step

1. On the Mac: set the backup folder to a folder on the NAS share
   (`/Volumes/backups/Mother Claude`), set the passphrase, **Back up now**.
2. On Windows: set the folder to the same one as Windows reaches it
   (`\\nas\backups\Mother Claude`, or a drive the share is mapped to), enter
   the same passphrase, and **Back up now** or **Pull…** once — Windows is
   then one of the backups' machines.
3. Under **Folder pairs**, on Windows: `D:\dev` here, the other machine
   `marcus-mbp (macOS)`, its folder `/Users/marcus/dev`, **Add pair** — or
   the same pair from the Mac: `/Users/marcus/dev` (or `~/dev`) there,
   `DESKTOP-PC (Windows)`, `D:\dev`.
4. On Windows, **Pull…**: the Mac's sessions land in their `D:\dev\…`
   projects (one an earlier pull put under the Mac's path moves there).
   `/resume` in `D:\dev\app` lists them, and so does Mother Claude, which
   continues them there. Skipped step 3? *Continue* asks for the folder
   here, and can add the pair then (above).
5. Work on Windows, **Back up now**; on the Mac, **Pull…**: the sessions are
   in `/Users/marcus/dev/…` again, continued.
6. Changed the same session on both? The usual rules keep both copies (one
   beside the other). To make one machine's copy win, **Force backup** on
   that machine, or **Force pull** on the other (below).

## Force backup, force pull, force restore

The rules above never overwrite: when both laptops changed a session, each
keeps its own and gets the other's beside it. When you know which laptop has
the copy you want, force it — like `git push --force`:

- **Force backup** (*Force backup…* on an account's row): this laptop's copy
  of every session it has replaces the backup's copy wherever the two differ —
  also where another laptop backed up a newer copy, or one that changed there
  too. A session only in the backup stays; a session running here is skipped.
  Every copy it replaces stays in the backup folder for **seven days**, so it
  can be fetched back. The other laptop keeps its own copy: its next pull puts
  this laptop's copy beside it (a conflict copy), or replaces its copy when its
  copy is where this one came from; a force pull there makes its copy this
  one's.
- **Force pull** (*Force* in *Pull…*): the backup's copy replaces this laptop's
  wherever the two differ — also where it changed here, is newer here, or
  changed on both (a session whose conflict copy you kept is one too). A
  session only on this laptop stays; a session running here is never touched
  (the pull lists it); a session *in storage only* keeps its rule. This
  laptop's copies are kept for **seven days: Undo** puts them back.
- **Force restore** (*Force* in *Restore…*): the same, into the account you
  restore into: the backup's copy also replaces one it has that is not as the
  backup has it.

Before anything is written, each says how many copies it replaces, what they
are (newer here, changed here, changed on both), that sessions only on the
other side stay and that running ones are skipped. Afterwards the next
ordinary backup or pull on either laptop has nothing to do for those sessions.
On a phone each needs the dangerous right (as deleting does), and the desktop
shows a notice naming the phone.

The sign-in does not travel: each laptop signs in to its accounts itself (or
holds the token you gave it). Account sets are matched by account **name**, so
name an account the same on both laptops — renaming an account renames its set.

Laptops are told apart by an id Mother Claude makes once per install and keeps
in its data folder, never by their name: two Macs with the same name never mix
up what each of them has. A Mac set up from another with Migration Assistant,
or from a backup of it, is recognised and gets an id of its own.

## Cloud folders

- **iCloud Drive:** right-click the `Mother Claude` folder → **Keep
  Downloaded**. With *Optimize Mac Storage* on, iCloud evicts files it thinks
  you do not need; a restore or pull then waits for them to download, and an
  archive that makes no progress for a minute is reported as failed rather than
  hanging. A set whose key (`identity.age`) has not downloaded yet is never
  given a new one: the backup waits until it is there.
- **Google Drive:** *Stream files* works (files download as they are read);
  *Mirror files* keeps a full local copy and makes restores instant. Make sure
  the `Mother Claude` folder is inside *My Drive*.
- Any folder works: an external disk, a NAS share, a Dropbox or OneDrive
  folder.

Pushing from two laptops at once is safe: each push merges the set's index with
what the other wrote, and a push made from an older copy of a session than the
one the other laptop just pushed loses — it is listed as a conflict and its
archive removed — instead of replacing it.

## What someone with access to the folder can do

A cloud folder is often shared, or reachable from more devices than yours.
Without the passphrase nobody can read anything in it. Nor can anyone slip a
session or settings into it: the set's index is authenticated with a key that
only the passphrase unlocks, every archive and bundle is checked against the
index before it is opened, and a set whose index fails the check is refused
(`backup_unauthenticated`) and never written over. Names in the index are never
followed outside the set or the account's directory.

What anyone who can write the folder can still do, as with any folder, is
delete files or put older ones back. A restore then reports an archive as
missing, or as not the one the index names, and writes nothing for it; an older
index makes the set look as it did then until the next push from a laptop that
has the newer copies.

## Opening a backup without Mother Claude

Every file is a standard [age](https://age-encryption.org) file. The set's key
is in `identity.age`, encrypted with your passphrase:

```bash
cd "Mother Claude/Work"
age -d -o key.txt identity.age                  # asks for the passphrase
# Which archive is each session's current one: the index's first line is its
# authentication code, the rest is JSON (each entry's "archive").
age -d -i key.txt index.json.age | tail -n +2 | python3 -m json.tool
age -d -i key.txt sessions/<id>.<hash>.tar.gz.age | tar xz -C ~/.claude
rm key.txt
```

A session's archive is named after its id and the start of the file's SHA-256;
an older one may still be there for a day after a newer push. The paths inside
an archive are relative to the config directory
(`projects/<folder>/<id>.jsonl`, `file-history/<id>/…`), so `-C` names the
directory to restore into. On Windows (PowerShell; `tar` comes with Windows,
`age` is installed as anywhere else) decrypt to a file first — Windows
PowerShell 5.1 does not pipe bytes between two programs unchanged:

```powershell
cd "D:\Backups\Mother Claude\Work"
age -d -o key.txt identity.age
age -d -i key.txt -o session.tar.gz sessions\<id>.<hash>.tar.gz.age
tar xzf session.tar.gz -C "$env:USERPROFILE\.claude"
Remove-Item key.txt, session.tar.gz
```

`set.json` is the set's only plaintext, for display:
the account name, the set's public key, the machines that pushed (their id,
name and OS) and when. Folder pairs are only in the index (`folderPairs`),
never in `set.json`.
