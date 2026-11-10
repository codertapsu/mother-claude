# S2 — moving a dirty working tree with git (Mac half)

Phase 0 spike S2 of [docs/PLAN-MACHINES.md](../../../docs/PLAN-MACHINES.md)
§5.10. Throwaway code: [`roundtrip.mjs`](roundtrip.mjs) (Node ≥ 20, git only,
no dependencies). Raw results: [`results-darwin-arm64.json`](results-darwin-arm64.json).

**Verdict: §5.10 works — snapshot, both transfers, both landings, undo and
bring-back round-trip a dirty tree exactly — once seven changes are made
(F1–F7 below).** 132 of 132 matrix runs behaved as designed on the Mac
(8 skipped: Git LFS is not installed here). The Windows half is still to run.

## Environment

| | |
|---|---|
| Machine | MacBook, macOS (Darwin 27.2.0), arm64 |
| git | 2.54.0 (Apple Git-157); **no git-lfs** |
| Node | 24.18.0 |
| File system | APFS, **case-insensitive**, symlinks and exec bits supported |
| Isolation | matrix runs with `GIT_CONFIG_GLOBAL=<empty>`, `GIT_CONFIG_NOSYSTEM=1`, a fixed identity; nothing touched a real repo |

## How to run

```bash
node spikes/machines/code/roundtrip.mjs matrix            # everything, ~3 min here
node spikes/machines/code/roundtrip.mjs matrix --only mix,behind --keep --verbose
```

The real two-machine commands (`haves`, `snapshot`, `apply`, `verify`,
`undo`, `demo`, `dirty`) are in the script's header and in "Windows half"
below. They use each machine's own git config, minus hooks and signing.

## What the script does (§5.10 as built)

- **Snapshot (S)**: two copies of `.git/index` — `write-tree` of one is the
  staged tree, `add -A` + `write-tree` of the other the working tree
  (untracked in, ignored out). Two commits, both with parent `base`:
  `index` (the staged tree) and `snap` (the working tree), kept as
  `refs/mother-claude/moves/<session>/<epoch>/{snap,index}`. A `git status`
  cross-check proves the working-tree tree (F1). The source's HEAD, refs,
  index bytes, status and every file's bytes are compared before and after:
  untouched in every run.
- **Transfer**: a bundle of the two refs `--not` the commits T reported
  having (default), or a push to hidden refs
  `refs/mother-claude/<machine>/<session>/{snap,index}` on a remote (a local
  bare repo stood in for GitHub), fetched by T and then deleted from the
  remote.
- **Landing in the checkout** (default, T clean): branch rules — same,
  fast-forward, create, unborn, detach; a branch with commits the base
  lacks, or checked out in another worktree, **stops** and offers a
  worktree. Then `read-tree -m -u <T's HEAD> base`, `read-tree -m -u base
  snap`, refs, `read-tree <staged tree>`, `update-index --refresh`, and
  submodule commits checked out with plumbing when T has them. Undo refs
  `refs/mother-claude/undo/<time>/{head,branch}`.
- **Landing in a worktree**: `worktree add --no-checkout` (`-b
  mother-claude/<session>` at base; `--detach` for a detached source;
  `--orphan` for an unborn one) in a scratch "app data" dir, then
  `read-tree -m -u <empty tree> snap` and `read-tree <staged tree>`. The
  checkout is never touched (proved per run).
- **Verify (T)**: HEAD and branch, index tree = staged tree, working-tree
  tree = snapshot tree, `git status` equal to the source's, and every file
  byte for byte, each difference classified (`eol`, `filter`, `filemode`,
  `symlink-as-file` expected; `content`, `missing`, `type` failures;
  `submodule`, `case`, `ita` known limitations).
- **Undo (T)**: back to T's previous HEAD, branch and files (or its parked
  state, F7); untracked files from the move removed only while unchanged.

## Matrix (macOS)

`pass` = exact; `stop` = refused by design, worktree offered; `limit` = a
known limitation, classified (see "Known limitations"). Order: bundle →
checkout | bundle → worktree | remote → checkout | remote → worktree.

