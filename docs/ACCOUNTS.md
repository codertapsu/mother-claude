# Accounts: several Claude accounts in one dashboard

> **Status: approved 2026-10-21, being built.** The design and the to-do list
> for the work: tick the boxes in §10 as tasks land, and change the design here
> first when the implementation finds it wrong. The review's answers are in
> §11; §3 records where building it changed a decision. §13 adds moving data
> anywhere (storages, bulk moves, customizations) and API servers per account,
> with its own checklist.
>
> Verified against Claude Code CLI 2.1.304 and the Agent SDK 0.3.304 on
> 2026-10-20, on the maintainer's MacBook. Every "verified" claim below was run,
> not read.

---

## 0. The problem

The maintainer has more than one Claude account. Today they all share
`~/.claude`: whichever account is signed in owns every session, and when it
reaches its usage limit the only way on is `/login` as another account. Mother
Claude should let several accounts work at once — each with a user-chosen name —
let the user pick the account per project, conversation or session, move data
between accounts, back the data up to a cloud folder, and carry it between
laptops.

The core user story, the one every phase serves: **account A has hit its limit;
continue this conversation on account B, now, without losing anything.**

---

## 1. What Claude Code gives us

### 1.1 A config directory is a whole world — sign-in included

`CLAUDE_CONFIG_DIR=<dir>` makes a `claude` process keep *everything* under
`<dir>`: its `.claude.json` (account identity, per-project trust, prompt
history, MCP servers, caches), `settings.json`, `projects/` (transcripts),
`sessions/` (pidfiles), `jobs/`, `plugins/`, `skills/`, `history.jsonl`,
`file-history/`, `todos/`.

It also keeps **its own sign-in**. On macOS the credentials are a Keychain
item whose name depends on the directory:

| `CLAUDE_CONFIG_DIR` | Keychain item | Verified |
|---|---|---|
| unset (the default `~/.claude`) | `Claude Code-credentials` | `claude auth status` → signed in, `claude.ai`, max |
| set to a new directory | `Claude Code-credentials-<8 hex>` for that directory | → `loggedIn: false`, `authMethod: none`; the CLI created `.claude.json`, `.claude.json.lock`, `backups/` there and touched nothing under `~/.claude` |
| set to `~/.claude` itself | the hashed name, not the default one | → `loggedIn: false` — the trap `CLAUDE.md` warns about |

The maintainer's Keychain already holds both kinds (`Claude Code-credentials`
and `Claude Code-credentials-4a43521a`). The hash is computed inside the CLI
from the directory; the scheme is not published, and we never need it — we only
ever ask the CLI with the directory set.

On Linux and Windows there is no Keychain: the CLI writes
`<dir>/.credentials.json` instead, so a second directory is a second sign-in
there too, and that file must **never** be copied, backed up or synced.

### 1.2 One account per process, same directory: `CLAUDE_CODE_OAUTH_TOKEN`

`claude setup-token` creates a long-lived token for a subscription account.
With `CLAUDE_CODE_OAUTH_TOKEN=<token>` in a process's environment the CLI uses
that account **without changing the directory**: `claude auth status --json`
reports `loggedIn: true, authMethod: "oauth_token"` (verified with a syntactically
valid dummy token; a real one comes from `setup-token`).

This is the mechanism for "shared projects and conversations, account chosen
per session". It composes with 1.1: a token can be used with any directory.

### 1.3 One process reads exactly one directory

There is no overlay. "The shared `~/.claude` plus per-account directories" is
not something the CLI can be told; where the user wants data in two places,
Mother Claude has to copy or move it (§5.9). The CLI's own messages confirm the
directory can be set from a user or managed settings `env` block too; this
design uses only the environment variable, which we control per child.

### 1.4 What a session is made of

Found by looking for one session id across `~/.claude` (`69ab6cca-…`):

| Path | Holds | Moves with the session |
|---|---|---|
| `projects/<encoded cwd>/<id>.jsonl` | the transcript (append-only NDJSON) | yes |
| `projects/<encoded cwd>/<id>/` | subagent transcripts, workflow journals | yes |
| `file-history/<id>/` | file snapshots for `/rewind` (112 MB in total here) | yes |
| `session-env/<id>` | the session's environment (a directory, may be empty) | yes |
| `todos/<id>*.json` | todo lists, when any | yes |
| `sessions/<pid>.json` | the live pidfile | **never** — exists only while running |
| `history.jsonl` lines | prompt history, keyed by **project** (`{display, pastedContents, timestamp, project}`), not by session | with a project move only |
| `.claude.json` → `projects[<cwd>]` | trust, allowed tools, MCP choices, last cost, `lastSessionId` | the entry is created in the target if missing; never overwritten |

### 1.5 Sizes on the reference machine

`projects/` 6.7 GB (13,926 files), `file-history/` 112 MB, `plugins/` 52 MB,
`skills/` 16 MB, `.claude.json` 105 KB with 24 projects. Transcripts are
append-only, which makes incremental backup cheap; their count and size make a
live cloud sync of the directory expensive and fragile (§3, D7).

### 1.6 Where the shared file lives

With the variable unset the global file is `~/.claude.json` (beside the
directory, not in it); with the variable set it is `<dir>/.claude.json`.
`ClaudeHome::claude_json()` already knows this rule.

---

## 2. Goals and non-goals

**Goals**

1. Several accounts registered under user-chosen names, each either a
   directory (`~/.claude-work`, any path) or a token used with the shared
   directory.
2. Every session, conversation and background job shows which account it
   belongs to; sessions Mother Claude starts, bridge conversations and the
   plan-usage reads use the chosen account.
3. "Continue this conversation on account B."
4. Move or copy sessions and whole projects between directory accounts, with
   an undo.
5. Encrypted, incremental backups of each account's data to any folder,
   iCloud Drive and Google Drive included, and restore from them.
6. Carry data between laptops through the same backup set: push when done,
   pull when sitting down.
7. **Invariant:** with only the Default account registered, behaviour,
   performance and every API response stay exactly what they are in v0.4.

**Non-goals**

- Making the CLI read two directories at once (impossible, §1.3).
- Live two-way synchronisation of a config directory through iCloud or Drive
  (unsafe, D7).
- Syncing sign-ins between machines. Each machine signs in, or holds a token the
  user put there.
- Accounts for the dashboard's *users* (phones). An account here is a Claude
  account; who may use it is decided by the existing principal rules.

---

## 3. Decisions

