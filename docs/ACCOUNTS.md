# Accounts: several Claude accounts in one dashboard

> **Status: approved 2026-10-21, being built.** The design and the to-do list
> for the work: tick the boxes in §10 as tasks land, and change the design here
> first when the implementation finds it wrong. The review's answers are in
> §11; §3 records where building it changed a decision.
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

**D9 — Account choice is not a dangerous action; account administration is.**
Picking an account for a new session or a bridge conversation is allowed to
any principal that may start sessions (it is the user's own account either
way). Adding, removing or signing in an account, entering a token, moving
data, restoring, pulling and changing backup settings are `DesktopOnly`.

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
set.json                      {"version":1,"account":"Work","createdAt":…,"machines":[{"host":"marcus-mbp","lastPush":…}]}
check.age                     the string "mother-claude" encrypted with the passphrase — "is this the right passphrase?" without downloading anything else
index.json.age                [{"id":…,"cwd":…,"transcriptBytes":…,"newestMtime":…,"lastActivityAt":…,"host":…,"pushedAt":…,"archiveSha256":…}, …]
sessions/<session id>.tar.gz.age
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
`BridgeThread.account`, `BackgroundTask.account`; `GET /usage` keeps its shape
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
- Account administration, data moves, restore, pull, backup settings:
  `DesktopOnly`. Account choice: any principal that may start sessions.
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
- [ ] **P1.8 UI: Accounts page, picker, meter switcher, limit offer.**
- [ ] **P1.9 Docs:** REMOTE.md, README, .env.example, SECURITY.md (§8).

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
- [ ] **P2.6 UI: badges, filters, fleet per account, Inbox and Agents.**

### Phase 3 — move and copy

- [ ] **P3.1 Transfer engine** (`claude/transfer.rs`): session and project,
      copy and move, every precondition, undo copy, atomic `.claude.json`
      edit, verification, index rescan, audit. Done when the §9 engine tests
      pass, including every refusal.
- [ ] **P3.2 Routes** `/data/move`, `/data/jobs/{id}`, `/data/jobs/{id}/undo`
      (`DesktopOnly`), the job model.
- [ ] **P3.3 "Continue on" for directory accounts** uses copy-then-fork
      (closes the gap left in P1.5).
- [ ] **P3.4 UI: move/copy dialog with dry run and Undo.**

### Phase 4 — backup

- [ ] **P4.1 Backup engine** (`claude/backup.rs`): walk, exclusions, session
      archives, config bundle, encrypted index, incremental rule, live-skip,
      progress job. Done when the round-trip tests pass and a 6.7 GB
      directory's second run archives nothing.
- [ ] **P4.2 Restore** into a chosen account, with the §5.9 preconditions and
      trust opt-in.
- [ ] **P4.3 Routes and schedule** (`/backup*`, `everyHours`, idle rule,
      passphrase in the Keychain, iCloud/Drive folder suggestions).
- [ ] **P4.4 UI: Backup & sync page** (without pull).
- [ ] **P4.5 Docs:** a section in REMOTE.md or a new `docs/BACKUP.md` with the
      folder advice ("Keep Downloaded" on iCloud, Drive streaming).

### Phase 5 — carry data between laptops

- [ ] **P5.1 Pull** with the §5.11 rules, conflicts surfaced, `pullOnLaunch`.
- [ ] **P5.2 Live test** across the two Macs, recorded in the checklist with
      the date and CLI version.
- [ ] **P5.3 UI: Pull… with the session list and conflicts.**

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
