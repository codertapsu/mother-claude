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
   or Dropbox when the Mac has them) or any folder. Each account with a
   directory of its own gets a sub-folder named after it, e.g.
   `Mother Claude/Work/`. A token account uses the Default's directory, so its
   sessions are backed up in the Default's set.
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
undo. Bringing back a folder's trust (below) needs the dangerous right on the
phone, and the desktop shows a notice naming the phone and what it did.

## Storages

The backup folder is the first **storage**. Under **Storage** you can add more:
another cloud's folder, a NAS share, an external disk (**Add storage…** offers
the same suggestions as the folder). A storage is any absolute folder that is
not inside an account's directory, the backup folder or another storage; its
folder is made when only it is missing, and a folder on a disk that is not
plugged in is refused.

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
- **Undo** (seven days, from the job): a move into a set puts the files back
  and clears the marks; a move out of a set puts its entries back into the
  set first — not over a newer copy another laptop pushed since — and then
  removes the restored files. If the set cannot take them back (an archive is
  gone, the passphrase is not on this Mac), nothing is undone.

Moving straight from one storage to another is not available yet: bring the
conversations into an account, then move them on.

## What is backed up

Per session, one archive: its transcript, its subagents' and workflows'
transcripts, its file-history snapshots (what `/rewind` restores), its session
environment and its todo lists. A session is archived again only when it
changed here, so after the first run a backup of a large directory writes only
what changed. A session that is running is skipped until it is idle.

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
asks again the first time it is opened.

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
  `<id>.conflict-<laptop>-<time it was pushed>.jsonl` and the pull lists it
  under *Conflicts*. Pulling again adds no second copy of the same version, and
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
directory to restore into. `set.json` is the set's only plaintext, for display:
the account name, the set's public key, the machines that pushed (their id and
name) and when.