**D1 — Two kinds of account, and the Default is one of them.**
`kind: "directory"` has its own config directory and its own sign-in;
`kind: "token"` is a long-lived token used with the Default account's
directory. The Default account (`id: "default"`) is a directory account whose
directory is whatever Mother Claude resolves today (`CLAUDE_CONFIG_DIR` of the
app's own environment, else `~/.claude`); it cannot be removed.
*Why not directories only:* the maintainer wants shared projects and
conversations across accounts; only a token gives that without copying.
*Why not tokens only:* some people want work and personal fully apart, and a
directory is the only separation the CLI itself enforces (settings, MCP
servers, trust, plugins).

**D2 — Token accounts are the recommended route; directory accounts are the
hard-separation option.** The UI says so when adding an account. "Continue
on B" is a plain fork for a token account and a copy-then-fork for a
directory account.

**D3 — Secrets live in the macOS Keychain, in Mother Claude's own items.**
Tokens and the backup passphrase go into Keychain items named
`dev.motherclaude.desktop/<account id>` (`security-framework` crate, already in
the dependency tree through `native-tls`); on
Linux and Windows into `0600` files in the app data dir, as `script-token` is
today. Secrets are never written to `accounts.json`, backups, logs, audit
lines or WebSocket frames. `accounts.json` keeps only `tokenSha256`, for
matching a running process to its account (D8).
*Getting a token (P1.1, verified in the CLI 2.1.304 binary):* `claude
setup-token` is an interactive terminal UI (an OAuth flow that opens the
browser, then shows the token), not a command with plain output. Mother Claude
runs it in a pseudo-terminal (`portable-pty`, already a dependency for PTY
attach), strips the terminal escapes and takes the first
`sk-ant-oat<2 digits>-…` it shows; after five minutes without one, or when the
flow asks for a code to be pasted into the terminal, it stops and the UI offers
the paste field instead. A pasted token is accepted after `claude auth status`
with it reports `loggedIn: true`.

**D4 — Writing under a Claude directory stays forbidden, with four explicit
desktop actions as the exceptions.** The existing rule in `CLAUDE.md` (the
plugin installer, a standing grant, a transcript purge) gains: **move/copy**,
**restore from backup**, **pull from a backup set**. Every one of them: is
`DesktopOnly`; refuses while any live process uses the directory it would
write (`sessions/*.json` with a living pid; `.claude.json.lock` younger than
10 s); takes a copy of what it is about to change into
`<app data dir>/undo/<timestamp>/` first; writes files atomically (temp +
rename); writes an audit line. Nothing writes to a directory in the
background, ever.

**D5 — The child environment is composed from the account, not inherited.**

| Account | `CLAUDE_CONFIG_DIR` | auth overrides (`ACCOUNT_OVERRIDE_ENV`) |
|---|---|---|
| Default | left as the app found it (set by the user, or absent) | left as the app found them — exactly today's behaviour |
| directory | set to the account's directory | **removed** |
| token | left as the app found it | **removed**, then `CLAUDE_CODE_OAUTH_TOKEN` set to the account's token |

`ACCOUNT_OVERRIDE_ENV` is the variables that would make a process use some
other identity than the account's: `CLAUDE_CODE_OAUTH_TOKEN`,
`ANTHROPIC_API_KEY`, `ANTHROPIC_AUTH_TOKEN`, `CLAUDE_CODE_USE_BEDROCK`,
`CLAUDE_CODE_USE_VERTEX`, `CLAUDE_CODE_USE_FOUNDRY`. *Changed while building:*
the first draft removed an inherited token from the Default account too; that
broke goal 7 and the existing rule that `SCRUBBED_ENV` never touches auth or
provider variables (someone who signs the Default in with a token or an API key
keeps doing so). A setting a directory account wants for itself (Bedrock, an
API key) goes in that directory's `settings.json` `env` block, which the CLI
applies after the environment. This applies to owned sessions (`control.rs`), the sidecar and every bridge
conversation (`Options.env` per query), the plan-usage helper, `claude auth
status`, `claude auth login`, `claude plugin …` for the account's directory,
and `claude logs/stop/respawn/rm` for the account's background jobs.

**D6 — The backup unit is one session; the format is `tar` + `gzip` + `age`.**
Per session one archive (`sessions/<id>.tar.gz.age`: the §1.4 paths that move
with it), plus a config bundle (settings, `.claude.json`, skills, plugin
registrations, the user's own files at the directory root) kept to the last
five, plus one encrypted index. `age` in passphrase mode (scrypt), pure Rust,
one file format for every platform. A session is re-archived only when its
transcript size or newest mtime changed; live sessions are skipped until idle.
Always excluded: `sessions/`, `jobs/`, `daemon/`, `cache/`, `debug/`,
`telemetry/`, `statsig/`, `paste-cache/`, `shell-snapshots/`, `backups/`,
`plugins/cache/`, `skills/synced/`, `*.lock`, `.credentials.json`,
`stats-cache.json`, `.last-*`.
*Why per session:* iCloud and Drive upload whole files; one 6.7 GB archive would
re-upload on every change, and many small files let restore and pull pick what
they need. *Why encrypt:* transcripts contain code and whatever was pasted;
`.claude.json` contains MCP server secrets and the account identity. *Why
these formats (changed while building, zstd → gzip):* every archive opens with
standard tools and no Mother Claude — `age -d sessions/<id>.tar.gz.age | tar
xz` — and gzip is already in the dependency tree as pure Rust, where zstd
would add a C library to every build, the Windows type-check included.

**D7 — Sync is a backup set used from two machines; pull only when idle.**
No file-level two-way sync of the directory, because: iCloud evicts files to
free space ("Optimize Mac Storage") and a dataless transcript cannot be tailed
or appended; both services re-upload a file on every append; two machines
writing the same file produce "conflicted copy" duplicates; `.claude.json` is
rewritten constantly under a lock; the sign-in does not travel. Instead: push =
backup; pull = restore of sessions the set has newer than local, refused while
a local session with that id is live. Merge rules in §5.11.

**D8 — A foreign session's account is read from where it runs, never
guessed.** A directory account's sessions are the pidfiles in *its*
`sessions/`. A token account's sessions run in the Default directory with the
token in their environment: for a live process of our own user we read that
one variable from the process environment (`KERN_PROCARGS2` on macOS, which
`live.rs` already reads for argv and which carries the environment after it;
`/proc/<pid>/environ` on Linux; not available on Windows → "unknown"), hash it
and compare with `tokenSha256`. The value is compared and dropped — never
stored, logged or served. A token we do not know shows as *token
(unregistered)*. Hook events also carry `transcript_path`, which names the
directory, so events from the plugin are attributed the same way.

*Changed while building (§4.4, §5.6):* the `planUsage` frame stays the
Default account's, unchanged; every other account's usage travels as
`{t:"accountUsage", accountId, usage}`. An older phone client ignores the new
frame, where it would have shown another account's usage as the Default's had
`planUsage` merely gained an `accountId`. A token account's `get_usage` is
expected to answer "unavailable" (its token is inference-only), so its owned
sessions' `rate_limit_event`s are its source and an "unavailable" read only
stops further reads.

*Changed while building (D3/§4.1):* `initialize.account` carries `email`,
`organization`, `subscriptionType`, `tokenSource`, `apiKeySource` and
`apiProvider` but no account id, so `accountUuid` is not learned from it. A
directory account's CLI cache is checked against its own `.claude.json`
`oauthAccount`, as the Default's always was; a token account never uses the
cache (§5.6).

*Changed while building (D8, verified 2026-10-21 on macOS 27 / Darwin 27.0):*
`KERN_PROCARGS2` no longer returns another process's environment — not even
for a child of the asking process (`ps -E` shows none either). On macOS a
token account's foreign sessions are therefore attributed through the hooks
plugin: `mc-hook` runs inside the session's process tree, hashes its
`CLAUDE_CODE_OAUTH_TOKEN` there (`openssl`/`sha256sum`/`shasum`, fed by the
`printf` builtin, so the token is in no argv) and sends only
`X-MC-Token-Sha256`. The environment read stays for Linux. A session without
the plugin, or that has fired no hook yet, shows the Default until it does.
Hook events are not attributed by `transcript_path` (§5.7): the dispatcher
never trusts a path a hook names, and every session that writes a pidfile is
attributed by the directory it is in.

*Changed after the review (D8, goal 7):* while no token account is
registered, a session's own `CLAUDE_CODE_OAUTH_TOKEN` is ignored entirely (the
hook's hash, `/proc`, the roster), so a terminal session that uses the user's
own token behaves exactly as before accounts. Once one exists, a token nobody
registered shows the *token (unregistered)* chip, and every decision — plan
usage, fork or resume in place, copy consent — treats the session as the
Default's. A session's account is resolved in this order: (1) its owned handle
or bridge conversation; (2) the live process — the pidfile's directory, and its
token only while token accounts exist (a live session whose token is not known
yet shows the Default and is not remembered); (3) the account seen this run,
or the token account remembered for it (`account-sessions.json`), but only
while the index finds the transcript in that account's directory — so a moved
session follows its transcript; (4) the index, then the Default. Registry
entries a run cannot use (another `CLAUDE_CONFIG_DIR`, a newer version's
kind) are written back unchanged on save, with their consents and defaults.
A successful `claude auth login` counts as a new sign-in even with the same
email, so it clears an "authentication failed" mark. The file watcher
re-makes itself when a directory it watches appears (a new account's
`projects/`).

*Changed while building (D4, §5.9):* a write touches a shared file only
when it must. A copy without the trust opt-in writes no `.claude.json` at all
(the project's entry *is* the trust — `hasTrustDialogAccepted`, allowed
tools, MCP choices — and the CLI resumes a conversation by its id without
it), so it touches only new files and runs while the target account has live
sessions; a project's `history.jsonl` lines, a trusted entry, and a project
move's removals from the source still refuse while a live process uses the
directory they would write. A session move needs no idle source: only the
moved session's own files are removed, and only after every copy verified —
renamed into the undo directory, so a move costs no extra space on one
volume. Undo deletes what was created (refusing when any of it changed since:
a copied conversation that was continued is not taken back), puts moved files
back, and removes exactly the `.claude.json` entry and the `history.jsonl`
lines that were added.

*Changed after the review (D4, §5.9):* a move never loses a file and nothing
acts on a stale plan. A job checks its plan again once it holds the jobs lock
(it may have waited behind a backup), again before it writes shared files and
again before the source moves: a session that started meanwhile
(`session_live`) or wrote a file since its copy verified (`session_changed`)
stops the move before anything leaves the source, and a session's folders
lose only what is empty. A run that fails after files moved puts them back
first, then removes its copies; if even that fails the undo directory and its
manifest stay, the job says `transfer_incomplete` and offers Undo. Undo
refuses while a session of the job runs or (when it changes shared files back)
while their directory is in use, puts the moved files back before deleting the
copies, and can be retried. A project **move without the trust opt-in leaves
the source's `.claude.json` entry where it is** (it holds the project's MCP
servers and allowed tools; only an entry carried to the target leaves the
source); its prompt history still moves. Each shared file is kept in
`undo/<job>/shared/` before the job's first change to it, for recovery by
hand. `history.jsonl` is edited as bytes and an unreadable one is an error,
never an empty file to write over; symlinks in a session's place are never
followed; "continue on" never waits behind another data job
(`data_job_running`). Job views name their accounts (`from`, `to`; `account`
for backup jobs) and `GET /data/jobs` adds earlier runs' jobs that can still
be undone, from their manifests.

*Changed while building (§5.4, P3.3):* "continue on" a directory account
copies a **snapshot**: the conversation usually still has a process (it
stopped at its limit), so the copy is of what is there now, an unfinished
last transcript line is left out, and each file is verified against the
source's first bytes. The account pair's copy consent is given on the desktop;
after that a continue-on from any surface may copy for that pair, as §11.2
intends. A second continue-on brings the copy already there up to date first
when it is only behind the original (nothing changed it in the target; the
older copy is kept for undo), so the fork has every turn; a copy that was
continued in the target is forked as it stands.

*Changed while building (D6, §4.3):* age's passphrase mode costs about a
second of scrypt per file — per archive, that would make a first backup of a
few hundred sessions take minutes of key derivation alone. Each set therefore
has its own X25519 key: `identity.age` holds it, encrypted with the passphrase
(scrypt), and replaces `check.age` (opening it proves the passphrase);
archives and the index are encrypted to the set's public key, which
`set.json` names. A run pays the passphrase once. Every file still opens with
standard tools — `age -d -o key.txt identity.age && age -d -i key.txt
sessions/<id>.tar.gz.age | tar xz` (verified with age 1.3.2) — and a new
passphrase only re-encrypts `identity.age`. The index is written merged with
the set's current one (the newer push of a session wins; each machine's
`seen` is its own), so two machines pushing in turn keep each other's
entries. Also excluded, as this machine showed them to be runtime state or
re-fetchable: `ide/`, `tasks/`, `plugins/marketplaces/`, `plugins/synced/`,
plugin cache files, `*.log`. The configuration bundle is never restored over
the live configuration: restoring it unpacks the newest one into
`<app data dir>/restored-config/<account>/` for review. A pull's prompt
history comes from the newest bundle; a session newer in the set and
unchanged here is replaced with the old copy kept for undo (the manifest is a
transfer's, so `/data/jobs/{id}/undo` undoes restores and pulls too). See
[BACKUP.md](BACKUP.md).

**D9 — Account choice is not a dangerous action; account administration is.**
Picking an account for a new session or a bridge conversation is allowed to
any principal that may start sessions (it is the user's own account either
way). Adding, removing or signing in an account, entering a token, moving
data, restoring, pulling and changing backup settings are `DesktopOnly`.

*Changed 2026-10-21 (§13.9): the user runs Mother Claude from the phone, on
the LAN or through a tunnel, and wants the data and API-server features there
too.* Moving and copying data, backups, restores, pulls, undo, storages, backup
settings, account defaults, copy consent, renaming accounts and API servers
are open to the desktop and to paired devices; the parts that run code or
change trust need the dangerous right; secrets, sign-ins and adding or
removing accounts stay on the desktop. §13.9 has the split.

---

## 4. Data model

### 4.1 The registry — `<app data dir>/accounts.json`, mode 0600

```jsonc
{
  "version": 1,
  "accounts": [
    { "id": "default", "name": "Default", "kind": "directory", "dir": null,
      "color": "#8b5cf6", "createdAt": 0 },
    { "id": "acc_5f1c9e2b7a40", "name": "Work", "kind": "directory",
      "dir": "/Users/marcus/.claude-work", "color": "#0ea5e9",
      "createdAt": 1792000000000, "hooksPlugin": "installed" },
    { "id": "acc_9b02d3e6c1f8", "name": "Marcus (Pro)", "kind": "token",
      "tokenSha256": "3f…", "accountUuid": null, "color": "#f59e0b",
      "createdAt": 1792000000000 }
  ],
  "defaults": {
    "newSession": "default",
    "bridge": "default",
    "byProject": { "/Users/marcus/development/projects/gitweaver": "acc_5f1c9e2b7a40" }
  },
  "copyConsent": [ { "from": "default", "to": "acc_5f1c9e2b7a40" } ],
  "backup": {
    "folder": "/Users/marcus/Library/Mobile Documents/com~apple~CloudDocs/Mother Claude",
    "everyHours": 6,
    "pullOnLaunch": false
  }
}
```

Rules: `id` is `default` or `acc_` + 12 hex; `name` unique, 1–40 characters,
trimmed; `dir` absolute, not the Default directory, not inside another
account's directory, created with mode 0700 when missing; a token account has
no `dir`. `accountUuid` is learned from the first `initialize.account` seen
for the account (owned session or helper) and used to attribute the CLI's own
usage cache (`cachedUsageUtilization`) correctly; it is an opaque id, not an
email. The file is parsed tolerantly (unknown fields kept, bad entries
skipped with a log line), like every other file we read.

### 4.2 Secrets

| Secret | macOS | Elsewhere |
|---|---|---|
| a token account's token | Keychain `dev.motherclaude.desktop` / account `acc_…` | `<app data dir>/secrets/<acc_…>.token`, 0600 |
| the backup passphrase | Keychain `dev.motherclaude.desktop` / account `backup-passphrase` | `<app data dir>/secrets/backup-passphrase`, 0600 |

### 4.3 The backup set — `<folder>/<account name>/`

```
set.json                      {"version":1,"account":"Work","createdAt":…,"recipient":"age1…","machines":[{"host":"marcus-mbp","lastPush":…}]}
identity.age                  the set's X25519 key, encrypted with the passphrase (§3, D6) — opening it proves the passphrase
index.json.age                {"sessions":{<id>:{"id","cwd","transcriptBytes","newestMtime","host","pushedAt","archiveSha256","archiveBytes","files"}},"config":[…],"seen":{<host>:{<id>:bytes}}}
sessions/<session id>.tar.gz.age     (with .mc/meta.json: id, cwd, the project's entry)
config/<ISO timestamp>.tar.gz.age    (last five kept)
```

Paths inside an archive are relative to the config directory, so a session
restores into any directory account, and the transcript's `projects/<encoded
cwd>` folder is recreated as it was.

### 4.4 API additions (all under `/api`, camelCase like the rest)

| Route | Guard | Does |
|---|---|---|
| `GET /accounts` | `Authed` | the registry without secrets, each account with `signedIn`, `email`, `subscription`, `hooksPlugin`, `live` (session count) |
| `POST /accounts` | `DesktopOnly` | add a directory account `{name, dir}` or a token account `{name, token}` (the token goes straight to the Keychain) |
| `PATCH /accounts/{id}` | `DesktopOnly` | rename, colour, make it the default for new sessions / the bridge |
| `DELETE /accounts/{id}` | `DesktopOnly` | forget the account (never deletes its directory or its sign-in) |
| `POST /accounts/{id}/login` | `DesktopOnly` (dangerous, as today) | `claude auth login` with the account's environment |
| `POST /accounts/{id}/token` | `DesktopOnly` | run `claude setup-token` for the account, capture the token (or accept a pasted one) |
| `POST /accounts/{id}/hooks/install` | `DesktopOnly` | the plugin installer with the account's directory |
| `PUT /accounts/defaults` | `DesktopOnly` | `newSession`, `bridge`, `byProject` |
| `POST /sessions/{id}/continue` | `Authed` + the existing bypass rules | `{account}` → fork under that account (copy first for a directory account) |
| `POST /data/move` | `DesktopOnly` | `{what: "session"\|"project", id\|cwd, from, to, mode: "move"\|"copy"}` → a job id |
| `GET /data/jobs/{id}` | `Authed` | progress, result, undo token |
| `POST /data/jobs/{id}/undo` | `DesktopOnly` | restore from the undo copy |
| `GET /backup`, `PUT /backup/settings`, `POST /backup/push`, `GET /backup/index`, `POST /backup/restore`, `POST /backup/pull` | `DesktopOnly` (read: `Authed`) | §5.10–5.11 |

Existing responses gain fields: `SessionSummary.account` (`{id, name, color,
kind}`; absent only for a session in a directory nobody registered),
`BridgeThread.account` (a background task shows its session's); `GET /usage` keeps its shape
for the Default account and adds `accounts: {<id>: PlanUsage}`; other
accounts' usage comes as `accountUsage {accountId, usage}` frames (§3); `POST /sessions` (spawn), `POST
/api/bridge/start` defaults and the bridge's own `POST /threads` / `/chat`
accept `account`. A new `accounts` WebSocket frame carries the registry
view when it changes.

---

## 5. Architecture changes (Rust)

### 5.1 From one home to many — `HomeSet`

`AppState.home: ClaudeHome` is read in 29 places across 30 files. It stays,
as the Default account's home, so that code reading "the" home keeps
working. Beside it: `AppState.accounts: AccountRegistry` and
`AppState.homes()` → `Vec<(AccountId, ClaudeHome)>` (the Default plus every
directory account; token accounts map to the Default home). Everything that
*enumerates* data moves to `homes()`; everything that *resolves a session id*
asks the index, which knows the home of every file it indexed. New module
`claude/accounts.rs` (registry, validation, env composition, secret access).

### 5.2 Roster per home (`claude/live.rs`)

`LiveRoster` scans `<home>/sessions/*.json` for each home and tags every
`LiveSession` with its account. For sessions in the Default home it also
reads the process environment for `CLAUDE_CODE_OAUTH_TOKEN` (D8), once per
pid, cached with the argv it already reads (`process_argv` grows a sibling
`process_env_var(pid, name)`). `ignore_cwd` (the usage helper's directory)
becomes per account.

### 5.3 Index and watcher per home

`TranscriptIndex` keeps one `FileIndex` per home and one checkpoint file per
home (`index-cache.json` for the Default, `index-cache-<acc_…>.json`
otherwise). Session ids are UUIDs, so a lookup by id checks every home and
the first hit wins; a duplicate id in two homes (a copied session) is reported
as the same session with `account` set to the home it is *live* in, else the
most recently written. `Watcher` watches every home's `projects/`, `sessions/`,
`jobs/`.

### 5.4 Owned sessions (`claude/control.rs`)

`SpawnOptions.account: AccountId` (default: `defaults.newSession`, or
`defaults.byProject[cwd]` when the cwd matches). `apply_child_env` takes the
account and composes the environment per D5. The handle remembers the account;
`SessionSummary.account` comes from it, not from the roster. "Continue on B"
(`POST /sessions/{id}/continue`): token account → `spawn` with `resume: id,
fork: true` under B; directory account → copy (§5.9) then the same fork.
The `initialize` response's `account` is already read (`AccountFacts`); it
now also fills `accountUuid` in the registry.

### 5.5 Bridge conversations (`bridge/`, `sidecar/src/bridge-host.ts`)

`BridgeDefaults.account` (start-time default, chosen on the API screen) and a
per-request `account` on `POST /threads` and `/chat`. The sidecar's
`buildOptions` sets the SDK's `Options.env` per query from an `env` object the
Rust side sends with the session options (the sidecar process itself keeps the
scrubbed environment; the token is handed per query so one host serves every
account). `ThreadRecord.account` is persisted with the thread.

*Changed after the review:* a conversation of a token account keeps its account
across a restart through `<app data dir>/account-sessions.json` (0600, token
accounts only, at most 2000 entries, pruned an hour after a transcript is
gone; never written while only the Default exists). With only the Default the
bridge answers as before accounts: a thread's `account` is left out when it is
the Default's and the start-time defaults leave out an unset one. A folder's
trust for a bridge call is checked in the conversation's account's
`.claude.json`, not the Default's. Removing the account the running API uses
as its start-time default is refused (`409 account_used_by_bridge`; any
running API server's, §13.5). The
sidecar gives every conversation the environment the SDK would have built
without `env` (it removes the same variables), and only store calls that
point `CLAUDE_CONFIG_DIR` elsewhere run one at a time; the Default's never
queue.

### 5.6 Plan usage per account (`claude/usage.rs`)

`UsageStore` becomes a map `AccountId → AccountUsage` with the same scheduler
per entry. The helper runs with the account's environment in
`<app data dir>/usage-helper/<acc_…>/`. A `rate_limit_event` from an owned
session updates that session's account. The `.claude.json` cache fallback is
read from the account's directory and only when its `oauthAccount.accountUuid`
matches the account's `accountUuid`. Limit-hit triggers are per account, and a
hit raises the "continue on …" offer for sessions of that account (§6.2).

### 5.7 The hooks plugin per directory account (`claude/hooks.rs`)

`claude plugin marketplace add` / `install` / `list --json` run with the
account's directory; `hooksPlugin` in the registry caches the answer. Hook
events already carry `transcript_path`; the dispatcher maps it to the home, so
attribution needs no new wire format. The hook token file (`hook.curlrc`) is
shared — it authenticates the plugin to us, not an account.

### 5.8 Sign-in status per account (`claude/auth.rs`)

`refresh_claude_auth` runs `claude auth status --json` once per account, in
parallel, at startup and after a login; `begin_login` takes the environment.
For token accounts the status comes from the same command with the token
set. An `authentication_failed` from an owned session or the bridge marks
the account "needs sign-in" immediately.

### 5.9 Moving and copying (`claude/transfer.rs`)

Preconditions, all checked before touching anything, each with its own error
code: desktop principal; `from` ≠ `to`, both directory accounts (a token
account shares the Default directory — nothing to move); the session is not
live anywhere (no living pidfile with its id in any home, no owned handle, no
bridge thread) and not mid-index; `to` has no session with that id
(`already_exists`); no live process uses `to` (`directory_in_use`) and, in
move mode, none uses `from`; `to/.claude.json.lock` absent or older than 10 s.

Steps: (1) copy everything to be changed into `<app data dir>/undo/<ts>/`;
(2) copy the §1.4 paths into `to`, verifying size and SHA-256 of each file
after the copy; (3) in `to/.claude.json`, create `projects[<cwd>]` from the
source entry when missing (never overwrite), written atomically; (4) for a
project: every session of the cwd as above, plus the `history.jsonl` lines
whose `project` is the cwd (appended to `to`), plus the `projects[<cwd>]`
entry removed from `from` in move mode; (5) in move mode delete the source
files only after step 2 verified; (6) tell the index to rescan both homes;
(7) audit line and an `accounts`/`sessions` frame. Undo replays the copy from
step 1 and is kept for seven days. A trust flag (`hasTrustDialogAccepted`) is
copied only when the user ticks "trust this folder for the target account
too" — a trusted folder runs its hooks and MCP servers unasked.

### 5.10 Backup (`claude/backup.rs`)

Engine: walk the account's directory with the §1.4 grouping and the D6
exclusions; for each session compare `transcriptBytes` + `newestMtime` with
the set's index; archive the changed ones (`tar` → `gzip` → `age`
passphrase) to a temp file in the set folder, fsync, rename; rewrite
`index.json.age`; bundle the config once per run when any of its files
changed. Skip sessions that are live. Report progress through a job
(`/data/jobs/{id}` reused). Schedule: manual, plus every `everyHours` while
no session of that account has been active for 10 minutes. Crates: `tar`,
`flate2`, `age` (passphrase mode), `walkdir`. Dataless files (iCloud, Drive)
are handled by reading through the file provider, which downloads on
demand; a read that stalls for 60 s fails that archive, not the run.

### 5.11 Sync = pull (`claude/backup.rs`)

Pull reads `index.json.age` and offers the sessions whose index entry is not
on this machine, or whose `transcriptBytes` is larger and `newestMtime` newer
than the local file. Rules: a session live locally is never touched; when
both sides changed (local file larger than the index's *previous* entry for
this host and the set's entry larger still), the remote copy is restored
beside the local one as `<id>.conflict-<host>-<ts>.jsonl` and listed under
"conflicts" in the UI, never silently merged; `.claude.json` project entries
are added when missing and never overwritten; `history.jsonl` lines are
appended de-duplicated by `(timestamp, display)`; trust flags are restored
only with the same opt-in as §5.9. "Pull on launch" is off by default and
only runs when no session is live.

*Changed after the review (§5.10, §5.11; docs/BACKUP.md has the user's
version):* a backup folder may be shared or reachable from other devices, so a
set is treated as untrusted input. **Set format 2:** the index's first line is
an HMAC-SHA256 of the rest, keyed from the set's secret key (which only the
passphrase opens); a missing or wrong one is refused (`backup_unauthenticated`)
and never written over. Archives and bundles are checked against the
authenticated index's SHA-256 before they are decrypted; `set.json` is display
only (a set.json naming another key stops a push; the recipient comes from the
decrypted key). Every session id and bundle name from a set is validated
before it names a path, and stage folders are always new. A set whose key is
missing or still an iCloud placeholder is never given a new key
(`set_key_missing`). **Archives are immutable**
(`sessions/<id>.<12 hex>.tar.gz.age`, `config/<time>-<12 hex>.tar.gz.age`):
the index names the new file after it is written, a replaced file is removed a
day later, orphans too, and each machine keeps its own five bundles; an
unreadable configuration file is a warning, never a lost index. **Machines**
are told apart by a random per-install id (`<app data dir>/backup-machine.json`,
0600; a Migration Assistant copy on other hardware gets a new one), not by
host name. **Push and pull rules:** `seen` records each machine's bytes, whole
seconds and archive name, so a restored or pulled session compares the same
and nothing ping-pongs between machines; a push replaces the set's copy only
when it changed here and the set's transcript is a byte prefix of the local
one, else it is a conflict or *newer in the set*; a pull updates only when the
local transcript is a byte prefix of the set's, else the set's copy lands
beside it as a conflict copy named after its push time (pulling again adds no
duplicate). The set's copy is staged before anything local moves, the undo
manifest is written first and the local copy is put back on any error.
**Passphrase change:** every set in the folder is opened first and nothing
changes if a set of an account here opens with none of this machine's
passphrases; the new one is kept as pending in the Keychain while sets are
re-keyed (every set opens with the current or a pending one), and becomes the
passphrase only at the end; a failure after a generated passphrase reached a
set returns it. **One set per directory:** a token account's sessions are in
the Default's set (push-all, the schedule, pull-on-launch and re-keying loop
over directories), and a set that is being backed up or pulled returns that
job instead of queueing another. Pulling everything lists a set it cannot
start in `skipped` (`{account, code, message}`) and pulls the rest. Index
entries keep the session's title, only inside the encrypted index.

---

## 6. UI (Angular)

1. **Settings → Accounts** (new page, desktop-only actions): the list with
   name, kind, directory, sign-in state, plan usage sparkline, hooks plugin
   state; *Add account* (choose "shared data, another account" → token, or
   "separate directory" → directory, with the trade-offs in one sentence
   each); rename, colour, remove; defaults for new sessions and the API; the
   per-project table.
2. **Account picker** on *New session* (pre-selected from `byProject`, then
   the default), on the API screen's defaults, and as a **Continue on…** entry
   in a session's menu. When an account hits a limit, the sessions list and
   the session page show "Account *Work* has reached its 5-hour limit — continue
   on *Marcus (Pro)*?".
3. **Badges and filters:** a coloured account chip on session cards, the
   session header, Inbox items, Agents and background jobs; an account filter
   on Sessions and Inbox; the fleet summary per account.
4. **Plan meter** shows the selected account with a switcher; the sheet lists
   every account's windows.
5. **Move/copy dialog** from a session or project menu: source, target, move
   or copy, the trust checkbox, a dry-run list of what will be written, then
   progress and an *Undo* button for seven days.
6. **Settings → Backup & sync:** folder picker with iCloud Drive and Google
   Drive suggested when present, passphrase (generated, shown once, kept in
   the Keychain), *Back up now*, schedule, last push per account, *Restore…*
   and *Pull…* with the session list and conflicts.
7. Phones see all of it read-only except the account picker (D9).

---

## 7. Security review

- Tokens: Keychain only; `tokenSha256` in the registry; never in frames, logs,
  audit lines, backups or error messages. The process-environment read (D8)
  reads one variable of our own user's processes, compares, drops.
- Adding, removing and signing in accounts, tokens, the backup passphrase,
  the hooks plugin installer: `DesktopOnly`. Data moves, backups, restores,
  pulls, undo, storages, backup settings, account defaults, copy consent,
  renaming, API servers: the desktop and paired devices, with a folder's trust
  and settings or MCP customizations needing the dangerous right (§13.9).
  Account choice: any principal that may start sessions.
- Writes under a Claude directory: only the four explicit actions (D4), each
  with the in-use check, the undo copy, atomic writes and an audit line.
- Backups: encrypted at rest with a passphrase the user holds; the set's only
  plaintext is `set.json` (account name, host names, timestamps).
- Trust flags are never restored or copied without the opt-in (§5.9).
- The CLI's `bypassPermissions` rules are untouched; the account does not
  change what a caller may do.
- A `/login` in the Default directory still changes the sign-in for every
  process without a token; the Accounts page shows the Default's email so a
  switch is visible.

---

## 8. Compatibility and documents to change

- `CLAUDE.md`: the "never write under `~/.claude`" rule lists the three new
  exceptions and their conditions; the `CLAUDE_CONFIG_DIR` rule becomes "set
  it only to a directory account's own directory, never for the Default";
  the data-sources table says "per home" for the roster, transcripts, jobs
  and hooks, and gains an *Accounts* row.
- `docs/REMOTE.md`: what phones can do with accounts; the account chip.
- `docs/BRIDGE.md`: `account` on start defaults, `/threads` and `/chat`.
- `README.md` and `.env.example`: the two kinds of account; the Keychain fact.
- `SECURITY.md`: §7 above.
- The `script-token` and device principals are unchanged; old phones keep
  working (new fields are additive, `GET /usage` keeps its shape).

---

## 9. Test plan

- Unit (Rust): registry load/validate/save (bad entries skipped, secrets never
  serialised); env composition per D5, table-driven; roster attribution with
  fixture pidfiles in two temp homes; the environment-variable matcher on a
  recorded `KERN_PROCARGS2` buffer; index lookup across homes incl. a
  duplicated id; usage store keyed by account; transfer engine on temp
  directories (copy, move, every refusal, undo, trust opt-in); backup
  round-trip (archive → restore byte-identical; wrong passphrase; truncated
  archive; live session skipped; incremental skip); pull merge rules incl. a
  conflict.
- Integration (`src-tauri/tests`): `fake-claude.py` logs `CLAUDE_CONFIG_DIR`
  and whether `CLAUDE_CODE_OAUTH_TOKEN` is set (never its value) → spawn,
  continue-on, helper and login carry the right environment; route guards
  (`DesktopOnly`, a device gets 403); bridge thread with an account.
- Angular: Accounts page, picker, badges, plan meter switcher, move dialog.
- Live, `#[ignore]`d, on the maintainer's Mac: two real directory accounts, a
  real `setup-token`, a real iCloud folder; a push on one Mac and a pull on
  the other.
- Performance: the steady tick with three homes stays under 100 ms (today
  69 ms with one).

---

## 10. The checklist

Tick as tasks land. Each task names its files and what "done" means; tests are
part of done. IDs are referenced from commit messages.

### Phase 0 — ground rules and scaffolding

- [x] **P0.1 Rules.** `CLAUDE.md` per §8 (the exceptions, the environment
      rule, the table; "per home" lands with Phase 2). Done when a reviewer can tell from `CLAUDE.md` alone
      what may write under a Claude directory and when.
- [x] **P0.2 Registry module.** `claude/accounts.rs`: `Account`,
      `AccountRegistry` (load, save, validate, add, rename, remove, defaults),
      `accounts.json` 0600. Done when the unit tests in §9 pass and a
      registry with only the Default round-trips to today's behaviour.
- [x] **P0.3 Secrets.** `claude/secrets.rs`: Keychain on macOS
      (`security-framework`), 0600 files elsewhere; get/set/delete; never
      logs values. Done when a token stored on macOS appears in Keychain
      Access under `dev.motherclaude.desktop` and nowhere on disk.
      *(2026-10-21: the `#[ignore]`d `keychain_round_trip` test filed, found
      with `security find-generic-password` and deleted a dummy item. The
      Keychain is used only for the app's own data dir, so tests get files or
      memory; `MOTHER_CLAUDE_SECRET_STORE=file` opts out on macOS.)*
- [x] **P0.4 Environment composition.** `control::apply_child_env(cmd,
      account)` per D5 (`accounts::AccountEnv`, `ACCOUNT_OVERRIDE_ENV`);
      `SCRUBBED_ENV` stays as it was, since the Default keeps an inherited
      token (D5, changed while building). Done when the table-driven test
      covers all three kinds.
- [x] **P0.5 Fake CLI.** `tests/fixtures/fake-claude.py` logs the two
      variables (value of the first, presence and SHA-256 of the second, and
      which override variables are present). Done when
      `tests/owned.rs` can assert them.

### Phase 1 — accounts and "continue on"

- [x] **P1.1 Spike: `setup-token` capture.** Verify what `claude setup-token`
      prints and when; decide capture vs paste. Record the answer in §3 (D3).
      *(Read from the CLI 2.1.304 binary, not run — a real run mints a real
      token: the success branch only displays the token, then exits after
      500 ms; a manual-code prompt exists. Capture first, paste as fallback.)*
- [x] **P1.2 `GET/POST/PATCH/DELETE /accounts`** with `DesktopOnly` on
      writes; the `accounts` frame; `AccountView` without secrets. Done when
      a phone can list accounts and gets 403 on a write.
- [x] **P1.3 Sign-in per account.** `auth.rs` status per account at startup
      (parallel), `POST /accounts/{id}/login`, `POST /accounts/{id}/token`
      (capture or paste). Done when a fresh directory account goes from
      "signed out" to an email in the UI without a terminal.
- [x] **P1.4 Owned sessions carry an account.** `SpawnOptions.account`,
      handle + `SessionSummary.account`, `defaults.byProject`. Done when the
      fake CLI logs the expected environment for each kind.
- [x] **P1.5 Continue on another account.** `POST /sessions/{id}/continue`;
      fork for token accounts; "copy first" wired to P3 (until then, refuse
      directory targets with a clear error). Done when a token-account fork
      answers under the other account in the live test. *(Against the fake
      CLI: the fork's argv and its token's hash; the live run with a real
      second account is the maintainer's.)*
- [x] **P1.6 Plan usage per account.** `UsageStore` keyed by account; helper
      per account; `GET /usage.accounts`; ~~`planUsage.accountId`~~ an
      `accountUsage` frame (§3); limit-hit offer (UI, P1.8). Done when two
      accounts show two meters and a limit hit names the right one.
- [x] **P1.7 Bridge per account.** `BridgeDefaults.account`, per-request
      `account`, sidecar `Options.env` per query, `ThreadRecord.account`,
      BRIDGE.md. Done when `/chat` with `account` runs under that account in
      the live bridge test. *(Offline against a fake host: each
      conversation's environment by kind, the fork and copy rules, the
      refusals. Every conversation now gets an explicit `env` (the host's own
      as it started, plus the account's part), so the host's store reads can
      point `CLAUDE_CONFIG_DIR` at another account's directory, one at a
      time, without reaching a conversation.)*
- [x] **P1.8 UI: Accounts page, picker, meter switcher, limit offer.**
- [x] **P1.9 Docs:** REMOTE.md, README, .env.example, SECURITY.md (§8).

### Phase 2 — see every account

- [x] **P2.1 `HomeSet`.** `AppState.homes()`; every enumerating call site
      moved (list them in the commit). Done when a session in a directory
      account's home appears in the list with its badge.
- [x] **P2.2 Roster per home + environment matcher** (D8). Done when a
      terminal session started with a registered token shows its account.
      *(macOS: through the hook's token hash, see §3; tested with the hook
      script and with a recorded hash.)*
- [x] **P2.3 Index and watcher per home**, checkpoint per home, duplicate-id
      rule. Done when the three-home tick stays under 100 ms. *(2026-10-21,
      release build, the real Default home plus two directory accounts of 20
      sessions each: median tick 25 ms, against 22 ms with one home —
      `measure_three_homes_tick`.)*
- [x] **P2.4 Hooks plugin per directory account** (`/accounts/{id}/hooks/
      install`, `hooksPlugin` state, ~~dispatcher by `transcript_path`~~
      attribution by the pidfile's directory, §3). Done when an approval
      from a directory account's session reaches the phone. *(The plugin and
      its token are shared, so a directory account's held approval takes the
      same path as the Default's; the live check is the maintainer's.)*
- [x] **P2.5 Background jobs per home** (`jobs/`, `claude logs/stop/respawn/
      rm` with the environment).
- [x] **P2.6 UI: badges, filters, fleet per account, Inbox and Agents.**
      *(Chips on Inbox cards, Agents groups and Services' background jobs —
      a background task shows its session's account, so no
      `BackgroundTask.account` field is served; account filters on Sessions
      and Inbox; per-account counts under the fleet summary. None of it
      shows with only the Default.)*

### Phase 3 — move and copy

- [x] **P3.1 Transfer engine** (`claude/transfer.rs`): session and project,
      copy and move, every precondition, undo copy, atomic `.claude.json`
      edit, verification, index rescan, audit. Done when the §9 engine tests
      pass, including every refusal.
- [x] **P3.2 Routes** `/data/move`, `/data/jobs/{id}`, `/data/jobs/{id}/undo`
      (`DesktopOnly`), the job model.
- [x] **P3.3 "Continue on" for directory accounts** uses copy-then-fork
      (closes the gap left in P1.5).
- [x] **P3.4 UI: move/copy dialog with dry run and Undo.** *(From the
      session menu and the Info tab's folder row, desktop only; the Accounts
      page lists recent moves and copies with their Undo, also after a
      restart.)*

### Phase 4 — backup

- [x] **P4.1 Backup engine** (`claude/backup.rs`): walk, exclusions, session
      archives, config bundle, encrypted index, incremental rule, live-skip,
      progress job. Done when the round-trip tests pass and a 6.7 GB
      directory's second run archives nothing. *(The incremental rule is tested;
      the 6.7 GB run is the maintainer's.)*
- [x] **P4.2 Restore** into a chosen account, with the §5.9 preconditions and
      trust opt-in.
- [x] **P4.3 Routes and schedule** (`/backup*`, `everyHours`, idle rule,
      passphrase in the Keychain, iCloud/Drive folder suggestions).
- [x] **P4.4 UI: Backup & sync page** (without pull). *(A generated
      passphrase is shown once and must be confirmed as written down; the
      restore dialog says which sessions Claude Code would prune again before
      it starts, §11.7.)*
- [x] **P4.5 Docs:** a section in REMOTE.md or a new `docs/BACKUP.md` with the
      folder advice ("Keep Downloaded" on iCloud, Drive streaming).

### Phase 5 — carry data between laptops

- [x] **P5.1 Pull** with the §5.11 rules, conflicts surfaced, `pullOnLaunch`.
- [ ] **P5.2 Live test** across the two Macs, recorded in the checklist with
      the date and CLI version. *(The maintainer's: it needs both Macs, a real
      cloud folder and real accounts — see "Checks that need real accounts"
      below.)*
- [x] **P5.3 UI: Pull… with the session list and conflicts.**

### Checks that need real accounts

Everything above runs against the fake CLI, a fake bridge host and temp
directories. These need the maintainer's Macs, real sign-ins and a real cloud
folder, and spend a little plan usage:

1. **A token account** (P1.3, P1.5): Settings → Accounts → Add → *Shared data,
   another account* → *Sign in to get a token*; sign in in the browser with the
   second subscription. The account shows *Token · signed in*. On a session of
   the Default: *Continue on…* the new account; the fork answers, and the plan
   meter of the new account moves after its first turn.
2. **A directory account** (P1.3, P2.1, P2.4): Add → *Separate directory* →
   `~/.claude-work` → *Sign in now*; the account shows its email. Install the
   hooks plugin for it, start `CLAUDE_CONFIG_DIR=~/.claude-work claude` in a
   terminal: the session is listed with the account's chip, and an approval it
   asks for reaches the phone.
3. **A terminal session with the token** (P2.2): `CLAUDE_CODE_OAUTH_TOKEN=<the
   token> claude` with the plugin installed: after its first tool call the
   session shows the token account's chip.
4. **The bridge** (P1.7): start the API with the token account as its default;
   `curl -sS http://127.0.0.1:5612/chat -d '{"message":"Which model are you?"}'`
   answers, and the conversation's chip names the account.
5. **Backups** (P4.1, P4.3): set the iCloud folder, generate a passphrase, *Back
   up now*; a second *Back up now* archives nothing; the archives open with
   `age -d -i key.txt …` as docs/BACKUP.md shows.
6. **Two Macs** (P5.2): on the second Mac set the same folder and passphrase,
   *Pull…*; continue a conversation on each Mac and pull again: one conflict
   file, no data lost. Record the date and the CLI version here.

---

## 11. The review's answers (2026-10-21)

All seven proposals stand, with one addition to 7. They are part of the design
now; the questions are kept below for the reasoning.

| # | Decision |
|---|---|
| 1 | Both: capture `setup-token` first (D3), paste as the fallback. |
| 2 | Ask once per account pair before copying a session for "continue on", and remember the answer (`copyConsent` in the registry, revocable on the Accounts page). |
| 3 | Generated passphrase, shown once, with "use my own" available. |
| 4 | The config bundle carries `.claude.json` whole (it is encrypted). |
| 5 | Trust flags are never restored or copied without the opt-in, even on the same machine. |
| 6 | Token-account attribution of foreign sessions is "unknown" on Windows. |
| 7 | Restore warns about pruning: a session older than the target account's `cleanupPeriodDays` (default 30) is marked "Claude Code will delete this again at its next cleanup unless `cleanupPeriodDays` is raised", and the restore dialog says so before it starts. |

### The questions as asked

1. **Token capture.** Should Mother Claude run `claude setup-token` itself and
   capture the token (nicer; depends on P1.1), or only accept a pasted token?
   Proposal: both, capture first.
2. **Directory accounts and "continue on".** Copy the session automatically
   before forking (one click, writes under the target directory), or ask each
   time? Proposal: ask once per account pair, remember the answer.
3. **Passphrase.** Generated and shown once (stronger, must be written down)
   or user-chosen? Proposal: generated, with "use my own" available.
4. **What the config bundle includes.** `.claude.json` whole (it has MCP
   secrets — encrypted anyway) or with `mcpServers[*].env`/`headers` stripped?
   Proposal: whole; the archive is encrypted and the point of a restore is to
   get the MCP servers back.
5. **Trust flags on restore/pull.** Proposal: never without the opt-in, even
   on the same machine.
6. **Windows.** Token-account detection for foreign sessions is "unknown"
   there (no environment read). Acceptable for the first release?
7. **Pruning.** The CLI prunes transcripts after 30 days; a restore can bring
   an old session back, which the CLI will prune again. Fine, or should
   restore warn?

---

## 12. Rough sizing

Working days, for one person who knows the codebase; the number is a guess
for planning, not a promise.

| Phase | Size | Notes |
|---|---|---|
| 0 | 2 | mostly tests and rules |
| 1 | 6–8 | the core value; P1.6 and P1.7 are the larger halves |
| 2 | 5–7 | `HomeSet` touches 30 files; the matcher needs the live test |
| 3 | 3–4 | the engine is small; the preconditions are the work |
| 4 | 4–5 | plus a soak with the real iCloud folder |
| 5 | 4–6 | plus the two-Mac test |

Phases 1 and 2 could be split: ship 1 (token accounts, continue-on, usage
per account) first — it needs none of 2 — and 2 as the following release.

---

## 13. Moving data anywhere, and an API server per account (2026-10-21)

Asked on 2026-10-21: bulk-move conversations and the rest of an account's
data between accounts, between the shared folder and an account, and between an
account and cloud storage (iCloud Drive, Google Drive, any other), in either
direction; run the API under a specific account, or several API servers at
once, each with its own account; and the same through remote access. This
section is the design; §13.8 is its checklist.

### 13.1 Words

- **Location**: where data lives. Either an **account** — its config
  directory: a directory account's own, or the **shared folder**, the Default's
  directory, which the Default and every token account use — or a **storage**.
- **Storage**: a folder that holds encrypted **sets** (§4.3, docs/BACKUP.md).
  The backup folder is the first storage (id `backup`); the user may add more
  (another cloud's folder, a NAS, an external disk), ids `stor_` + 12 hex.
  Every set in every storage opens with the one backup passphrase (§4.2).
- **Set**: `<storage>/<name>/`. A set named after an account is that
  account's backup (backed up, pulled, scheduled as §5.10–5.11). A set no
  account is named after ("Shared", "Archive 2026") is never backed up, pulled
  or scheduled: only Move data reads and writes it.
- **Items**: conversations (by id); projects (by folder: every conversation
  of it, its prompt-history lines and, with the trust opt-in, its
  `.claude.json` entry); everything of an account; and **customizations**:
  `memory` (`CLAUDE.md`), `settings` (`settings.json`), `skills`, `agents`,
  `commands`, `outputStyles` (`output-styles/`), `keybindings`, `mcpServers`
  (the user-scope `mcpServers` of `.claude.json`) and `history`
  (`history.jsonl`). Plugins are never copied: install them in the account.

### 13.2 What each direction does

| From → To | an account in another directory | an account in the same directory | a storage |
|---|---|---|---|
| **an account** | move or copy (§5.9), any number of items in one job | **assign**: nothing moves; the conversations are recorded as the target account's (`account-sessions.json`), so they show, count and continue under it. Copy is refused (`same_directory`): it would be the same conversation | **copy**: back the conversations up into the set (a push of just those). **Move**: the same, then each one whose copy in the set verified is removed here (into the undo directory) and its index entry marked *archived* |
| **a storage** | **copy**: restore (§5.10) into the account. **Move**: restore, then remove the entries from the set (their archives kept seven days, for the undo) | as left | copy or move between sets: decrypt, encrypt to the target set's key, verify (P2) |

Customizations are only ever copied (a move is refused), between accounts in
different directories.

### 13.3 Rules

1. **One request, one plan, one job per source.** A request names a source
   and a target location and any mix of items; the dry run lists everything
   it would do. With `items.sessions` only, `from` may be left out: each
   conversation goes from where it is, one job per source location.
2. **Bulk skips, single refuses.** With the new `items` body, a conversation
   that runs, that the target already has, or that cannot be read is listed in
   `skipped` with its code and the rest goes ahead. The old body (`what`:
   `session` | `project`) still refuses with the code, exactly as before.
3. **The preconditions of D4 and §5.9 hold for every write** into a config
   directory: desktop principal, `directory_in_use`, `config_locked`, checked
   again right before writing, undo copy first, atomic writes, audit line.
   Reading a storage needs the passphrase (`no_passphrase`) and an
   authenticated set (`backup_unauthenticated`, `set_key_missing`).
4. **Archived** entries (`archived: {at, machine}` in the set's index): a
   pull never brings an archived conversation to a machine that does not have
   it; a machine that has it keeps it, and its push of a changed copy replaces
   the entry, which clears the mark. Restore lists and Move data show it as
   *in storage only*.
5. **A push of some conversations** updates only their `seen` entries; the
   machine's other `seen` entries stay as they were.
6. **Undo.** Between directories: §5.9. Assign: the previous account of each
   conversation back. Copy to storage: none (it is a backup). Move to storage:
   the local files back; the set keeps its copy and the archived mark is
   cleared. Copy from storage: as a restore. Move from storage: the set's
   entries first (only where the set has no newer entry for the id since;
   their archives are kept seven days for this), then the restored files as a
   restore's undo; if the set cannot take them back, nothing is undone.
7. **Customizations**: each category adds what the target lacks; with
   `replaceExisting`, what it has is replaced (the old copy into the undo
   directory). `mcpServers` merge by server name, `history` by `(timestamp,
   display)` as §5.9. A symbolic link (a skill linked from a repository) is
   recreated as a link to the same absolute target, never followed. The dry
   run warns when the copied settings choose how Claude signs in (an `env` with
   one of `ACCOUNT_OVERRIDE_ENV`, `apiKeyHelper`, `forceLoginMethod`,
   `forceLoginOrgUUID`) or run commands (`hooks`, `statusLine`), and names MCP
   servers only by name: their `env` and `headers` values are never served.
8. **Storages**: a passphrase change re-keys every set in every storage
   (every one opened first, as docs/BACKUP.md says for the backup folder); the
   schedule and pull on launch use the backup folder only; removing a storage
   forgets it and never deletes its files. A storage is an absolute folder, not
   inside an account's directory, not another storage.
9. **Goal 7 still holds**: with only the Default account, no storage besides
   the backup folder and no API server besides the main one, every response
   is what it was before this section.

### 13.4 API for moving data

`POST /api/data/move` (`DesktopOnly`):

```jsonc
{
  "from": "acc_…" | {"account": "acc_…"} | {"storage": "backup" | "stor_…", "set": "Work"},
  "to":   "acc_…" | {"account": "acc_…"} | {"storage": "backup" | "stor_…", "set": "Work"},
  "items": {
    "sessions": ["<id>", …],        // any of these four
    "projects": ["/abs/folder", …],
    "all": true,                    // every conversation of `from`
    "customizations": ["memory", "settings", "skills", "agents", "commands",
                       "outputStyles", "keybindings", "mcpServers", "history"]
  },
  "mode": "move" | "copy",
  "trust": false,                   // carry the projects' trust and entries
  "replaceExisting": false,         // customizations: replace what the target has
  "dryRun": false
}
```

A bare string is an account id. A storage target's `set` defaults to the set
of the source account's directory (a token account's is the Default's).
"Everything of" a shared-folder account is the conversations attributed to it
(§5 attribution); with only the Default, the whole folder.

Dry run → `200 {plan}`; otherwise `202 {jobs: [JobView…], job: JobView}` (`job`
is the first, for the old client). The plan, camelCase, lists capped at 1000
entries each with `truncated: true` beyond:

```jsonc
{
  "from": <location>, "to": <location>, "mode": "move", "jobs": 1,
  "sessions": [{"id", "cwd", "title", "files", "bytes", "from": <location>,
                "action": "move" | "copy" | "assign" | "archive" | "restore"}],
  "skipped": [{"id"?, "cwd"?, "item"?, "code", "message"}],
  "projects": ["/abs/folder"], "addsProjectEntries": [], "removesProjectEntries": [],
  "historyLines": 0,
  "customizations": [{"item": "skills", "name": "review", "action": "add" | "replace" | "skip" | "merge", "reason"?}],
  "warnings": ["…"],
  "files": 0, "bytes": 0, "writesShared": false
}
```

The old body's dry run keeps its old plan shape. A job view adds
`fromStorage` / `toStorage` (storage ids) and `set` beside `from` / `to` /
`account`; its `result` names what it did: `moved`, `copied`, `assigned`,
`archived` or `restored` (ids), `skipped`, `customizations`.

Storages, all `DesktopOnly` to change, `Authed` to read:

- `GET /api/backup` adds `storage: [{id, label, path, kind: "icloud" | "gdrive"
  | "onedrive" | "dropbox" | "folder", primary, available, sets: [{name,
  account?, exists, keyMissing?, sessions?, archived?}]}]`, the backup folder
  first (`primary: true`).
- `POST /api/backup/storage {path, label?}` → `201` the storage;
  `PATCH /api/backup/storage/{id} {label}`; `DELETE /api/backup/storage/{id}`.
- `GET /api/backup/index?storage=<id>&set=<name>`, beside the existing
  `?account=`; its session items gain `archived`.

### 13.5 API servers

**Model.** An API server is `{id, name, port, defaults}`, `defaults` being
§5.5's `BridgeDefaults` plus `accountOnly`. The **main** server (id `main`) is
the one there always was: port 5612 (`MOTHER_CLAUDE_BRIDGE_PORT`), `/v1` on the
main server, its defaults not saved, exactly as before. Others are added on
the API screen and saved in `<app data dir>/api-servers.json` (0600), each with
a loopback port of its own (1024–65535, none another server's, the main
server's or the tunnel listener's). None listens until it is started; the
"open to tunnels" choice is made at each start and never saved.
`MOTHER_CLAUDE_BRIDGE_AUTOSTART=1` starts the main server at launch as before,
`=all` every saved one too (each closed to tunnels);
`MOTHER_CLAUDE_BRIDGE_ACCOUNT` (an id or a name) and
`MOTHER_CLAUDE_BRIDGE_ACCOUNT_ONLY=1` set the autostarted main server's
account.

**Account.** `defaults.account` unset: automatic, as §5.5. Set, with
`accountOnly` false: the default for new conversations, a request may name
another (the main server's behaviour so far). Set, with `accountOnly` true
(what a server made for an account starts with): the server runs **only** as
that account. New conversations run under it; a request naming another account
is refused (`403 account_not_served`); a conversation, operation or runtime
request of another account is not listed, read, resumed or answered through it
(`404`, as if it did not exist); `/auth` and `/auth/login` are that account's
sign-in. One account's API can then be put behind a tunnel without opening the
others.

**One sidecar host** serves every server (each query carries its account's
environment, §5.5); conversations are one store, each recording the server
that made it (`server`, left out for the main one).

**Reaching them.** Each server answers on `http://127.0.0.1:<port>` at `/` and
`/v1`. Every server is also reachable through the main ones: under
`/servers/<id>/` and `/servers/<id>/v1/` on the main API port (one tunnel
reaches every server) and on the main server's LAN mount
(`https://<mac>:6725/servers/<id>/v1`, a token always, as `/v1` there). A
request through such a prefix gets the target server's rules — its token
policy, its account, whether it is started — while the port it arrived on
decides whether a tunnel points at it. The tunnel listener (6726) still serves
no `/v1`.

**Tunnels per server.** Detection tracks every server's port: a tunnel to a
server's port turns that server's tokenless mode off while it lasts, as it does
for the main port, and the Remote page and the API screen name the server a
tunnel points at.

**Who may.** As `POST /api/bridge/start` today: anyone authenticated to the
dashboard may add, start, stop, change and remove servers; the dangerous parts
keep their rules (a permission mode that skips approvals and opening to
tunnels: desktop only; an untrusted default folder: the dangerous right).
Removing an account a running server uses is refused
(`409 account_used_by_bridge`); a stopped server whose account was removed
says so and cannot start (`account_not_found`) until another is chosen.

**Dashboard API.**

- `GET /api/bridge` as before for the main server, plus `servers:
  [ServerStatus…]` (main first) whenever a server besides the main one exists.
  A `ServerStatus` has the main payload's per-server fields (`id`, `name`,
  `running`, `port`, `url`, `prefix`, `startedAt`, `requireToken`,
  `tokenPolicy`, `exposedByTunnel`, `tunnelUrl`, `defaults`, `activeThread`,
  `threads`).
- `POST /api/bridge/servers {name, port?, defaults?}` → `201` its status (not
  started; `port` defaults to the first free from 5613); `PATCH
  /api/bridge/servers/{id} {name?, port?, defaults?}` (a running server is
  restarted with them); `DELETE /api/bridge/servers/{id}` (stopped first);
  `POST /api/bridge/servers/{id}/start {defaults?, openToTunnels?}`, `…/stop`,
  `…/active-thread {threadId}`. `/api/bridge/start`, `/stop` and
  `/active-thread` stay the main server's. An id is a slug of the name
  (`[a-z0-9-]{1,32}`, made unique; `main` is taken).
- The bridge's own `/metadata` and `/capabilities` carry `server: {id, name,
  account?: {id, name}, accountOnly}`; the OpenAPI document's `servers` names
  the base URL the request came through.

*Changed while building (P7):*

- **Wire.** `accountOnly` is in `defaults` (left out when false), needs
  `defaults.account` (`422 invalid_field`, field `accountOnly`) and is
  accepted by `POST /api/bridge/start` too. A `ServerStatus` also has
  `docsUrl`, `consoleUrl` and, only when `defaults.account` names a removed
  account, `accountMissing: true`; `prefix` is `""` for the main server and
  `/servers/<id>` for the others (the LAN form is that plus `/v1`). The main
  payload itself gains no `id`/`name`/`prefix`: the main server is
  `servers[0]` (`main`, "Main"), and with other servers present its `threads`
  are the main server's only. `POST /api/bridge/servers` → `201`; `PATCH`,
  `…/start`, `…/stop`, `…/active-thread` → `200` that server's status;
  `DELETE` → `204`; an unknown id `404 server_not_found`. `main` answers on
  `…/start`, `…/stop`, `…/active-thread` too; `PATCH`/`DELETE` of it are
  `409 main_server_fixed`. A start with `defaults` saves them for an added
  server; without, its saved ones apply; `openToTunnels` left out is off for
  an added server (`MOTHER_CLAUDE_BRIDGE_REQUIRE_TOKEN=never` is the main
  server's starting position only), so a stopped added server shows the
  configured policy closed to tunnels.
- **Refusals.** A name is 1–40 characters, no control characters
  (`422 invalid_field`, field `name`), unique in any case including "Main"
  (`409 server_name_taken`). A port outside 1024–65535 is `422 invalid_field`
  (field `port`); one another server, the main API, the app or the tunnel
  listener has is `409 port_unavailable` (field `port`, plus `server` when a
  server holds it); a bind failure at start is `409 port_unavailable` as
  before. An account no one has in a body is `422 invalid_field` (field
  `account`), as the main start answers; starting a server whose *saved*
  account was removed is `409 account_not_found`. `account_used_by_bridge`
  carries `server` and `serverName` when it is not the main server. The main
  server, stopped, still forgets a removed account (and `accountOnly` with
  it), as before. Port `0` — any free port at each start — exists only for
  tests and embedders (`control::add_server`); the dashboard refuses it.
- **Reaching them.** On the main API port `/servers/<id>/` serves the target at
  `/` and `/v1`; the main app port serves only `/servers/<id>/v1/`, never
  through a tunnel (the route policy and the tunnel listener treat
  `/servers/*` as `/v1`). A request names its server with a `Mount` and gets a
  `MountPolicy` (server, token policy, arrival port, whether a tunnel points at
  that port, the server's defaults); a router built without a port (tests,
  embedders) counts a tunnel to any API port.
- **Tunnels.** `DetectedTunnel.target` is `bridge` for every API server's
  port, saved servers included, with `server: "<id>"` when it is not the main
  one; `GET /api/remote`'s `bridge` adds `servers: [{id, name, port, running,
  exposed}]` once another server exists; warnings and notices name the server.
- **`accountOnly`.** Besides the routes above, `GET /events` filters by each
  event's `params.thread_id` (one without is left out), `GET /threads`'
  `stored` reads the account's directory and, in the Default's (shared with
  the token accounts), keeps only that account's conversations; `/auth` adds
  `account: {id, name}`, and `/auth/login` signs the account in (a token
  account: `422 login_failed`, it gets a new token on the desktop);
  `/metadata`'s `claude_home` is the account's directory; `/models` reads the
  account's settings. `/messages` and `/files` use `ANTHROPIC_API_KEY` on every
  server. The `server` object of `/metadata` and `/capabilities` is left out
  while only the Default account and only the main server exist (rule 9).
- A removed server's conversations stay, as the main server's. Autostart with
  an account no one has, or `ACCOUNT_ONLY` without an account, starts no main
  server rather than the wrong one; `all` drops a saved server's nominated
  conversation that has gone since, rather than keeping the server down.

### 13.6 UI

1. **Sessions → Select** (desktop): checkboxes on the cards, *Select all
   shown*, and a bar *Move or copy N…* that opens Move data with them.
2. **Move data** (replaces the Move/copy dialog; opened from a session's or
   project's menu, an account, a storage or the Select bar): *From* and *To*
   list the accounts (directory, or the shared folder with the accounts that
   use it) and the storages (with their sets, and *New set…*); *What*:
   everything, projects, conversations (each a searchable checklist) and
   customizations; *Move* or *Copy*; the trust and replace options; the dry
   run (what goes, what is skipped and why, warnings); then progress and Undo.
3. **Accounts**: *Move or copy data…* on each account.
4. **Backup & sync**: a *Storage* section — the backup folder first, *Add
   storage…* (the same suggestions as the folder), and each storage's sets
   with *Bring back…* and *Move here…*.
5. **API**: the servers, main first, each with its state, port, account and
   tunnel; *Add API server* (name, account, *Only this account*, port, the
   defaults); the panel shows the selected server, with its addresses (its
   port, through the main port, on the network).
6. **Phones** (LAN or tunnel) get all of it, as §13.9 says: Select, Move
   data, Undo, the storages, Back up now, Restore, Pull, the backup settings,
   account defaults and copy consent, and the API servers. What needs the
   dangerous right (a folder's trust, settings and MCP customizations) is
   shown disabled with the reason on a device without it; what stays on the
   desktop (the passphrase, adding or signing in accounts, tokens, the hooks
   plugin, opening the API to tunnels) says so. Every layout works at 360 px.

### 13.7 Documents to change

`CLAUDE.md` (D4's list: customizations; the bridge paragraph: several servers;
the accounts row), docs/BACKUP.md (storages, archived, moving to and from),
docs/BRIDGE.md (servers, prefixes, `accountOnly`, the env variables),
docs/REMOTE.md (tunnels per server), SECURITY.md, README.md, `.env.example`.

### 13.8 Checklist

**Phase 6 — move data anywhere**

- [ ] P6.1 Bulk transfer: the `items` body, skips with reasons, several
  projects' entries and history in one job, `from` left out → a job per
  source; the old body unchanged.
- [ ] P6.2 Assign between accounts that share a directory, with its undo.
- [ ] P6.3 Storages: settings, routes, the view, sets not named after an
  account, the index by storage and set, re-keying every storage.
- [ ] P6.4 Account → storage: copy (a push of some conversations) and move
  (archive, local removal, undo); pull skips archived entries.
- [ ] P6.5 Storage → account: copy (restore from any set into any account)
  and move (restore, then remove the entries; undo puts them back).
- [ ] P6.6 Customizations, with their undo and warnings; D4 in CLAUDE.md.
- [ ] P6.7 Storage → storage (P2).
- [ ] P6.8 UI: Select, Move data, the account action, the Storage section, job
  titles, the mock backend.
- [ ] P6.9 Documents (§13.7).
- [ ] P6.10 From the phone (§13.9): the guards, the dangerous parts, the
  desktop notice; the UI on phones.

**Phase 7 — API servers per account**

- [x] P7.1 The servers: registry, `api-servers.json`, the main one as before.
- [x] P7.2 Per server: listener, defaults, active conversation, token policy,
  started gate.
- [x] P7.3 `accountOnly` on every bridge route; `/auth` per account.
- [x] P7.4 `/servers/<id>` on the main API port and the LAN mount.
- [x] P7.5 Tunnel detection per port; the Remote page names the server.
- [x] P7.6 The dashboard routes, `servers` in `GET /api/bridge`, the account
  removal rule.
- [x] P7.7 Autostart: `all`, `MOTHER_CLAUDE_BRIDGE_ACCOUNT(_ONLY)`.
- [ ] P7.8 UI: the server list, adding and removing, the panel per server,
  the addresses, the mock backend.
- [ ] P7.9 Documents (§13.7).

### 13.9 From the phone (D9 revised, 2026-10-21)

"Remote" in the request means the dashboard on the phone, on the LAN or
through a tunnel (ngrok and the like, §2.8 of the remote design): everything in
§13 has to work there. Who may do what:

| Action | Desktop | Paired device | Device with the dangerous right |
|---|---|---|---|
| Move or copy data in any direction, assign, archive, restore, pull, Back up now, undo a data job | yes | yes | yes |
| Add, rename or remove a storage; backup folder, schedule, pull on launch | yes | yes | yes |
| Account defaults (new sessions, API, per project), copy consent, rename or recolour an account | yes | yes | yes |
| Add, start, stop, change or remove an API server | yes | yes | yes |
| Carry a folder's trust (`trust: true`); copy the `settings` or `mcpServers` customizations | yes | no (`403 dangerous_blocked`) | yes |
| Set or generate the backup passphrase; add, remove or sign in an account; tokens; the hooks plugin installer; opening an API server to tunnels; a permission mode that skips approvals | yes | no (`403 desktop_only`) | no |

- A **paired device** is the device principal (a `mcd_` token, LAN or
  tunnel). The local-script token stays off the data and account routes, as
  it is off admin routes today; API servers keep today's rule for starting the
  main one (any authenticated caller, local scripts included).
- Every such action a device starts is audited with the device, and the
  desktop shows a notice naming the device and what it did, as it does when a
  device uses the dangerous right.
- The D4 guarantees do not depend on who asks: the in-use checks, the undo
  copy, atomic writes and the audit line are the same for a phone.