| Case | Result | Notes |
|---|---|---|
| mix: staged, staged-then-changed, unstaged, deleted (staged and not), renamed (`git mv` and on disk), new staged, untracked, ignored | pass ×4 | undo restores T exactly; ignored files never travel |
| binary changed + new binary | pass ×4 | |
| exec bit: unstaged chmod, staged chmod, new executable | pass ×4 | |
| … target `core.fileMode=false` (as Windows) | limit ×4 | `filemode`: the unstaged chmod and the new executable lose +x; the staged chmod survives |
| spaces, non-ASCII, an NFD name | pass ×4 | |
| a tracked file matching `.gitignore` | pass ×4 | it travels; ignored ones do not |
| LF source → target `autocrlf=true` | pass ×4 | raw `eol` only (CRLF on T), trees and status equal |
| CRLF working tree (`autocrlf=true`) → target `autocrlf=false` | pass ×4 | raw `eol` only |
| `* text=auto eol=lf`, target `autocrlf=true` | pass ×4 | byte-identical |
| CRLF committed without attributes, changed; target `autocrlf=true` | pass ×4 | git keeps CRLF blobs as CRLF; no spurious change |
| target branch behind the base | pass ×4 | fast-forward; undo restores the old tip |
| target branch diverged | stop, pass, stop, pass | |
| target checkout dirty | stop, pass, stop, pass | T's own change untouched; remote: hidden refs kept while the move is pending (F6) |
| target on another branch | pass ×4 | T switched to the source's branch; undo switches back |
| unborn source, empty target | pass ×4 | worktree via `--orphan` |
| detached source | pass ×4 | T detached at the base; its branch untouched |
| submodule moved to a commit T has | pass, limit, pass, limit | checkout: submodule checked out by plumbing; a new worktree has no initialized submodules |
| submodule moved to a commit T lacks | limit ×4 | needs F5 (otherwise the fetch itself fails) |
| tracked symlink retargeted + new untracked symlink | pass ×4 | |
| … target `core.symlinks=false` (as Windows by default) | limit ×4 | `symlink-as-file`: the untracked link arrives as a plain file; the tracked one keeps mode 120000 in the index |
| `git mv Readme.md README.md` | pass ×4 | |
| Readme.md → README.md on disk only | limit ×4 | `case`: invisible to git on a case-insensitive FS, not carried |
| `x/File.txt` + `x/file.txt` in one tree | pass ×4 | `case-collision` warning raised |
| names Windows cannot hold (`aux.txt`, `trailing-dot.`, `what?.txt`) | pass ×4 | `windows-names` warning raised |
| a 255-character path | pass ×4 | `long-path` warning raised |
| `git add -N` entry | limit ×4 | `ita`: arrives as untracked |
| target is a `--depth 1` clone; the change restores an old blob | pass ×4 | see F-notes |
| hooks diagnostic (hooks not overridden) | observe | see F3 |
| round trip S → T → work on T (edits, a commit) → back into S | pass ×4 | back lands on S's parked checkout (F7), fast-forward, verified; undo of the way back restores the parked state |
| … S edited while the session was away | pass ×4 | back is refused ("changes of its own") |
| large: 2,000 files / 50 MB | pass ×4 | timings below |
| Git LFS, objects carried / not carried | skipped | git-lfs not installed here; runs on Windows |

## Timings and sizes (this Mac)

| | |
|---|---|
| Small cases (median of 108 runs) | snapshot 0.40 s (incl. the spike's own before/after proof), apply 0.16 s, verify 0.12 s; bundle ≈ 680 B |
| Large: 2,000 files, 50 MB; 50 changed, 22 new (two 1 MB binaries), 5 deleted | snapshot 1.8 s, of which the product's part is trees 0.45 s + commits 0.05 s + bundle 0.07 s; the rest is the spike's own before/after proof (≈ 0.5 s each) and manifest (0.27 s) |
| Large bundle, T has the base | **2.06 MB** (vs a full bundle 23.2 MB, 0.14 s) |
| Large apply | fetch 0.25 s, to base 0.03 s, to snapshot 0.12 s, index 0.12 s |
| Large, remote mode (local bare repo) | push 0.12 s, fetch 0.21 s |
| Incremental move after S commits the moved work | **1.6 KB chained** vs 2.06 MB unchained (F6) |

## Changes §5.10 needs

**F1 — An index copy must keep the original's mtime, and the snapshot must
be checked against `git status`.** A plain copy is dated now; git trusts an
entry's stat data when the entry is older than the index file, so a
same-size file rewritten in the same second as the last index write looks
unchanged to `add -A` on the copy, and the snapshot **silently misses the
edit**. Measured through the script (`S2_PLAIN_COPY=1`): a plain copy
missed an unstaged edit in **6 of 60** snapshots, the mtime-kept copy in
**0 of 60**. The fix as built: copy, set its mtime to the original's (floored
to the second), then require that the paths `git status` reports as changed
in the working tree equal the paths that differ between the two trees; on a
mismatch, re-hash every file (index copy dated 1 s after the epoch, so every
entry is racy) and check again. Verify always uses the re-hash mode.

