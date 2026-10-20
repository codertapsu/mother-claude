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

**Settings → Backup & sync** on the desktop:

1. **Folder.** Pick one of the suggestions (iCloud Drive, Google Drive, OneDrive
   or Dropbox when the Mac has them) or any folder. Each account gets its own
   sub-folder named after it, e.g. `Mother Claude/Work/`.
2. **Passphrase.** *Generate* one — eight groups of four characters, 160 bits —
   and write it down: it is shown **once**, then kept in the macOS Keychain
   (`dev.motherclaude.desktop`). *Use my own* takes any passphrase of at least
   twelve characters. Without it no backup can be read, by you or anyone;
   Mother Claude cannot recover it. Changing it later re-keys every set in the
   folder; nothing already backed up becomes unreadable.
3. **Schedule.** *By hand only*, or every *N* hours: an account is backed up
   on schedule only after none of its sessions was active for ten minutes.
4. **Back up now** runs every account at once.

Phones see all of this, read-only.

## What is backed up

Per session, one archive: its transcript, its subagents' and workflows'
transcripts, its file-history snapshots (what `/rewind` restores), its session
environment and its todo lists. A session is archived again only when its
transcript grew or a file of it changed, so after the first run a backup of a
large directory writes only what changed. A session that is running is skipped
until it is idle.

Per account, the **configuration bundle** (the last five are kept): settings,
`CLAUDE.md`, your skills, agents and commands, plugin registrations, prompt
history and `.claude.json` (whole — it holds your MCP servers and per-project
choices, which is why the bundle is encrypted).

**Never backed up:** sign-in credentials (`.credentials.json`; on macOS they are
in the Keychain anyway), the live session list, background jobs, caches, logs,
telemetry, plugin caches and downloaded marketplaces, synced skills, lock files.

## Restoring

**Restore…** lists the sessions of a set, each marked *new here*, *same*,
*newer here* or *newer in the backup*. Pick the ones to bring back and the
account to restore them into. Restoring writes only new files — a session the
account already has is skipped — and can be undone for seven days.

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
settings* unpacks the newest bundle into a folder for you to look through and
copy from.

## Carrying sessions to another laptop

On the first laptop, back up. On the second, set the **same folder and the same
passphrase**, then **Pull…**:

- sessions the set has and this laptop does not are restored;
- a session that is newer in the set and unchanged here since this laptop last
  had it is replaced (the older copy is kept for undo);
- a session that changed **on both** laptops is never merged: the set's
  transcript lands beside the local one as
  `<id>.conflict-<laptop>-<time>.jsonl` and the pull lists it under
  *Conflicts*;
- a session running here is never touched;
- prompt history is merged, without duplicates.

*Pull on launch* (off by default) pulls every account when the app starts —
only when no session is running.

The sign-in does not travel: each laptop signs in to its accounts itself (or
holds the token you gave it). Account sets are matched by account **name**, so
name an account the same on both laptops — renaming an account renames its set.

## Cloud folders

- **iCloud Drive:** right-click the `Mother Claude` folder → **Keep
  Downloaded**. With *Optimize Mac Storage* on, iCloud evicts files it thinks
  you do not need; a restore or pull then waits for them to download, and an
  archive that makes no progress for a minute is reported as failed rather than
  hanging.
- **Google Drive:** *Stream files* works (files download as they are read);
  *Mirror files* keeps a full local copy and makes restores instant. Make sure
  the `Mother Claude` folder is inside *My Drive*.
- Any folder works: an external disk, a NAS share, a Dropbox or OneDrive
  folder.

Pushing from two laptops at once is safe: each push merges the set's index
with what the other wrote.

## Opening a backup without Mother Claude

Every file is a standard [age](https://age-encryption.org) file. The set's key
is in `identity.age`, encrypted with your passphrase:

```bash
cd "Mother Claude/Work"
age -d -o key.txt identity.age                  # asks for the passphrase
age -d -i key.txt sessions/<id>.tar.gz.age | tar xz -C ~/.claude
rm key.txt
```

The paths inside an archive are relative to the config directory
(`projects/<folder>/<id>.jsonl`, `file-history/<id>/…`), so `-C` names the
directory to restore into. `set.json` is the set's only plaintext: the account
name, the set's public key, the machines that pushed and when.