**F2 — The staged tree travels as a commit of its own.** `idx_tree` is not
reachable from `snap`, so neither a bundle nor a push carries it. Built as
an `index` commit (parent `base`, like `git stash`'s index commit); refs
`…/<epoch>/{snap,index}`, hidden refs `…/<session>/{snap,index}`.

**F3 — "No git hook runs (plumbing only)" is false.** With the repository's
hooks left on, the snapshot runs `post-index-change` (its writes to the
index copies) and `reference-transaction` (`update-ref`) on S, and the apply
runs `reference-transaction` (fetch, refs) and `post-index-change`
(`read-tree`, `update-index`) on T; a push runs `pre-push` unless `--no-verify`. Fix: every
git call with `-c core.hooksPath=<an empty directory>` (and `--no-verify`
on push); `worktree add --no-checkout` also avoids `post-checkout`. Keep that
directory private and check it is still empty before use: `git rev-parse
--git-path hooks` returns the override, so a tool that installs hooks that
way writes *into* it (this spike's own fixture did, and then every git call
ran them).

**F4 — Never read or write the real index.** `git write-tree` rewrote the
index file it read (cache-tree), and `git status` rewrote it unless
`GIT_OPTIONAL_LOCKS=0` (diagnostics: `writeTreeRewritesIndex: true`,
`statusRewritesIndex: true`, with the variable `false`). Both trees come from
copies, and every call runs with `GIT_OPTIONAL_LOCKS=0`.

**F5 — Fetches must not follow submodules.** By default a fetch that brings
commits changing a gitlink also fetches the submodule; when the submodule's
remote lacks the commit, **the whole fetch fails** ("not our ref"). Fetch
with `--no-recurse-submodules` (also `--no-tags --no-write-fetch-head
--no-auto-maintenance`), push with `--recurse-submodules=no`, and check
submodule commits separately (built: checked out with plumbing when the
target's submodule has them, else reported).

**F6 — Chain a session's snapshots.** A bundle's `--not <earlier
snapshot>` excludes nothing when the earlier snapshot is not an ancestor
(git marks only the boundary commits' trees uninteresting), so a second move
re-sent everything the first carried: 2,060,705 B. With the earlier
snapshots the target reports having added as extra parents (after `base`,
which stays the first parent): **1,613 B**. `--objects-edge-aggressive` does
not help. Remote mode: a move that stops before T fetches keeps its hidden
refs on the remote — the product deletes them when the move completes or is
cancelled.

**F7 — Bring back needs a "parked" rule.** After a move the source checkout
still holds the dirty state it sent, so the default landing ("only when
clean") refused every bring-back. Rule as built: a checkout whose HEAD is a
snapshot's base, index its staged tree and working tree its tree — for a
snapshot of *this session* it made or received — counts as clean for that
session; the landing starts from that tree (`read-tree <it>`, then
`update-index --refresh`, needed because a bare `read-tree` leaves no stat
data and the merge then refuses "not uptodate"). Any edit made meanwhile
makes it a normal dirty checkout: refused, worktree offered. Undo returns to
the parked state.

Smaller points, built and passing:

- **Clean** means `git status --porcelain -uall` is empty: untracked files
  count (`read-tree -u` would refuse to overwrite them, and they would join
  the moved tree); an operation in progress (merge, rebase, cherry-pick,
  revert, bisect) refuses on both sides.
- **The worktree landing's branch** (`mother-claude/<session>`) comes home as
  that branch: the round trip from a worktree switched S to it ("create")
  instead of fast-forwarding S's own branch. The move metadata should carry
  the home branch so bring-back lands on it. (Not built.)
- **A shallow target** worked with both transfers: the bundle excludes only
  objects of the boundary commits' trees, which a shallow clone has. A
  partial clone (`--filter=blob:none`) target was not tested: blobs it
  "has" through a commit may be absent locally and fetched lazily from its
  remote, which then needs the network.
- **Snapshot commits** use the machine's git identity (a stand-in only when
  none is set) and are never signed (`--no-gpg-sign`, `commit.gpgSign=false`):
  signing would prompt for a key.
- **Line endings**: every `autocrlf`/`eol` combination tried is tree-exact;
  only raw line endings differ, as each machine's settings dictate, and
  `git status` is the same on both machines.

## Known limitations (classified by verify, warned by the snapshot)

| Limitation | Snapshot warning | Advice for the preflight |
|---|---|---|
| Unstaged mode change, new executable → target with `core.fileMode=false` (Windows) loses +x | `unstaged-mode-change`, `untracked-exec` | stage the mode change (staged modes survive) |
| Untracked symlink → target with `core.symlinks=false` arrives as a plain file | `untracked-symlink` | commit or stage it; or Developer Mode on Windows |
| Submodule commit missing on the target; changes inside a submodule never travel; a new worktree has no submodules | `submodules` (lists changed ones) | push the submodule commit first |
| Case-only rename on disk (no `git mv`) on a case-insensitive FS is invisible to git | — (seen only by comparing on-disk names) | use `git mv` |
| `git add -N` entries arrive untracked | — | — |
| Paths differing only in case, names Windows cannot hold, paths > 200 chars | `case-collision`, `windows-names`, `long-path` | warn before moving to Windows |
| Git LFS: objects are not in the bundle | `lfs` | carried in the move dir (built, untested here) |

## Not covered on this Mac

Windows itself; Git LFS (no git-lfs here); a real GitHub remote (a local
bare repo stood in: GitHub's acceptance of `refs/mother-claude/*` pushes is
unverified); partial clones, sparse checkouts, split index, fsmonitor;
repositories over 1 GB.

## Windows results (2026-11-11)

Windows 11, Git for Windows 2.45.1, Git LFS 3.5.1, Node 24, NTFS,
Defender real-time scanning on. `matrix`: **132 of 132 as designed** (90
pass, 22 limits, 4 stopped by design, 4 expected errors from
`lfs-not-carried`; 12 skipped: exec bits and Windows-illegal names), raw
results in `results-win32-x64.json`. Two findings, fixed in the script:

**F8 — Long paths are dropped silently.** With `core.longpaths` unset (git's
default; the matrix's isolated config, and a fresh Git for Windows),
`add -A` meets a 255-character path with *"warning: … Filename too long"*,
exits 0, and leaves the file out; `git status` misses it the same way, so
F1's cross-check agreed and verify passed on a snapshot that had lost the
file. Fix: `-c core.longpaths=true` on every git call on Windows (built);
the product also fails the snapshot on any such warning.

**F9 — Git LFS writes hooks into the overriding hooks folder.** Every LFS
case left `post-checkout`, `post-commit`, `post-merge` and `pre-push` in the
"empty" `core.hooksPath` folder (LFS installs through `git rev-parse
--git-path hooks`). The first run stopped on the script's own guard. Fix:
the product uses a fresh, private folder per operation, checked empty
before each call and deleted after; the script now reports and empties it
(`hookIntrusions` in the results).

Also seen: `lfs-carried` passes; symlinks are files on both sides
(`core.symlinks=false` even with Developer Mode); the large case's snapshot
took 42 s, most of it the script's own byte-by-byte proof with Defender
scanning — the incremental move after it took 3.7 s with a 1.6 KB bundle.
The real Mac ↔ Windows round trip (step 2 below) is still to run.
## Windows half

On the Windows desktop, with Node ≥ 20 and Git for Windows (it ships
git-lfs), at the same commit of this repo:

1. **The matrix**:
   `node spikes\machines\code\roundtrip.mjs matrix` →
   `results-win32-x64.json`. Expect: the exec-bit, symlink (unless Developer
   Mode) and `windows-names` cases skipped; **`lfs-carried` pass** and
   `lfs-not-carried` an error (the objects must travel); `long-path`
   observed — if it fails without `core.longpaths=true`, the setup must turn
   that on (plan §5.14). Then
   `set S2_PLAIN_COPY=1` and `node … matrix --only mix,binary,paths,behind`
   three times: does F1's miss happen with NTFS timestamps too? (The
   cross-check catches it either way; look for "working tree re-hashed".)
   Compare the `large` timings (Defender scanning may slow them).
2. **A real round trip, Mac → Windows → Mac**, each machine with its own
   git config (Windows: `autocrlf=true` from Git for Windows):
   - Mac: `node spikes/machines/code/roundtrip.mjs demo ~/mc-s2` — creates
     `~/mc-s2/mc-demo` and `~/mc-s2/mc-demo.bundle`; carry the bundle to
     Windows (NAS, USB stick, or `mc-spike` once S1 works);
     Windows: `git clone <the bundle> D:\mc-s2\mc-demo`.
   - Mac: `node … dirty ~/mc-s2/mc-demo`.
   - Windows: `node … haves D:\mc-s2\mc-demo > haves.json`; carry it to the Mac.
   - Mac: `node … snapshot ~/mc-s2/mc-demo ~/mc-s2/move1 --haves haves.json --session ses_demo`;
     carry `move1` to Windows.
   - Windows: `node … apply D:\mc-s2\mc-demo D:\mc-s2\move1`, then
     `node … verify D:\mc-s2\mc-demo D:\mc-s2\move1`.
     Expect PASS or LIMIT with `filemode` (`scripts/new-tool.sh`) and
     `symlink-as-file` (`link-to-notes`; the committed `link-to-readme` is
     already a file there without Developer Mode); raw differences `eol`
     only; never `content`. `git status` on Windows should read like the
     Mac's.
   - Windows: change a file (`docs\notes.txt`), then move back the same way
     (Mac `haves` → Windows `snapshot … --session ses_demo` → Mac `apply`,
     `verify`). Expect the Mac's checkout to be taken as parked, PASS, and a
     small chained bundle.
3. **Optionally the remote mode** with a scratch GitHub repo both machines
   can reach without a prompt (`--transfer remote --remote origin`): check
   that GitHub accepts the hidden refs and that they are gone after `apply`.
