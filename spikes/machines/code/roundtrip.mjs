#!/usr/bin/env node
/*
 * Mother Claude — Machines, Phase 0 spike S2 (docs/PLAN-MACHINES.md §5.10).
 * Throwaway: not shipped, not part of any build.
 *
 * Moves a dirty working tree between two clones of one repository with git
 * plumbing only: a snapshot commit on the source (S), carried as a bundle
 * (default) or as hidden refs on a remote, and landed in the target's (T)
 * checkout when it is clean (default) or in a separate worktree. Then proves
 * the result: HEAD, index (what was staged), working tree (what was on disk,
 * untracked included, ignored not), `git status`, and every file byte for byte
 * with each difference explained.
 *
 * Node >= 20, git on PATH, no npm dependencies; macOS and Windows alike.
 * Git always runs through spawnSync with an argv array, never a shell string.
 *
 *   node roundtrip.mjs matrix [--only <case>[,<case>…]] [--keep] [--verbose]
 *       Every case in throwaway repos under the OS temp dir, with git isolated
 *       from this machine's global and system config. Prints a table and
 *       writes results-<platform>-<arch>.json next to this file (-partial.json
 *       with --only).
 *
 *   A real move between two machines, with each machine's own git config:
 *     T: node roundtrip.mjs haves <repo> > haves.json        (carry it to S)
 *     S: node roundtrip.mjs snapshot <repo> <movedir> [--haves haves.json]
 *            [--transfer bundle|remote] [--remote origin] [--session <id>]
 *            [--push-branch] [--no-carry-lfs]
 *        (carry <movedir> to T by any means: NAS, USB stick, mc-spike)
 *     T: node roundtrip.mjs apply <repo> <movedir> [--landing checkout|worktree]
 *            [--worktree-root <dir>] [--keep-remote-refs]
 *     T: node roundtrip.mjs verify <checkout-or-worktree> <movedir>
 *     T: node roundtrip.mjs undo <repo> <movedir>
 *
 *   node roundtrip.mjs demo <dir>     a scratch repo + a bundle to clone it on the other machine
 *   node roundtrip.mjs dirty <repo>   the standard dirty state for a real round trip
 *
 * Exit codes: 0 pass, 2 pass with a known limitation, 3 stopped (by design),
 * 1 failure or error.
 *
 * S2_PLAIN_COPY=1 makes index copies plain copies (dated now), to reproduce the
 * missed-edit finding; the snapshot's `git status` cross-check then re-hashes.
 */

import { spawnSync } from 'node:child_process';
import crypto from 'node:crypto';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { performance } from 'node:perf_hooks';
import { fileURLToPath, pathToFileURL } from 'node:url';

const HERE = path.dirname(fileURLToPath(import.meta.url));
const IS_WIN = process.platform === 'win32';
const META = 'meta.json';
const APPLY = 'apply.json';
const BUNDLE = 'move.bundle';

// ---------------------------------------------------------------- utilities

const now = () => performance.now();
const since = (t) => Math.round(performance.now() - t);
const sha256 = (buf) => crypto.createHash('sha256').update(buf).digest('hex');
const rmrf = (p) => fs.rmSync(p, { recursive: true, force: true, maxRetries: 5 });
const readJson = (p) => JSON.parse(fs.readFileSync(p, 'utf8'));
const writeJson = (p, v) => fs.writeFileSync(p, JSON.stringify(v, null, 2) + '\n');
const at = (top, rel) => path.join(top, ...rel.split('/'));
const short = (oid) => (oid ? oid.slice(0, 10) : '(none)');
const stamp = () =>
  new Date().toISOString().replace(/[-:]/g, '').replace(/\..+/, '') + '-' + crypto.randomBytes(2).toString('hex');
const safeName = (s) =>
  String(s).replace(/[^A-Za-z0-9._-]+/g, '-').replace(/^-+|-+$/g, '').slice(0, 40) || 'machine';

function tmpDir(prefix) {
  return fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), prefix)));
}

function put(top, rel, content, { exec = false } = {}) {
  const p = at(top, rel);
  fs.mkdirSync(path.dirname(p), { recursive: true });
  fs.writeFileSync(p, content);
  if (exec && !IS_WIN) fs.chmodSync(p, 0o755);
}

function lfNormalize(buf) {
  const out = Buffer.allocUnsafe(buf.length);
  let j = 0;
  for (let i = 0; i < buf.length; i++) {
    if (buf[i] === 13 && buf[i + 1] === 10) continue;
    out[j++] = buf[i];
  }
  return out.subarray(0, j);
}

/** Deterministic pseudo-random bytes (xorshift32), for reproducible fixtures. */
function prng(seed) {
  let x = seed >>> 0 || 1;
  return () => {
    x ^= x << 13;
    x >>>= 0;
    x ^= x >>> 17;
    x ^= x << 5;
    x >>>= 0;
    return x;
  };
}
const WORDS = 'alpha bravo charlie delta echo foxtrot golf hotel india juliet kilo lima mike november oscar papa quebec romeo sierra tango uniform victor whiskey xray yankee zulu const let function return if else for while import export async await'.split(' ');
function textBlob(seed, bytes) {
  const r = prng(seed);
  const parts = [];
  let n = 0;
  while (n < bytes) {
    let line = '';
    const words = 4 + (r() % 10);
    for (let i = 0; i < words; i++) line += WORDS[r() % WORDS.length] + ' ';
    line = line.trimEnd() + '\n';
    parts.push(line);
    n += line.length;
  }
  return parts.join('');
}
function binBlob(seed, bytes) {
  const r = prng(seed);
  const b = Buffer.alloc(bytes);
  for (let i = 0; i < bytes; i += 4) b.writeUInt32LE(r(), i - (i + 4 > bytes ? i + 4 - bytes : 0));
  b[0] = 0; // a NUL byte: git sees binary
  return b;
}

// ---------------------------------------------------------------- git runner

class GitError extends Error {
  constructor(args, res) {
    const err = (res.stderr || '').toString().trim();
    super(`git ${args.join(' ')} → exit ${res.status}${err ? `: ${err.slice(0, 1500)}` : ''}`);
    this.status = res.status;
    this.stderr = err;
  }
}

const SCRUBBED = new Set([
  'GIT_DIR', 'GIT_WORK_TREE', 'GIT_INDEX_FILE', 'GIT_OBJECT_DIRECTORY', 'GIT_ALTERNATE_OBJECT_DIRECTORIES',
  'GIT_NAMESPACE', 'GIT_COMMON_DIR', 'GIT_PREFIX', 'GIT_CONFIG', 'GIT_CONFIG_PARAMETERS', 'GIT_CONFIG_COUNT',
]);

let toolDirs = null;
function ensureToolDirs() {
  if (toolDirs) return toolDirs;
  const dir = tmpDir('mc-s2-tool-');
  const emptyHooks = path.join(dir, 'no-hooks');
  fs.mkdirSync(emptyHooks);
  const emptyConfig = path.join(dir, 'empty.gitconfig');
  fs.writeFileSync(emptyConfig, '');
  toolDirs = { dir, emptyHooks, emptyConfig };
  process.on('exit', () => rmrf(dir));
  return toolDirs;
}

/**
 * A git runner.
 * - isolated: no global or system config, a fixed identity (the matrix).
 * - hooks: false (the design) overrides core.hooksPath with an empty dir, so no
 *   hook of the repository runs; true only for the hooks diagnostic.
 */
function makeCtx({ isolated = false, hooks = false } = {}) {
  const dirs = ensureToolDirs();
  const env = {};
  for (const [k, v] of Object.entries(process.env)) {
    if (SCRUBBED.has(k) || /^GIT_CONFIG_(KEY|VALUE)_\d+$/.test(k)) continue;
    env[k] = v;
  }
  env.GIT_TERMINAL_PROMPT = '0';
  env.GIT_OPTIONAL_LOCKS = '0'; // our `git status` never rewrites an index
  if (isolated) {
    env.GIT_CONFIG_GLOBAL = dirs.emptyConfig;
    env.GIT_CONFIG_NOSYSTEM = '1';
    Object.assign(env, {
      GIT_AUTHOR_NAME: 'spike', GIT_AUTHOR_EMAIL: 'spike@example.invalid',
      GIT_COMMITTER_NAME: 'spike', GIT_COMMITTER_EMAIL: 'spike@example.invalid',
    });
  }
  const pre = [
    '-c', 'commit.gpgSign=false', '-c', 'tag.gpgSign=false',
    '-c', 'gc.auto=0', '-c', 'maintenance.auto=false', '-c', 'advice.detachedHead=false',
  ];
  if (!hooks) pre.push('-c', `core.hooksPath=${dirs.emptyHooks.replace(/\\/g, '/')}`);
  if (isolated) pre.push('-c', 'init.defaultBranch=main', '-c', 'protocol.file.allow=always');

  function git(cwd, args, o = {}) {
    const res = spawnSync('git', [...pre, ...args], {
      cwd,
      env: o.env ? { ...env, ...o.env } : env,
      input: o.input,
      encoding: o.buffer ? 'buffer' : 'utf8',
      maxBuffer: 1 << 30,
      windowsHide: true,
    });
    if (res.error) throw res.error;
    if (res.status !== 0 && !o.ok) throw new GitError(args, res);
    if (o.res) return res;
    if (o.buffer || o.raw) return res.stdout;
    return res.stdout.replace(/\r?\n$/, '');
  }
  const tryGit = (cwd, args, o = {}) => git(cwd, args, { ...o, res: true, ok: true }).status === 0;
  const rev = (cwd, spec) => git(cwd, ['rev-parse', '-q', '--verify', spec], { ok: true }) || null;
  return { git, tryGit, rev, isolated, hooks, env };
}

// ---------------------------------------------------------------- repo helpers

function repoInfo(ctx, repo) {
  const top = path.resolve(ctx.git(repo, ['rev-parse', '--show-toplevel']));
  const gitDir = path.resolve(top, ctx.git(top, ['rev-parse', '--absolute-git-dir']));
  const commonDir = path.resolve(top, ctx.git(top, ['rev-parse', '--git-common-dir']));
  const indexPath = path.resolve(top, ctx.git(top, ['rev-parse', '--git-path', 'index']));
  const sym = ctx.git(top, ['symbolic-ref', '-q', 'HEAD'], { ok: true }) || null;
  const head = ctx.rev(top, 'HEAD^{commit}');
  return { top, gitDir, commonDir, indexPath, sym, head };
}

function configOf(ctx, top) {
  const get = (k, type) =>
    ctx.git(top, ['config', ...(type ? [`--type=${type}`] : []), '--get', k], { ok: true }) || null;
  const bool = (k, dflt) => {
    const v = get(k, 'bool');
    return v === null ? dflt : v === 'true';
  };
  return {
    autocrlf: get('core.autocrlf') || 'false',
    eol: get('core.eol') || null,
    fileMode: bool('core.fileMode', !IS_WIN),
    symlinks: bool('core.symlinks', !IS_WIN),
    ignoreCase: bool('core.ignoreCase', false),
    precomposeUnicode: bool('core.precomposeUnicode', false),
    longpaths: bool('core.longpaths', false),
  };
}

/**
 * A copy of the index that git reads like the original. A plain copy gets a new
 * mtime, and git trusts an entry's stat data when the entry is older than the
 * index file — so a same-size file rewritten in the same second as the index
 * looks unchanged and `git add -A` misses it (seen in this spike). The copy
 * therefore keeps the original's mtime, floored to the second (never later than
 * the original, so never less careful). `racy: true` dates it 1 s after the
 * epoch: every entry is then racy and git re-hashes every file whose stat
 * looks unchanged — slower, independent of timestamps.
 */
function copyIndex(info, dir, name, { racy = false } = {}) {
  const p = path.join(dir, name);
  if (fs.existsSync(info.indexPath)) {
    fs.copyFileSync(info.indexPath, p);
    const st = fs.statSync(info.indexPath);
    // S2_PLAIN_COPY=1 reproduces the miss (a copy dated now) for the experiment in RESULTS-S2.md
    if (racy || !process.env.S2_PLAIN_COPY) fs.utimesSync(p, st.atime, racy ? 1 : Math.floor(st.mtimeMs / 1000));
  }
  return p;
}

/** The working tree as a tree: `git add -A` into a copy of the index. */
function worktreeTree(ctx, info, tmp, { racy = false } = {}) {
  const idx = copyIndex(info, tmp, racy ? 'worktree-racy' : 'worktree', { racy });
  ctx.git(info.top, ['add', '-A'], { env: { GIT_INDEX_FILE: idx } });
  return ctx.git(info.top, ['write-tree'], { env: { GIT_INDEX_FILE: idx } });
}

/**
 * Cross-check: the paths `git status` (the real index, git's own racy logic)
 * reports as changed in the working tree must be exactly the paths that differ
 * between the staged tree and the working-tree tree.
 */
function worktreeCheck(ctx, top, idxTree, wtTree) {
  const st = parseStatus(ctx.git(top, ['status', '--porcelain=v1', '-z', '-uall', '--ignore-submodules=dirty'], { raw: true }));
  const want = new Set([...st].filter(([, xy]) => xy[1] !== ' ').map(([p]) => p));
  const got = new Set(ctx.git(top, ['diff-tree', '-r', '-z', '--name-only', '--no-renames', idxTree, wtTree], { raw: true }).split('\0').filter(Boolean));
  const missing = [...want].filter((p) => !got.has(p));
  const extra = [...got].filter((p) => !want.has(p));
  return { ok: !missing.length && !extra.length, missing, extra };
}

function emptyTree(ctx, top) {
  return ctx.git(top, ['hash-object', '-w', '-t', 'tree', '--stdin'], { input: '' });
}

function lsTree(ctx, top, tree) {
  const out = ctx.git(top, ['ls-tree', '-r', '-z', '--full-tree', tree], { raw: true });
  const entries = [];
  for (const rec of out.split('\0')) {
    if (!rec) continue;
    const tab = rec.indexOf('\t');
    const [mode, type, oid] = rec.slice(0, tab).split(' ');
    entries.push({ path: rec.slice(tab + 1), mode, type, oid });
  }
  return entries;
}

/** `git diff-tree -r -z` between two trees, each change classified. */
function treeDiff(ctx, top, a, b) {
  const out = ctx.git(top, ['diff-tree', '-r', '-z', '--no-renames', a, b], { raw: true });
  const parts = out.split('\0');
  const changes = [];
  for (let i = 0; i < parts.length - 1; i++) {
    if (!parts[i].startsWith(':')) continue;
    const [srcMode, dstMode, srcOid, dstOid, status] = parts[i].slice(1).split(' ');
    changes.push({ path: parts[++i], srcMode, dstMode, srcOid, dstOid, status });
  }
  for (const c of changes) c.kind = classifyTreeChange(c, changes);
  return changes;
}

function classifyTreeChange(c, all) {
  if (c.srcMode === '160000' || c.dstMode === '160000') return 'submodule';
  if (c.srcOid === c.dstOid && (c.status === 'M' || c.status === 'T')) {
    if (c.srcMode === '120000' || c.dstMode === '120000') return 'symlink-as-file';
    if (c.srcMode !== c.dstMode) return 'filemode';
  }
  if ((c.status === 'A' || c.status === 'D') &&
      all.some((o) => o !== c && o.path !== c.path && o.path.toLowerCase() === c.path.toLowerCase())) return 'case';
  return 'content';
}

function parseStatus(z) {
  const m = new Map();
  const parts = z.split('\0');
  for (let i = 0; i < parts.length; i++) {
    const p = parts[i];
    if (!p) continue;
    const xy = p.slice(0, 2);
    m.set(p.slice(3), xy);
    if (xy[0] === 'R' || xy[0] === 'C') i++; // the "from" path follows
  }
  return m;
}

function statusDiff(want, got) {
  const a = parseStatus(want);
  const b = parseStatus(got);
  const diffs = [];
  for (const p of new Set([...a.keys(), ...b.keys()])) {
    const x = a.get(p) ?? null;
    const y = b.get(p) ?? null;
    if (x === y) continue;
    diffs.push({ path: p, want: x, got: y, kind: x === ' A' && y === '??' ? 'ita' : 'other' });
  }
  return diffs;
}

const dirCache = new Map();
/** The name on disk when it differs from `rel`'s last part only in case (not in Unicode normalization). */
function caseOnDisk(file) {
  const dir = path.dirname(file);
  const base = path.basename(file);
  let names = dirCache.get(dir);
  if (!names) {
    try { names = fs.readdirSync(dir); } catch { return null; }
    dirCache.set(dir, names);
  }
  if (names.includes(base)) return null;
  const nfc = base.normalize('NFC');
  if (names.some((n) => n.normalize('NFC') === nfc)) return null;
  const lower = nfc.toLowerCase();
  return names.find((n) => n.normalize('NFC').toLowerCase() === lower) ?? null;
}

function rawOf(ctx, top, rel) {
  const p = at(top, rel);
  let st;
  try { st = fs.lstatSync(p); } catch { return { kind: 'missing' }; }
  const caseName = caseOnDisk(p);
  if (st.isSymbolicLink()) return { kind: 'symlink', target: fs.readlinkSync(p), ...(caseName ? { caseName } : {}) };
  if (st.isDirectory()) {
    const head = fs.existsSync(path.join(p, '.git')) ? ctx.rev(p, 'HEAD^{commit}') : null;
    return { kind: 'dir', head, ...(caseName ? { caseName } : {}) };
  }
  const buf = fs.readFileSync(p);
  return {
    kind: 'file',
    size: buf.length,
    sha256: sha256(buf),
    lfSha256: sha256(lfNormalize(buf)),
    crlf: buf.includes('\r\n'),
    exec: IS_WIN ? null : !!(st.mode & 0o111),
    ...(caseName ? { caseName } : {}),
  };
}

function sameRaw(a, b) {
  if (a.kind !== b.kind) return false;
  if (a.kind === 'file') return a.sha256 === b.sha256;
  if (a.kind === 'symlink') return a.target === b.target;
  return true;
}

/** What the user would notice if our code touched their checkout. */
function probe(ctx, repo) {
  const info = repoInfo(ctx, repo);
  const top = info.top;
  dirCache.clear();
  const status = ctx.git(top, ['status', '--porcelain=v1', '-z', '-uall'], { raw: true });
  const indexSha = fs.existsSync(info.indexPath) ? sha256(fs.readFileSync(info.indexPath)) : null;
  const tmp = tmpDir('mc-s2-probe-');
  let indexTree;
  try {
    indexTree = ctx.git(top, ['write-tree'], { env: { GIT_INDEX_FILE: copyIndex(info, tmp, 'i') } });
  } finally { rmrf(tmp); }
  const refs = ctx.git(top, ['for-each-ref', '--format=%(refname) %(objectname)'])
    .split('\n').filter(Boolean);
  const listed = ctx.git(top, ['ls-files', '-z', '-c', '-o', '--exclude-standard'], { raw: true })
    .split('\0').filter(Boolean);
  const h = crypto.createHash('sha256');
  for (const rel of [...new Set(listed)].sort()) {
    const r = rawOf(ctx, top, rel);
    h.update(`${rel}\0${r.kind}\0${r.sha256 ?? r.target ?? r.head ?? ''}\0${r.exec ?? ''}\n`);
  }
  const worktrees = ctx.git(top, ['worktree', 'list', '--porcelain'])
    .split('\n').filter((l) => l.startsWith('worktree ')).map((l) => l.slice(9));
  return { sym: info.sym, head: info.head, status, indexSha, indexTree, refs, filesHash: h.digest('hex'), worktrees };
}

function compareProbes(a, b, fields, { ignoreRefPrefixes = ['refs/mother-claude/'] } = {}) {
  const diffs = [];
  for (const f of fields) {
    let x = a[f];
    let y = b[f];
    if (f === 'refs') {
      const keep = (l) => !ignoreRefPrefixes.some((p) => l.startsWith(p));
      x = x.filter(keep).join('\n');
      y = y.filter(keep).join('\n');
    } else if (Array.isArray(x)) {
      x = x.join('\n');
      y = y.join('\n');
    }
    if (x !== y) diffs.push(f);
  }
  return diffs;
}

function preflightRefuse(info) {
  for (const f of ['MERGE_HEAD', 'CHERRY_PICK_HEAD', 'REVERT_HEAD', 'rebase-merge', 'rebase-apply', 'BISECT_LOG']) {
    if (fs.existsSync(path.join(info.gitDir, f))) throw new Error(`a git operation is in progress (${f}): finish or abort it first`);
  }
}

/** Real mode: the machine's own identity; only when it has none, a stand-in, so commit-tree works. */
function identityEnv(ctx, top) {
  if (ctx.isolated) return undefined;
  const name = ctx.git(top, ['config', '--get', 'user.name'], { ok: true });
  const email = ctx.git(top, ['config', '--get', 'user.email'], { ok: true });
  const env = {};
  if (!name) Object.assign(env, { GIT_AUTHOR_NAME: 'Mother Claude', GIT_COMMITTER_NAME: 'Mother Claude' });
  if (!email) Object.assign(env, { GIT_AUTHOR_EMAIL: 'mother-claude@localhost', GIT_COMMITTER_EMAIL: 'mother-claude@localhost' });
  return Object.keys(env).length ? env : undefined;
}

// ---------------------------------------------------------------- T → S: what T has

function haves(ctx, repo) {
  const info = repoInfo(ctx, repo);
  const out = ctx.git(info.top, [
    'for-each-ref', '--format=%(objectname) %(objecttype) %(*objectname)',
    'refs/heads', 'refs/remotes', 'refs/tags', 'refs/mother-claude/moves',
  ]);
  const set = new Set(info.head ? [info.head] : []);
  for (const line of out.split('\n')) {
    const [oid, type, peeled] = line.split(' ');
    if (type === 'commit') set.add(oid);
    else if (peeled) set.add(peeled);
  }
  const shallow = ctx.git(info.top, ['rev-parse', '--is-shallow-repository']) === 'true';
  return { haves: [...set].slice(0, 500), shallow, machine: safeName(os.hostname()) };
}

function filterHaves(ctx, top, list) {
  if (!list.length) return [];
  const out = ctx.git(top, ['cat-file', '--batch-check=%(objectname) %(objecttype)'], { input: list.join('\n') + '\n' });
  const ok = new Set();
  for (const line of out.split('\n')) {
    const [oid, type] = line.split(' ');
    if (type === 'commit') ok.add(oid);
  }
  return [...ok];
}

// ---------------------------------------------------------------- snapshot (S)

const WIN_RESERVED = /^(con|prn|aux|nul|com[1-9]|lpt[1-9])(\..*)?$/i;

function snapshotWarnings(ctx, top, { entries, idxTree, wtTree, base, status }) {
  const w = [];
  const paths = entries.map((e) => e.path);
  // paths differing only in case
  const seen = new Map();
  const collisions = [];
  for (const p of paths) {
    const k = p.toLowerCase();
    if (seen.has(k)) collisions.push([seen.get(k), p]);
    else seen.set(k, p);
  }
  if (collisions.length) w.push({ kind: 'case-collision', detail: collisions.slice(0, 20) });
  // names Windows cannot hold
  const bad = paths.filter((p) => p.split('/').some((n) =>
    WIN_RESERVED.test(n) || /[<>:"|?*\\\x00-\x1f]/.test(n) || /[. ]$/.test(n)));
  if (bad.length) w.push({ kind: 'windows-names', detail: bad.slice(0, 20) });
  // long paths (Windows MAX_PATH is 260 for the whole path)
  const long = paths.filter((p) => p.length > 200);
  if (long.length) w.push({ kind: 'long-path', detail: long.slice(0, 5).map((p) => `${p.length}: ${p.slice(0, 60)}…`) });
  // submodules: commits must exist on T; changes inside them do not travel
  const gitlinks = entries.filter((e) => e.mode === '160000');
  if (gitlinks.length) {
    const dirty = [...parseStatus(status)].filter(([p, xy]) => gitlinks.some((g) => g.path === p) && xy !== '  ').map(([p]) => p);
    w.push({ kind: 'submodules', detail: { paths: gitlinks.map((g) => g.path), changed: dirty } });
  }
  // Git LFS: objects do not travel in the bundle
  const attrs = entries.filter((e) => e.path === '.gitattributes' || e.path.endsWith('/.gitattributes'));
  if (attrs.some((e) => /filter=lfs/.test(ctx.git(top, ['cat-file', '-p', e.oid])))) w.push({ kind: 'lfs' });
  // changes a target without exec bits or symlinks cannot hold
  const idx = new Map(lsTree(ctx, top, idxTree).map((e) => [e.path, e]));
  const modeOnly = entries.filter((e) => idx.has(e.path) && idx.get(e.path).oid === e.oid && idx.get(e.path).mode !== e.mode
    && e.mode !== '120000' && idx.get(e.path).mode !== '120000');
  if (modeOnly.length) w.push({ kind: 'unstaged-mode-change', detail: modeOnly.map((e) => e.path).slice(0, 20) });
  const newExec = entries.filter((e) => !idx.has(e.path) && e.mode === '100755');
  if (newExec.length) w.push({ kind: 'untracked-exec', detail: newExec.map((e) => e.path).slice(0, 20) });
  const newLinks = entries.filter((e) => !idx.has(e.path) && e.mode === '120000');
  if (newLinks.length) w.push({ kind: 'untracked-symlink', detail: newLinks.map((e) => e.path).slice(0, 20) });
  if (ctx.git(top, ['rev-parse', '--is-shallow-repository']) === 'true') w.push({ kind: 'shallow-source' });
  if (!base) w.push({ kind: 'unborn' });
  return w;
}

function lfsPointers(ctx, top, base, revs) {
  const blobs = new Set();
  for (const rev of revs) {
    if (base) {
      for (const c of treeDiff(ctx, top, base, rev)) if (c.status !== 'D' && /^1006|^1007/.test(c.dstMode)) blobs.add(c.dstOid);
    } else {
      for (const e of lsTree(ctx, top, rev)) if (e.type === 'blob' && e.mode !== '120000') blobs.add(e.oid);
    }
  }
  if (!blobs.size) return [];
  const sizes = ctx.git(top, ['cat-file', '--batch-check=%(objectname) %(objectsize)'], { input: [...blobs].join('\n') + '\n' });
  const oids = [];
  for (const line of sizes.split('\n')) {
    const [oid, size] = line.split(' ');
    if (Number(size) > 1024) continue;
    const text = ctx.git(top, ['cat-file', '-p', oid]);
    const m = /^version https:\/\/git-lfs\.github\.com\/spec\/v1\noid sha256:([0-9a-f]{64})\nsize (\d+)/.exec(text);
    if (m) oids.push({ oid: m[1], size: Number(m[2]) });
  }
  return oids;
}

const lfsObject = (commonDir, oid) => path.join(commonDir, 'lfs', 'objects', oid.slice(0, 2), oid.slice(2, 4), oid);

/**
 * §5.10 snapshot. The real index, branch and working tree are untouched:
 * both trees come from copies of the index (`git write-tree` may rewrite the
 * index it reads, see diagnostics), and HEAD, refs and files are compared
 * before and after.
 */
function snapshot(ctx, repo, outdir, o = {}) {
  const timings = {};
  let t = now();
  const info = repoInfo(ctx, repo);
  const top = info.top;
  const session = o.session || `ses_${crypto.randomBytes(4).toString('hex')}`;
  const epoch = String(o.epoch ?? Date.now());
  const machine = safeName(o.machine || os.hostname());
  const transfer = o.transfer || 'bundle';
  if (!['bundle', 'remote'].includes(transfer)) throw new Error(`unknown transfer ${transfer}`);
  preflightRefuse(info);
  if (ctx.git(top, ['ls-files', '-u', '-z'], { raw: true })) throw new Error('the index has unmerged entries: finish the merge first');

  const before = probe(ctx, top);
  timings.probe = since(t); t = now();
  const tmp = tmpDir('mc-s2-snap-');
  try {
    const stagedIdx = copyIndex(info, tmp, 'staged');
    const idxTree = ctx.git(top, ['write-tree'], { env: { GIT_INDEX_FILE: stagedIdx } });
    let wtTree = worktreeTree(ctx, info, tmp);
    let wtCheck = worktreeCheck(ctx, top, idxTree, wtTree);
    if (!wtCheck.ok) {
      // git's stat data misled the copy: hash every file instead
      const first = wtCheck;
      wtTree = worktreeTree(ctx, info, tmp, { racy: true });
      wtCheck = { ...worktreeCheck(ctx, top, idxTree, wtTree), recomputed: true, first };
      if (!wtCheck.ok) throw new Error(`the snapshot does not match git status: missing ${wtCheck.missing.join(', ')}; extra ${wtCheck.extra.join(', ')}`);
    }
    timings.trees = since(t); t = now();

    const base = info.head;
    const branch = info.sym;
    const parents = base ? ['-p', base] : [];
    // Chain: earlier snapshots of this session that T reports having become extra
    // parents (after base, so base stays the first parent). Otherwise a bundle's
    // `--not <earlier snapshot>` excludes nothing (it is not an ancestor) and an
    // incremental move re-sends everything the earlier move carried.
    const usedHaves = transfer === 'bundle' ? filterHaves(ctx, top, o.haves || []) : [];
    const own = sessionMoves(ctx, top, session).map((m) => m.snap);
    const chained = o.chain === false ? [] : own.filter((oid) => usedHaves.includes(oid));
    const chainArgs = chained.flatMap((oid) => ['-p', oid]);
    const idEnv = identityEnv(ctx, top);
    // The staged tree is not reachable from the snapshot, so it travels as a
    // commit of its own (like git stash's "index" commit), with the same parent.
    const indexCommit = ctx.git(top, ['commit-tree', '--no-gpg-sign', ...parents,
      '-m', `mother-claude: staged state of ${session} from ${machine}`, idxTree], { env: idEnv });
    const snap = ctx.git(top, ['commit-tree', '--no-gpg-sign', ...parents, ...chainArgs,
      '-m', `mother-claude: move ${session} from ${machine}`, wtTree], { env: idEnv });
    const refBase = `refs/mother-claude/moves/${session}/${epoch}`;
    const refs = { snap: `${refBase}/snap`, index: `${refBase}/index` };
    ctx.git(top, ['update-ref', '-m', 'mother-claude snapshot', refs.snap, snap, '']);
    ctx.git(top, ['update-ref', '-m', 'mother-claude snapshot', refs.index, indexCommit, '']);
    timings.commits = since(t); t = now();

    const entries = lsTree(ctx, top, wtTree);
    dirCache.clear();
    const manifest = entries.map((e) => ({ ...e, raw: rawOf(ctx, top, e.path) }));
    const status = ctx.git(top, ['status', '--porcelain=v1', '-z', '-uall'], { raw: true });
    const warnings = snapshotWarnings(ctx, top, { entries, idxTree, wtTree, base, status });
    timings.manifest = since(t); t = now();

    fs.mkdirSync(outdir, { recursive: true });
    let bundle = null;
    let remote = null;
    if (transfer === 'bundle') {
      const offered = o.haves || [];
      const used = usedHaves;
      const file = path.join(outdir, BUNDLE);
      rmrf(file);
      ctx.git(top, ['bundle', 'create', '-q', file, refs.snap, refs.index, ...(used.length ? ['--not', ...used] : [])]);
      bundle = { file: BUNDLE, size: fs.statSync(file).size, havesOffered: offered.length, havesUsed: used.length };
      timings.bundle = since(t);
    } else {
      const name = o.remote || 'origin';
      const hidden = `refs/mother-claude/${machine}/${session}`;
      const specs = [`${snap}:${hidden}/snap`, `${indexCommit}:${hidden}/index`];
      const pushBranch = !!(o.pushBranch && branch && base);
      if (pushBranch) specs.push(`${base}:${branch}`);
      ctx.git(top, ['push', '--no-verify', '-q', '--recurse-submodules=no', name, ...specs]);
      remote = { remote: name, hidden: { snap: `${hidden}/snap`, index: `${hidden}/index` }, pushedBranch: pushBranch };
      timings.push = since(t);
    }
    t = now();

    let lfs = null;
    if (warnings.some((x) => x.kind === 'lfs') && ctx.tryGit(top, ['lfs', 'version'])) {
      const pointers = lfsPointers(ctx, top, base, [snap, indexCommit]);
      const carried = [];
      const missing = [];
      if (o.carryLfs !== false) {
        for (const p of pointers) {
          const src = lfsObject(info.commonDir, p.oid);
          if (!fs.existsSync(src)) { missing.push(p.oid); continue; }
          const dst = lfsObject(outdir, p.oid);
          fs.mkdirSync(path.dirname(dst), { recursive: true });
          fs.copyFileSync(src, dst);
          carried.push(p.oid);
        }
      }
      lfs = { pointers: pointers.length, carried, missing, carriedDisabled: o.carryLfs === false };
      timings.lfs = since(t);
    }

    const after = probe(ctx, top);
    const changed = compareProbes(before, after, ['status', 'indexSha', 'sym', 'head', 'refs', 'filesHash', 'worktrees']);
    const meta = {
      version: 1,
      tool: 'mother-claude S2 roundtrip.mjs',
      session, epoch, machine,
      platform: `${process.platform}-${process.arch}`,
      created: new Date().toISOString(),
      branch, base, idxTree, wtTree, snap, indexCommit, refs,
      transfer, bundle, remote, lfs, chained,
      config: configOf(ctx, top),
      status, manifest, warnings,
      untouched: { ok: changed.length === 0, changed },
      wtCheck,
      timings,
    };
    writeJson(path.join(outdir, META), meta);
    return meta;
  } finally {
    rmrf(tmp);
  }
}

// ---------------------------------------------------------------- apply (T)

function fetchMove(ctx, top, indir, meta, o) {
  const dst = meta.refs;
  if (meta.transfer === 'bundle') {
    const file = path.join(indir, meta.bundle.file);
    ctx.git(top, ['bundle', 'verify', '-q', file]);
    ctx.git(top, ['fetch', '-q', '--no-tags', '--no-write-fetch-head', '--no-auto-maintenance', '--no-recurse-submodules', file,
      `${dst.snap}:${dst.snap}`, `${dst.index}:${dst.index}`]);
  } else {
    const r = meta.remote;
    ctx.git(top, ['fetch', '-q', '--no-tags', '--no-write-fetch-head', '--no-auto-maintenance', '--no-recurse-submodules', r.remote,
      `${r.hidden.snap}:${dst.snap}`, `${r.hidden.index}:${dst.index}`]);
    if (!o.keepRemoteRefs) ctx.git(top, ['push', '--no-verify', '-q', '--recurse-submodules=no', r.remote, '--delete', r.hidden.snap, r.hidden.index]);
  }
  const snap = ctx.rev(top, `${dst.snap}^{commit}`);
  const index = ctx.rev(top, `${dst.index}^{commit}`);
  if (snap !== meta.snap || index !== meta.indexCommit) throw new Error('the fetched snapshot does not match meta.json');
  // the commits' objects are complete (a bundle that relied on objects T lacks fails here)
  ctx.git(top, ['rev-list', '--objects', '--missing=error', meta.snap, meta.indexCommit, ...(meta.base ? ['--not', meta.base] : [])], { raw: true });
}

/** Every snapshot of this session this repository has (made here or received), newest first. */
function sessionMoves(ctx, top, session) {
  const out = ctx.git(top, ['for-each-ref', '--sort=-refname', '--format=%(refname) %(objectname)', `refs/mother-claude/moves/${session}/`]);
  const byEpoch = new Map();
  for (const line of out.split('\n').filter(Boolean)) {
    const [ref, oid] = line.split(' ');
    const m = /\/moves\/[^/]+\/([^/]+)\/(snap|index)$/.exec(ref);
    if (!m) continue;
    const e = byEpoch.get(m[1]) ?? { epoch: m[1] };
    e[m[2]] = oid;
    byEpoch.set(m[1], e);
  }
  return [...byEpoch.values()].filter((e) => e.snap && e.index)
    .sort((a, b) => (b.epoch.length - a.epoch.length) || (b.epoch < a.epoch ? -1 : b.epoch > a.epoch ? 1 : 0));
}

/**
 * A checkout that still holds exactly a state of this session it sent or
 * received — HEAD at that snapshot's base, index = its staged tree, working
 * tree = its tree — counts as clean for this session: the session left from
 * here and nobody touched the files since. That makes *bring back* a
 * fast-forward instead of a refusal.
 */
function parkedState(ctx, info, session) {
  const moves = sessionMoves(ctx, info.top, session);
  if (!moves.length) return null;
  const tmp = tmpDir('mc-s2-parked-');
  try {
    const idxNow = ctx.git(info.top, ['write-tree'], { env: { GIT_INDEX_FILE: copyIndex(info, tmp, 'staged') } });
    const wtNow = worktreeTree(ctx, info, tmp, { racy: true });
    for (const m of moves) {
      const wtTree = ctx.rev(info.top, `${m.snap}^{tree}`);
      const idxTree = ctx.rev(info.top, `${m.index}^{tree}`);
      const base = ctx.rev(info.top, `${m.index}^1`); // the index commit has base as its only parent
      if (wtNow === wtTree && idxNow === idxTree && (info.head ?? null) === base) return { ...m, wtTree, idxTree, base };
    }
    return null;
  } finally { rmrf(tmp); }
}

function installLfs(ctx, top, indir, meta) {
  if (!meta.lfs?.carried?.length) return 0;
  const info = repoInfo(ctx, top);
  let n = 0;
  for (const oid of meta.lfs.carried) {
    const src = lfsObject(indir, oid);
    const dst = lfsObject(info.commonDir, oid);
    if (fs.existsSync(dst)) continue;
    fs.mkdirSync(path.dirname(dst), { recursive: true });
    fs.copyFileSync(src, dst);
    n++;
  }
  return n;
}

function otherWorktreeOn(ctx, top, branchRef) {
  const out = ctx.git(top, ['worktree', 'list', '--porcelain']);
  const norm = (p) => (IS_WIN ? path.resolve(p).toLowerCase() : path.resolve(p));
  let wt = null;
  for (const line of out.split('\n')) {
    if (line.startsWith('worktree ')) wt = line.slice(9);
    else if (line === `branch ${branchRef}` && wt && norm(wt) !== norm(top)) return wt;
  }
  return null;
}

/** Submodule commits named by the snapshot: checked out (plumbing) when present on T. */
function landSubmodules(ctx, top, meta) {
  const out = [];
  for (const e of meta.manifest.filter((x) => x.mode === '160000')) {
    const p = at(top, e.path);
    if (!fs.existsSync(path.join(p, '.git'))) { out.push({ path: e.path, state: 'not-initialized' }); continue; }
    const head = ctx.rev(p, 'HEAD^{commit}');
    if (head === e.oid) { out.push({ path: e.path, state: 'same' }); continue; }
    if (!ctx.tryGit(p, ['cat-file', '-e', `${e.oid}^{commit}`])) { out.push({ path: e.path, state: 'missing', want: e.oid }); continue; }
    if (ctx.git(p, ['status', '--porcelain=v1', '-z', '-uall'], { raw: true })) { out.push({ path: e.path, state: 'dirty' }); continue; }
    ctx.git(p, ['read-tree', '-m', '-u', head || emptyTree(ctx, p), e.oid]);
    ctx.git(p, ['update-ref', '--no-deref', 'HEAD', e.oid]);
    out.push({ path: e.path, state: 'updated', from: head, to: e.oid });
  }
  return out;
}

function apply(ctx, repo, indir, o = {}) {
  const meta = readJson(path.join(indir, META));
  const landing = o.landing || 'checkout';
  if (!['checkout', 'worktree'].includes(landing)) throw new Error(`unknown landing ${landing}`);
  const timings = {};
  let t = now();
  const info = repoInfo(ctx, repo);
  const top = info.top;
  const result = { landing, session: meta.session, epoch: meta.epoch, machine: safeName(o.machine || os.hostname()), timings };
  const finish = (r) => { writeJson(path.join(indir, APPLY), { ...result, ...r }); return { ...result, ...r }; };
  preflightRefuse(info);

  let parked = null;
  if (landing === 'checkout') {
    const st = ctx.git(top, ['status', '--porcelain=v1', '-z', '-uall'], { raw: true });
    if (st) {
      parked = parkedState(ctx, info, meta.session);
      if (!parked) return finish({ outcome: 'stop', reason: 'the target checkout has changes of its own', offer: 'worktree', targetStatus: parseStatus(st).size });
    }
  }
  fetchMove(ctx, top, indir, meta, o);
  result.fetched = true;
  timings.fetch = since(t); t = now();
  const lfsInstalled = installLfs(ctx, top, indir, meta);
  const empty = emptyTree(ctx, top);

  if (landing === 'checkout') {
    let action;
    let tB = null;
    if (meta.branch) {
      tB = ctx.rev(top, `${meta.branch}^{commit}`);
      if (tB) {
        if (meta.base && tB === meta.base) action = 'same';
        else if (meta.base && ctx.tryGit(top, ['merge-base', '--is-ancestor', tB, meta.base])) action = 'fast-forward';
        else return finish({ outcome: 'stop', reason: `the target's ${meta.branch.replace('refs/heads/', '')} has commits the snapshot's base lacks`, offer: 'worktree, or reset it to the source (its tip kept as an undo ref)', targetTip: tB });
      } else action = meta.base ? 'create' : 'unborn';
      const other = otherWorktreeOn(ctx, top, meta.branch);
      if (other) return finish({ outcome: 'stop', reason: `${meta.branch} is checked out in another worktree (${other})`, offer: 'worktree' });
    } else action = 'detach';

    const ts = stamp();
    const undoRef = `refs/mother-claude/undo/${ts}`;
    if (info.head) ctx.git(top, ['update-ref', `${undoRef}/head`, info.head, '']);
    if (tB && tB !== info.head) ctx.git(top, ['update-ref', `${undoRef}/branch`, tB, '']);
    const undo = { ts, ref: undoRef, prevSym: info.sym, prevHead: info.head, branchOld: tB, action, parked: parked ? { epoch: parked.epoch, wtTree: parked.wtTree, idxTree: parked.idxTree } : null };

    // 1. the checkout at base. A clean checkout's index is its HEAD's tree; a
    //    parked one's working tree is the parked snapshot's tree, so the index
    //    is set to that tree first and the move starts from there.
    if (parked) {
      ctx.git(top, ['read-tree', parked.wtTree]);
      // read-tree leaves no stat data, so every entry would look modified and the
      // merge below refuses ("not uptodate"); a refresh re-hashes and records it
      ctx.git(top, ['update-index', '-q', '--refresh'], { ok: true });
    }
    const from = parked ? parked.wtTree : info.head || empty;
    const baseRev = meta.base || empty;
    if (from !== baseRev) ctx.git(top, ['read-tree', '-m', '-u', from, baseRev]);
    timings.toBase = since(t); t = now();
    // 2. the working tree = the snapshot
    ctx.git(top, ['read-tree', '-m', '-u', baseRev, meta.snap]);
    timings.toSnapshot = since(t); t = now();
    // 3. refs: the branch at base (created or fast-forwarded), HEAD on it (or detached)
    if (action === 'fast-forward') ctx.git(top, ['update-ref', meta.branch, meta.base, tB]);
    if (action === 'create') ctx.git(top, ['update-ref', meta.branch, meta.base, '']);
    if (meta.branch) { if (info.sym !== meta.branch) ctx.git(top, ['symbolic-ref', 'HEAD', meta.branch]); }
    else ctx.git(top, ['update-ref', '--no-deref', 'HEAD', meta.base]);
    // 4. the index = what was staged
    ctx.git(top, ['read-tree', meta.idxTree]);
    ctx.git(top, ['update-index', '-q', '--refresh'], { ok: true });
    timings.index = since(t); t = now();
    const submodules = landSubmodules(ctx, top, meta);
    timings.submodules = since(t);
    return finish({ outcome: 'applied', path: top, action, parked: parked?.epoch ?? null, undo, submodules, lfsInstalled });
  }

  // worktree landing: the checkout is never touched
  const root = path.resolve(o.worktreeRoot || path.join(os.tmpdir(), 'mc-s2-appdata', 'machines', 'worktrees'));
  const dir = path.join(root, meta.session);
  if (fs.existsSync(dir) && fs.readdirSync(dir).length) return finish({ outcome: 'stop', reason: `${dir} exists and is not empty` });
  fs.mkdirSync(root, { recursive: true });
  const wtBranch = meta.branch ? `refs/heads/mother-claude/${meta.session}` : null;
  if (meta.base) {
    if (wtBranch) ctx.git(top, ['worktree', 'add', '-q', '--no-checkout', '-b', wtBranch.replace('refs/heads/', ''), dir, meta.base]);
    else ctx.git(top, ['worktree', 'add', '-q', '--no-checkout', '--detach', dir, meta.base]);
  } else {
    ctx.git(top, ['worktree', 'add', '-q', '--orphan', '-b', wtBranch.replace('refs/heads/', ''), dir]);
  }
  timings.worktreeAdd = since(t); t = now();
  ctx.git(dir, ['read-tree', '-m', '-u', empty, meta.snap]);
  ctx.git(dir, ['read-tree', meta.idxTree]);
  ctx.git(dir, ['update-index', '-q', '--refresh'], { ok: true });
  timings.toSnapshot = since(t); t = now();
  const submodules = landSubmodules(ctx, dir, meta);
  return finish({ outcome: 'applied', path: dir, wtBranch, submodules, lfsInstalled });
}

// ---------------------------------------------------------------- verify (T)

const EXPECTED_RAW = new Set(['eol', 'filter', 'filemode', 'symlink-as-file']);
const LIMIT_KINDS = new Set(['filemode', 'symlink-as-file', 'submodule', 'case', 'ita']);

function rawCompare(ctx, top, manifest, cfg) {
  dirCache.clear();
  const diffs = [];
  for (const e of manifest) {
    const want = e.raw;
    if (want.kind === 'missing') continue;
    const got = rawOf(ctx, top, e.path);
    const d = { path: e.path };
    if (e.mode === '160000') {
      if (got.kind === 'dir' && got.head === e.oid) continue;
      diffs.push({ ...d, kind: 'submodule', want: e.oid, got: got.head ?? got.kind });
      continue;
    }
    if (got.kind === 'missing') { diffs.push({ ...d, kind: 'missing' }); continue; }
    if ((want.caseName || null) !== (got.caseName || null)) diffs.push({ ...d, kind: 'case', want: want.caseName ?? '(as in git)', got: got.caseName ?? '(as in git)' });
    if (want.kind === 'file' && got.kind === 'file') {
      if (got.sha256 === want.sha256) {
        if (want.exec !== null && got.exec !== null && want.exec !== got.exec) diffs.push({ ...d, kind: cfg.fileMode ? 'exec-bit' : 'filemode', want: want.exec, got: got.exec });
        continue;
      }
      if (got.lfSha256 === want.lfSha256) { diffs.push({ ...d, kind: 'eol', want: want.crlf ? 'CRLF' : 'LF', got: got.crlf ? 'CRLF' : 'LF' }); continue; }
      const oid = ctx.git(top, ['hash-object', `--path=${e.path}`, at(top, e.path)]);
      diffs.push({ ...d, kind: oid === e.oid ? 'filter' : 'content' });
      continue;
    }
    if (want.kind === 'symlink' && got.kind === 'symlink') {
      if (got.target !== want.target) diffs.push({ ...d, kind: 'content', want: want.target, got: got.target });
      continue;
    }
    if (want.kind === 'symlink' && got.kind === 'file') {
      const text = fs.readFileSync(at(top, e.path), 'utf8');
      diffs.push({ ...d, kind: text === want.target ? 'symlink-as-file' : 'content' });
      continue;
    }
    diffs.push({ ...d, kind: 'type', want: want.kind, got: got.kind });
  }
  return diffs;
}

function verify(ctx, targetPath, indir) {
  const meta = readJson(path.join(indir, META));
  const ap = readJson(path.join(indir, APPLY));
  if (ap.outcome !== 'applied') return { outcome: ap.outcome, reason: ap.reason, checks: [] };
  const t = now();
  const info = repoInfo(ctx, targetPath);
  const top = info.top;
  const cfg = configOf(ctx, top);
  const checks = [];
  const wantSym = ap.landing === 'worktree' ? ap.wtBranch ?? null : meta.branch ?? null;
  checks.push({
    check: 'HEAD at the base, on the branch',
    ok: (info.head ?? null) === (meta.base ?? null) && (info.sym ?? null) === wantSym,
    want: `${wantSym ?? 'detached'} @ ${short(meta.base)}`, got: `${info.sym ?? 'detached'} @ ${short(info.head)}`,
  });
  const tmp = tmpDir('mc-s2-verify-');
  let idxDiff;
  let wtDiff;
  try {
    const idxNow = ctx.git(top, ['write-tree'], { env: { GIT_INDEX_FILE: copyIndex(info, tmp, 'staged') } });
    const wtNow = worktreeTree(ctx, info, tmp, { racy: true });
    idxDiff = idxNow === meta.idxTree ? [] : treeDiff(ctx, top, meta.idxTree, idxNow);
    wtDiff = wtNow === meta.wtTree ? [] : treeDiff(ctx, top, meta.wtTree, wtNow);
  } finally { rmrf(tmp); }
  checks.push({ check: 'index = the staged tree', ok: !idxDiff.length, diffs: idxDiff });
  checks.push({ check: 'working tree = the snapshot tree', ok: !wtDiff.length, diffs: wtDiff });
  const st = ctx.git(top, ['status', '--porcelain=v1', '-z', '-uall'], { raw: true });
  const stDiff = st === meta.status ? [] : statusDiff(meta.status, st);
  checks.push({ check: 'git status as on the source', ok: !stDiff.length, diffs: stDiff });
  const raw = rawCompare(ctx, top, meta.manifest, cfg);
  checks.push({ check: 'every file byte for byte (or explained)', ok: raw.every((d) => EXPECTED_RAW.has(d.kind)), diffs: raw });

  const fails = [];
  const limits = new Set();
  if (!checks[0].ok) fails.push(`HEAD: want ${checks[0].want}, got ${checks[0].got}`);
  const treePaths = new Set();
  for (const c of [...idxDiff, ...wtDiff]) {
    treePaths.add(c.path);
    if (LIMIT_KINDS.has(c.kind)) limits.add(c.kind);
    else fails.push(`tree ${c.kind}: ${c.path} (${c.srcMode} ${short(c.srcOid)} → ${c.dstMode} ${short(c.dstOid)})`);
  }
  for (const d of stDiff) {
    if (d.kind === 'ita') limits.add('ita');
    else if (!treePaths.has(d.path)) fails.push(`status: ${d.path} want "${d.want}" got "${d.got}"`);
  }
  for (const d of raw) {
    if (EXPECTED_RAW.has(d.kind)) continue;
    if (LIMIT_KINDS.has(d.kind)) limits.add(d.kind);
    else fails.push(`file ${d.kind}: ${d.path}${d.want !== undefined ? ` (want ${d.want}, got ${d.got})` : ''}`);
  }
  const outcome = fails.length ? 'fail' : limits.size ? 'limit' : 'pass';
  const rawKinds = [...new Set(raw.map((d) => d.kind))];
  return { outcome, limits: [...limits], fails, rawKinds, checks, config: cfg, ms: since(t) };
}

// ---------------------------------------------------------------- undo (T)

function removeEmptyParents(top, file) {
  let d = path.dirname(file);
  while (d.startsWith(top) && d !== top) {
    try { if (fs.readdirSync(d).length) break; fs.rmdirSync(d); } catch { break; }
    d = path.dirname(d);
  }
}

function undo(ctx, repo, indir) {
  const meta = readJson(path.join(indir, META));
  const ap = readJson(path.join(indir, APPLY));
  if (ap.outcome !== 'applied') throw new Error('nothing to undo: the move was not applied');
  const info = repoInfo(ctx, repo);
  const top = info.top;
  if (ap.landing === 'worktree') {
    ctx.git(top, ['worktree', 'remove', '--force', ap.path]);
    if (ap.wtBranch) ctx.git(top, ['update-ref', '-d', ap.wtBranch]);
    return { undone: true, landing: 'worktree', kept: [] };
  }
  const u = ap.undo;
  const empty = emptyTree(ctx, top);
  const prev = u.parked ? u.parked.wtTree : u.prevHead || empty;
  const prevPaths = new Set(lsTree(ctx, top, prev).map((e) => e.path));
  const idxPaths = new Set(lsTree(ctx, top, meta.idxTree).map((e) => e.path));
  ctx.git(top, ['read-tree', '--reset', '-u', prev]);
  // files that were untracked on the source: removed only while still exactly as moved
  const kept = [];
  dirCache.clear();
  for (const e of meta.manifest) {
    if (idxPaths.has(e.path) || prevPaths.has(e.path)) continue;
    const now = rawOf(ctx, top, e.path);
    if (now.kind === 'missing') continue;
    if (sameRaw(now, e.raw) || (now.kind === 'file' && e.raw.kind === 'file' && now.lfSha256 === e.raw.lfSha256)) {
      fs.rmSync(at(top, e.path), { force: true });
      removeEmptyParents(top, at(top, e.path));
    } else kept.push(e.path);
  }
  if (u.parked) ctx.git(top, ['read-tree', u.parked.idxTree]); // the parked staged state, files untouched
  if (u.action === 'fast-forward') ctx.git(top, ['update-ref', meta.branch, u.branchOld, meta.base]);
  if (u.action === 'create') ctx.git(top, ['update-ref', '-d', meta.branch, meta.base]);
  if (u.prevSym) ctx.git(top, ['symbolic-ref', 'HEAD', u.prevSym]);
  else if (u.prevHead) ctx.git(top, ['update-ref', '--no-deref', 'HEAD', u.prevHead]);
  ctx.git(top, ['update-index', '-q', '--refresh'], { ok: true });
  return { undone: true, landing: 'checkout', kept };
}

// ---------------------------------------------------------------- matrix fixtures

function installHooks(u, repo, markers) {
  fs.mkdirSync(markers, { recursive: true });
  // Not `rev-parse --git-path hooks`: that follows core.hooksPath, which our runner
  // points at the shared empty dir — the repository's own hooks live here.
  const hooksDir = path.join(path.resolve(repo, u.git(repo, ['rev-parse', '--git-common-dir'])), 'hooks');
  fs.mkdirSync(hooksDir, { recursive: true });
  const log = path.join(markers, 'fired').replace(/\\/g, '/');
  for (const name of ['post-checkout', 'post-merge', 'post-index-change', 'reference-transaction', 'pre-push', 'pre-commit', 'post-commit', 'post-rewrite', 'pre-auto-gc']) {
    const p = path.join(hooksDir, name);
    fs.writeFileSync(p, `#!/bin/sh\necho "${name} in ${path.basename(repo)}" >> '${log}'\nexit 0\n`);
    if (!IS_WIN) fs.chmodSync(p, 0o755);
  }
}
const firedHooks = (markers) => {
  const f = path.join(markers, 'fired');
  return fs.existsSync(f) ? [...new Set(fs.readFileSync(f, 'utf8').split('\n').filter(Boolean))] : [];
};

function world(root, name) {
  const dir = path.join(root, name);
  fs.mkdirSync(dir, { recursive: true });
  return {
    dir,
    origin: path.join(dir, 'origin.git'),
    subOrigin: path.join(dir, 'sub-origin.git'),
    seed: path.join(dir, 'seed'),
    src: path.join(dir, 'src'),
    dst: path.join(dir, 'dst'),
    move: path.join(dir, 'move'),
    appdata: path.join(dir, 'appdata', 'machines', 'worktrees'),
    markers: path.join(dir, 'markers'),
  };
}

function writeFiles(dir, files) {
  for (const [rel, v] of Object.entries(files)) {
    const p = at(dir, rel);
    if (v === null) { fs.rmSync(p, { force: true }); continue; }
    fs.mkdirSync(path.dirname(p), { recursive: true });
    if (v && typeof v === 'object' && !Buffer.isBuffer(v) && 'symlink' in v) {
      fs.rmSync(p, { force: true });
      fs.symlinkSync(v.symlink, p);
    } else if (v && typeof v === 'object' && !Buffer.isBuffer(v) && 'exec' in v) {
      fs.writeFileSync(p, v.content);
      if (!IS_WIN) fs.chmodSync(p, v.exec ? 0o755 : 0o644);
    } else fs.writeFileSync(p, v);
  }
}

/** origin.git with the given commits on main (each: files, optional before(seed)). */
function seedOrigin(u, w, commits, { seedConfig = {} } = {}) {
  u.git(w.dir, ['init', '--bare', '-q', w.origin]);
  u.git(w.dir, ['clone', '-q', w.origin, w.seed]);
  for (const [k, v] of Object.entries(seedConfig)) u.git(w.seed, ['config', k, v]);
  for (const c of commits) {
    writeFiles(w.seed, c.files || {});
    c.before?.(w.seed);
    u.git(w.seed, ['add', '-A']);
    u.git(w.seed, ['commit', '-q', '--allow-empty', '-m', c.msg || 'commit']);
  }
  u.git(w.seed, ['push', '-q', 'origin', 'HEAD:refs/heads/main']);
}

function clone(u, w, which, cfg = {}, extra = [], url = w.origin) {
  const args = ['clone', '-q'];
  for (const [k, v] of Object.entries(cfg)) args.push('-c', `${k}=${v}`);
  u.git(w.dir, [...args, ...extra, url, w[which]]);
  return w[which];
}

const BASE_FILES = {
  'a.txt': 'alpha\n', 'b.txt': 'bravo\n', 'c.txt': 'charlie\n', 'd.txt': 'delta\n', 'e.txt': 'echo\n',
  'src/main.js': 'console.log(1);\n', 'old-name.txt': 'rename me\n', 'disk-rename.txt': 'moved on disk\n',
  '.gitignore': '*.log\nbuild/\nnode_modules/\n',
};

function dirtyMix(u, s) {
  put(s, 'a.txt', 'alpha 2\n'); u.git(s, ['add', 'a.txt']); // staged
  put(s, 'b.txt', 'bravo 2\n'); u.git(s, ['add', 'b.txt']); put(s, 'b.txt', 'bravo 3\n'); // staged, then changed again
  put(s, 'c.txt', 'charlie 2\n'); // unstaged
  u.git(s, ['rm', '-q', 'd.txt']); // staged delete
  fs.rmSync(at(s, 'e.txt')); // unstaged delete
  u.git(s, ['mv', 'old-name.txt', 'new-name.txt']); // staged rename
  fs.renameSync(at(s, 'disk-rename.txt'), at(s, 'moved-on-disk.txt')); // unstaged rename
  put(s, 'f.txt', 'foxtrot\n'); u.git(s, ['add', 'f.txt']); // staged new
  put(s, 'g.txt', 'golf\n'); put(s, 'notes/todo.md', '- [ ] move\n'); // untracked
  put(s, 'debug.log', 'ignored\n'); put(s, 'build/out.bin', Buffer.from([0, 1, 2, 3])); // ignored
}

const ignoredAbsent = (paths) => (w, row) => {
  const where = row.targetPath;
  if (!where) return;
  for (const p of paths) if (fs.existsSync(at(where, p))) row.problems.push(`ignored file travelled: ${p}`);
};

let largeOrigin = null;
function buildLargeOrigin(u, root) {
  if (largeOrigin) return largeOrigin;
  const t = now();
  const w = world(root, '_large-origin');
  const files = {};
  for (let i = 0; i < 1900; i++) files[`pkg${i % 40}/mod${i}.js`] = textBlob(1000 + i, 20_000);
  for (let i = 0; i < 100; i++) files[`assets/img${i}.bin`] = binBlob(5000 + i, 120_000);
  seedOrigin(u, w, [{ files, msg: 'large base' }]);
  const size = Object.values(files).reduce((n, v) => n + v.length, 0);
  largeOrigin = { origin: w.origin, bytes: size, files: Object.keys(files).length, seedMs: since(t) };
  return largeOrigin;
}

function lfsConfig() {
  return {
    'filter.lfs.clean': 'git-lfs clean -- %f',
    'filter.lfs.smudge': 'git-lfs smudge -- %f',
    'filter.lfs.process': 'git-lfs filter-process',
    'filter.lfs.required': 'true',
  };
}

/*
 * Each case: build(w, u, run) makes origin, S (w.src) and T (w.dst);
 * expect per landing: pass | stop | limit | observe, with the limits allowed.
 */
const CASES = [
  {
    id: 'mix', title: 'staged + unstaged + untracked, deleted, renamed (staged and on disk), ignored', undo: true,
    build(w, u) { seedOrigin(u, w, [{ files: BASE_FILES }]); clone(u, w, 'src'); clone(u, w, 'dst'); dirtyMix(u, w.src); },
    expect: { checkout: 'pass', worktree: 'pass' },
    after: ignoredAbsent(['debug.log', 'build/out.bin']),
  },
  {
    id: 'binary', title: 'binary file changed, new binary untracked',
    build(w, u) {
      seedOrigin(u, w, [{ files: { 'img.bin': binBlob(1, 4096), 'a.txt': 'a\n' } }]);
      clone(u, w, 'src'); clone(u, w, 'dst');
      put(w.src, 'img.bin', binBlob(2, 4096)); put(w.src, 'new.bin', binBlob(3, 8192));
    },
    expect: { checkout: 'pass', worktree: 'pass' },
  },
  {
    id: 'exec-bit', title: 'exec bit: unstaged chmod, staged chmod, new executable (target fileMode=true)',
    skip: (env) => (env.execBit ? null : 'no exec bits on this file system'),
    build(w, u) { buildExec(w, u, {}); },
    expect: { checkout: 'pass', worktree: 'pass' },
  },
  {
    id: 'exec-bit-to-filemode-false', title: 'the same, target core.fileMode=false (as on Windows)',
    skip: (env) => (env.execBit ? null : 'no exec bits on this file system'),
    build(w, u) { buildExec(w, u, { 'core.fileMode': 'false' }); },
    expect: { checkout: 'limit', worktree: 'limit', limits: ['filemode'] },
  },
  {
    id: 'paths', title: 'spaces, non-ASCII, NFD names; staged, unstaged, untracked',
    build(w, u) {
      seedOrigin(u, w, [{ files: { 'über/straße.txt': 'v1\n', 'plain.txt': 'p\n' } }]);
      clone(u, w, 'src'); clone(u, w, 'dst');
      const s = w.src;
      put(s, 'dir with space/naïve café.txt', 'nfc name\n');
      put(s, '日本語/ファイル.md', '# こんにちは\n'); u.git(s, ['add', '日本語/ファイル.md']);
      put(s, 'über/straße.txt', 'v2\n');
      put(s, 'café nfd.txt', 'nfd name\n');
    },
    expect: { checkout: 'pass', worktree: 'pass' },
  },
  {
    id: 'ignored', title: 'ignored files stay; a tracked file matching .gitignore travels',
    build(w, u) {
      seedOrigin(u, w, [{ files: { '.gitignore': '*.log\nnode_modules/\n', 'a.txt': 'a\n' }, before: (seed) => { put(seed, 'keep.log', 'tracked v1\n'); u.git(seed, ['add', '-f', 'keep.log']); } }]);
      clone(u, w, 'src'); clone(u, w, 'dst');
      put(w.src, 'keep.log', 'tracked v2\n'); put(w.src, 'trace.log', 'x\n'); put(w.src, 'node_modules/x/index.js', 'module.exports = 1;\n');
    },
    expect: { checkout: 'pass', worktree: 'pass' },
    after: ignoredAbsent(['trace.log', 'node_modules/x/index.js']),
  },
  {
    id: 'autocrlf-false-to-true', title: 'LF source (autocrlf=false) → target autocrlf=true',
    build(w, u) {
      seedOrigin(u, w, [{ files: { 'a.txt': 'one\ntwo\n', 'src/b.js': 'let x = 1;\n' } }]);
      clone(u, w, 'src'); clone(u, w, 'dst', { 'core.autocrlf': 'true' });
      put(w.src, 'a.txt', 'one\ntwo\nthree\n'); put(w.src, 'c.md', '# c\n');
      put(w.src, 'src/b.js', 'let x = 2;\n'); u.git(w.src, ['add', 'src/b.js']);
    },
    expect: { checkout: 'pass', worktree: 'pass' },
    after: (w, row) => { if (!row.rawKinds?.includes('eol')) row.notes.push('no line-ending differences seen'); },
  },
  {
    id: 'autocrlf-true-to-false', title: 'CRLF working tree (autocrlf=true, as on Windows) → target autocrlf=false',
    build(w, u) {
      seedOrigin(u, w, [{ files: { 'a.txt': 'one\ntwo\n', 'lf-edit.txt': 'x\n' } }]);
      clone(u, w, 'src', { 'core.autocrlf': 'true' }); clone(u, w, 'dst');
      put(w.src, 'a.txt', 'one\r\ntwo\r\nthree\r\n'); put(w.src, 'c.md', '# c\r\n'); put(w.src, 'lf-edit.txt', 'x\ny\n');
    },
    expect: { checkout: 'pass', worktree: 'pass' },
  },
  {
    id: 'gitattributes-eol-lf', title: '`* text=auto eol=lf` committed; target autocrlf=true keeps LF',
    build(w, u) {
      seedOrigin(u, w, [{ files: { '.gitattributes': '* text=auto eol=lf\n', 'a.txt': 'one\n' } }]);
      clone(u, w, 'src'); clone(u, w, 'dst', { 'core.autocrlf': 'true' });
      put(w.src, 'a.txt', 'one\ntwo\n'); put(w.src, 'b.txt', 'b\n');
    },
    expect: { checkout: 'pass', worktree: 'pass' },
    after: (w, row) => { if (row.rawKinds?.length) row.problems.push(`raw differences with eol=lf: ${row.rawKinds.join(', ')}`); },
  },
  {
    id: 'crlf-committed', title: 'a CRLF file committed without attributes, changed; target autocrlf=true',
    build(w, u) {
      seedOrigin(u, w, [{ files: { 'win.txt': 'line1\r\nline2\r\n', 'unix.txt': 'u\n' } }]);
      clone(u, w, 'src'); clone(u, w, 'dst', { 'core.autocrlf': 'true' });
      put(w.src, 'win.txt', 'line1\r\nline2 changed\r\n');
    },
    expect: { checkout: 'pass', worktree: 'pass' },
  },
  {
    id: 'behind', title: 'target branch behind the base: fast-forward', undo: true,
    build(w, u) {
      seedOrigin(u, w, [{ files: { 'a.txt': '1\n' } }]);
      clone(u, w, 'src'); clone(u, w, 'dst');
      put(w.src, 'a.txt', '2\n'); put(w.src, 'b.txt', 'b\n'); u.git(w.src, ['add', '-A']); u.git(w.src, ['commit', '-q', '-m', 'c2 on S']);
      put(w.src, 'a.txt', '3\n'); put(w.src, 'c.txt', 'c\n');
    },
    expect: { checkout: 'pass', worktree: 'pass' },
  },
  {
    id: 'diverged', title: 'target branch has a commit the base lacks: stop, worktree offered',
    build(w, u) {
      seedOrigin(u, w, [{ files: { 'a.txt': '1\n' } }]);
      clone(u, w, 'src'); clone(u, w, 'dst');
      put(w.dst, 't.txt', 't\n'); u.git(w.dst, ['add', '-A']); u.git(w.dst, ['commit', '-q', '-m', 'only on T']);
      put(w.src, 's.txt', 's\n'); u.git(w.src, ['add', '-A']); u.git(w.src, ['commit', '-q', '-m', 'only on S']);
      put(w.src, 'a.txt', '2\n');
    },
    expect: { checkout: 'stop', worktree: 'pass' },
  },
  {
    id: 'dirty-target', title: 'target checkout has changes of its own: stop, worktree offered',
    build(w, u) {
      seedOrigin(u, w, [{ files: { 'a.txt': '1\n', 'b.txt': 'b\n' } }]);
      clone(u, w, 'src'); clone(u, w, 'dst');
      put(w.dst, 'b.txt', 'changed on T\n');
      put(w.src, 'a.txt', '2\n');
    },
    expect: { checkout: 'stop', worktree: 'pass' },
    after: (w, row) => { if (fs.readFileSync(at(w.dst, 'b.txt'), 'utf8') !== 'changed on T\n') row.problems.push("T's own change was lost"); },
  },
  {
    id: 'target-other-branch', title: 'target on another branch: switched to the source branch', undo: true,
    build(w, u) {
      seedOrigin(u, w, [{ files: { 'a.txt': '1\n' } }]);
      clone(u, w, 'src'); clone(u, w, 'dst');
      u.git(w.dst, ['switch', '-q', '-c', 'feature']); put(w.dst, 'f.txt', 'f\n'); u.git(w.dst, ['add', '-A']); u.git(w.dst, ['commit', '-q', '-m', 'feature']);
      put(w.src, 'a.txt', '2\n'); put(w.src, 'n.txt', 'n\n');
    },
    expect: { checkout: 'pass', worktree: 'pass' },
  },
  {
    id: 'unborn', title: 'source branch has no commit yet; target an empty repo', undo: true,
    build(w, u) {
      u.git(w.dir, ['init', '--bare', '-q', w.origin]);
      for (const r of [w.src, w.dst]) { u.git(w.dir, ['init', '-q', r]); u.git(r, ['remote', 'add', 'origin', w.origin]); }
      put(w.src, 'a.txt', 'staged\n'); u.git(w.src, ['add', 'a.txt']); put(w.src, 'a.txt', 'staged then changed\n');
      put(w.src, 'b.txt', 'untracked\n');
    },
    expect: { checkout: 'pass', worktree: 'pass' },
  },
  {
    id: 'detached', title: 'source on a detached HEAD', undo: true,
    build(w, u) {
      seedOrigin(u, w, [{ files: { 'a.txt': '1\n' } }, { files: { 'a.txt': '2\n' } }]);
      clone(u, w, 'src'); clone(u, w, 'dst');
      u.git(w.src, ['switch', '-q', '--detach', 'HEAD~1']);
      put(w.src, 'a.txt', 'on detached\n'); put(w.src, 'x.txt', 'x\n');
    },
    expect: { checkout: 'pass', worktree: 'pass' },
  },
  {
    id: 'submodule-present', title: 'submodule moved to a commit the target has',
    build(w, u) { buildSubmodule(w, u, true); },
    expect: { checkout: 'pass', worktree: 'limit', limits: ['submodule'] },
  },
  {
    id: 'submodule-missing', title: 'submodule moved to a commit the target lacks',
    build(w, u) { buildSubmodule(w, u, false); },
    expect: { checkout: 'limit', worktree: 'limit', limits: ['submodule'] },
  },
  {
    id: 'symlink', title: 'tracked symlink retargeted, new untracked symlink (target symlinks=true)',
    skip: (env) => (env.symlinks ? null : 'cannot create symlinks here'),
    build(w, u) { buildSymlink(w, u, {}); },
    expect: { checkout: 'pass', worktree: 'pass' },
  },
  {
    id: 'symlink-to-no-symlinks', title: 'the same, target core.symlinks=false (as on Windows by default)',
    skip: (env) => (env.symlinks ? null : 'cannot create symlinks here'),
    build(w, u) { buildSymlink(w, u, { 'core.symlinks': 'false' }); },
    expect: { checkout: 'limit', worktree: 'limit', limits: ['symlink-as-file'] },
  },
  {
    id: 'case-rename-staged', title: '`git mv Readme.md README.md` (staged)',
    build(w, u) {
      seedOrigin(u, w, [{ files: { 'Readme.md': 'r\n' } }]);
      clone(u, w, 'src'); clone(u, w, 'dst');
      u.git(w.src, ['mv', 'Readme.md', 'README.md']);
    },
    expect: { checkout: 'pass', worktree: 'pass' },
  },
  {
    id: 'case-rename-unstaged', title: 'Readme.md renamed to README.md on disk only',
    build(w, u) {
      seedOrigin(u, w, [{ files: { 'Readme.md': 'r\n' } }]);
      clone(u, w, 'src'); clone(u, w, 'dst');
      fs.renameSync(at(w.src, 'Readme.md'), at(w.src, 'tmp-name.md'));
      fs.renameSync(at(w.src, 'tmp-name.md'), at(w.src, 'README.md'));
    },
    expect: (env) => (env.caseInsensitive
      ? { checkout: 'limit', worktree: 'limit', limits: ['case'] }
      : { checkout: 'pass', worktree: 'pass' }),
  },
  {
    id: 'case-collision', title: 'x/File.txt and x/file.txt in one tree (made on a case-sensitive machine)',
    build(w, u) {
      seedOrigin(u, w, [{ files: { 'x/File.txt': 'upper\n' }, before: (seed) => {
        const blob = u.git(seed, ['hash-object', '-w', '--stdin'], { input: 'lower\n' });
        u.git(seed, ['add', '-A']);
        u.git(seed, ['update-index', '--add', '--cacheinfo', `100644,${blob},x/file.txt`]);
      } }]);
      clone(u, w, 'src'); clone(u, w, 'dst');
      put(w.src, 'other.txt', 'o\n');
    },
    expect: { checkout: 'observe', worktree: 'observe' },
    after: (w, row) => { if (row.warnings && !row.warnings.includes('case-collision')) row.problems.push('no case-collision warning'); },
  },
  {
    id: 'windows-names', title: 'names Windows cannot hold (aux.txt, a trailing dot, ?)',
    skip: () => (IS_WIN ? 'these names cannot be created on Windows' : null),
    build(w, u) {
      seedOrigin(u, w, [{ files: { 'a.txt': 'a\n' } }]);
      clone(u, w, 'src'); clone(u, w, 'dst');
      put(w.src, 'aux.txt', 'reserved\n'); put(w.src, 'trailing-dot.', 'dot\n'); put(w.src, 'what?.txt', 'q\n'); put(w.src, 'con/readme.txt', 'c\n');
    },
    expect: { checkout: 'pass', worktree: 'pass' },
    after: (w, row) => { if (row.warnings && !row.warnings.includes('windows-names')) row.problems.push('no windows-names warning'); },
  },
  {
    id: 'long-path', title: 'a 250-character path (Windows MAX_PATH)',
    build(w, u) {
      seedOrigin(u, w, [{ files: { 'a.txt': 'a\n' } }]);
      clone(u, w, 'src'); clone(u, w, 'dst');
      put(w.src, 'deep/' + 'abcdefghij/'.repeat(22) + 'file.txt', 'deep\n');
    },
    expect: (env) => (env.platform === 'win32' ? { checkout: 'observe', worktree: 'observe' } : { checkout: 'pass', worktree: 'pass' }),
    after: (w, row) => { if (row.warnings && !row.warnings.includes('long-path')) row.problems.push('no long-path warning'); },
  },
  {
    id: 'intent-to-add', title: '`git add -N` entry',
    build(w, u) {
      seedOrigin(u, w, [{ files: { 'a.txt': 'a\n' } }]);
      clone(u, w, 'src'); clone(u, w, 'dst');
      put(w.src, 'h.txt', 'intent\n'); u.git(w.src, ['add', '-N', 'h.txt']); put(w.src, 'a.txt', 'a2\n');
    },
    expect: { checkout: 'limit', worktree: 'limit', limits: ['ita'] },
  },
  {
    id: 'shallow-target', title: 'target is a shallow clone (--depth 1); the change restores an old blob',
    build(w, u) {
      seedOrigin(u, w, [{ files: { 'a.txt': 'v1\n', 'k.txt': 'k\n' } }, { files: { 'a.txt': 'v2\n' } }, { files: { 'b.txt': 'b\n' } }]);
      clone(u, w, 'src');
      clone(u, w, 'dst', {}, ['--depth', '1'], pathToFileURL(w.origin).href);
      put(w.src, 'a.txt', 'v1\n'); put(w.src, 'n.txt', 'n\n');
    },
    expect: { checkout: 'observe', worktree: 'observe' },
  },
  {
    id: 'hooks-diagnostic', title: 'the mix case with the repo hooks NOT overridden (which hooks would run)', hooksDiag: true,
    build(w, u) { seedOrigin(u, w, [{ files: BASE_FILES }]); clone(u, w, 'src'); clone(u, w, 'dst'); dirtyMix(u, w.src); },
    expect: { checkout: 'observe', worktree: 'observe' },
  },
  {
    id: 'lfs-carried', title: 'Git LFS file changed and added; objects carried in the move dir',
    skip: (env) => (env.lfs ? null : 'skipped: git-lfs not installed'),
    build(w, u) { buildLfs(w, u); },
    expect: { checkout: 'pass', worktree: 'pass' },
  },
  {
    id: 'lfs-not-carried', title: 'the same without carrying LFS objects (they are on S only)', carryLfs: false,
    skip: (env) => (env.lfs ? null : 'skipped: git-lfs not installed'),
    build(w, u) { buildLfs(w, u); },
    expect: { checkout: 'observe', worktree: 'observe' },
  },
  {
    id: 'round-trip', title: 'S → T, the session works on T (edits, a commit), then back T → S into the parked checkout',
    build(w, u) { seedOrigin(u, w, [{ files: BASE_FILES }]); clone(u, w, 'src'); clone(u, w, 'dst'); dirtyMix(u, w.src); },
    expect: { checkout: 'pass', worktree: 'pass' },
    after: (w, row, x) => bringBack(w, row, x, { editSource: false }),
  },
  {
    id: 'round-trip-source-edited', title: 'the same, but S was edited while the session was away: back is refused',
    build(w, u) { seedOrigin(u, w, [{ files: BASE_FILES }]); clone(u, w, 'src'); clone(u, w, 'dst'); dirtyMix(u, w.src); },
    expect: { checkout: 'pass', worktree: 'pass' },
    after: (w, row, x) => bringBack(w, row, x, { editSource: true }),
  },
  {
    id: 'large', title: '2,000 files / ~50 MB; 50 changed, 20+2 new, 5 deleted; then an incremental move', large: true,
    build(w, u, run) {
      const L = buildLargeOrigin(u, run.root);
      clone(u, w, 'src', {}, [], L.origin); clone(u, w, 'dst', {}, [], L.origin);
      w.origin = L.origin; // shared by every large run (the hidden-ref check looks here)
      const s = w.src;
      for (let i = 0; i < 50; i++) fs.appendFileSync(at(s, `pkg${i % 40}/mod${i}.js`), `// changed ${i}\n`);
      for (let i = 0; i < 20; i++) put(s, `new/file${i}.js`, textBlob(9000 + i, 10_000));
      put(s, 'new/big1.bin', binBlob(77, 1_000_000)); put(s, 'new/big2.bin', binBlob(78, 1_000_000));
      for (let i = 100; i < 105; i++) fs.rmSync(at(s, `pkg${i % 40}/mod${i}.js`));
      w.largeInfo = L;
    },
    expect: { checkout: 'pass', worktree: 'pass' },
    after: largeExtras,
  },
];

function buildExec(w, u, dstCfg) {
  seedOrigin(u, w, [{ files: { 'run.sh': { content: 'echo run\n', exec: false }, 'stage.sh': { content: 'echo stage\n', exec: false }, 'a.txt': 'a\n' } }]);
  clone(u, w, 'src'); clone(u, w, 'dst', dstCfg);
  fs.chmodSync(at(w.src, 'run.sh'), 0o755); // unstaged mode change
  fs.chmodSync(at(w.src, 'stage.sh'), 0o755); u.git(w.src, ['add', 'stage.sh']); // staged mode change
  put(w.src, 'tool.sh', 'echo tool\n', { exec: true }); // new executable
}

function buildSymlink(w, u, dstCfg) {
  seedOrigin(u, w, [{ files: { 'a.txt': 'a\n', 'b.txt': 'b\n', 'link-a': { symlink: 'a.txt' } } }]);
  clone(u, w, 'src'); clone(u, w, 'dst', dstCfg);
  writeFiles(w.src, { 'link-a': { symlink: 'b.txt' }, 'link-b': { symlink: 'b.txt' } });
}

function buildSubmodule(w, u, present) {
  u.git(w.dir, ['init', '--bare', '-q', w.subOrigin]);
  const subSeed = path.join(w.dir, 'sub-seed');
  u.git(w.dir, ['clone', '-q', w.subOrigin, subSeed]);
  put(subSeed, 'lib.txt', 'v1\n'); u.git(subSeed, ['add', '-A']); u.git(subSeed, ['commit', '-q', '-m', 'lib v1']);
  u.git(subSeed, ['push', '-q', 'origin', 'HEAD:refs/heads/main']);
  seedOrigin(u, w, [{ files: { 'a.txt': 'a\n' }, before: (seed) => u.git(seed, ['submodule', 'add', '-q', w.subOrigin, 'lib']) }]);
  for (const r of ['src', 'dst']) { clone(u, w, r); u.git(w[r], ['submodule', 'update', '-q', '--init']); }
  const sLib = at(w.src, 'lib');
  u.git(sLib, ['switch', '-q', '-c', 'work']);
  put(sLib, 'lib.txt', 'v2\n'); u.git(sLib, ['commit', '-q', '-am', 'lib v2']);
  if (present) {
    u.git(sLib, ['push', '-q', 'origin', 'HEAD:refs/heads/main']);
    u.git(at(w.dst, 'lib'), ['fetch', '-q', 'origin']);
  }
  put(w.src, 'a.txt', 'a2\n');
}

function buildLfs(w, u) {
  seedOrigin(u, w, [{ files: { 'a.txt': 'a\n' }, before: (seed) => {
    u.git(seed, ['lfs', 'track', '*.bin']);
    put(seed, 'big.bin', binBlob(11, 100_000));
  } }], { seedConfig: lfsConfig() });
  u.git(w.seed, ['lfs', 'push', '--all', 'origin']);
  clone(u, w, 'src', lfsConfig()); clone(u, w, 'dst', lfsConfig());
  put(w.src, 'big.bin', binBlob(12, 100_000)); put(w.src, 'new.bin', binBlob(13, 50_000));
}

/** The session continues on T, then moves back to S (checkout landing on S). */
function bringBack(w, row, x, { editSource }) {
  if (row.outcome !== 'pass') return;
  const { tool, meta, ap } = x;
  const t = ap.path; // T's checkout or worktree, where the session now runs
  put(t, 'g.txt', 'golf, continued on T\n');
  put(t, 'c.txt', 'charlie from T\n'); tool.git(t, ['add', 'c.txt']);
  tool.git(t, ['commit', '-q', '-m', 'work done on T']);
  put(t, 'h.txt', 'hotel, new on T\n'); put(t, 'a.txt', 'alpha from T, unstaged\n');
  if (editSource) put(w.src, 'g.txt', 'golf, edited on S meanwhile\n');
  const back = path.join(w.dir, 'move-back');
  const h = row.transfer === 'bundle' ? haves(tool, w.src) : { haves: [] };
  const m2 = snapshot(tool, t, back, { session: meta.session, epoch: 2, haves: h.haves, transfer: row.transfer, remote: 'origin', machine: 'dst-machine' });
  const a2 = apply(tool, w.src, back, { landing: 'checkout' });
  const info = `back: ${m2.bundle ? `bundle ${m2.bundle.size} B, chained ${m2.chained.length}; ` : ''}`;
  if (editSource) {
    if (a2.outcome === 'stop') row.notes.push(`${info}stopped as designed (${a2.reason})`);
    else row.problems.push(`back should have stopped: S was edited meanwhile (${a2.outcome})`);
    return;
  }
  if (a2.outcome !== 'applied') { row.problems.push(`back: ${a2.outcome} ${a2.reason ?? ''}`); return; }
  const v = verify(tool, w.src, back);
  if (v.outcome !== 'pass') row.problems.push(`back: ${v.outcome} ${v.fails.join('; ')}`);
  else row.notes.push(`${info}landed on the parked checkout (epoch ${a2.parked}), ${a2.action}, verified`);
  const r = undo(tool, w.src, back);
  const stillParked = parkedState(tool, repoInfo(tool, w.src), meta.session);
  if (!stillParked || r.kept.length) row.problems.push(`undo of the way back did not restore the parked state${r.kept.length ? ` (kept ${r.kept.join(', ')})` : ''}`);
  else row.notes.push('undo of the way back restored the parked state');
}

function largeExtras(w, row, x) {
  if (row.transfer !== 'bundle' || row.landing !== 'checkout' || row.outcome !== 'pass') return;
  const { tool, meta } = x;
  const L = w.largeInfo;
  const extra = { repoFiles: L.files, repoBytes: L.bytes, seedMs: L.seedMs };
  // a full bundle (T has nothing): its size and time
  let t = now();
  const full = path.join(w.dir, 'full.bundle');
  tool.git(w.src, ['bundle', 'create', '-q', full, meta.refs.snap, meta.refs.index]);
  extra.fullBundle = { bytes: fs.statSync(full).size, ms: since(t) };
  // incremental: S commits, changes a little more, moves again; T still has epoch 1
  const s = w.src;
  tool.git(s, ['add', '-A']);
  tool.git(s, ['commit', '-q', '-m', 'S commits the moved work']);
  for (let i = 200; i < 205; i++) fs.appendFileSync(at(s, `pkg${i % 40}/mod${i}.js`), `// again ${i}\n`);
  put(s, 'new/again.txt', 'again\n');
  const move2 = path.join(w.dir, 'move2');
  t = now();
  const h = haves(tool, w.dst);
  const meta2 = snapshot(tool, s, move2, { session: meta.session, epoch: 2, haves: h.haves, transfer: 'bundle', machine: 'src-machine' });
  extra.incremental = { snapshotMs: since(t), bundleBytes: meta2.bundle.size, havesUsed: meta2.bundle.havesUsed, chained: meta2.chained.length };
  const unchained = snapshot(tool, s, path.join(w.dir, 'move2-unchained'), { session: meta.session, epoch: 3, haves: h.haves, transfer: 'bundle', machine: 'src-machine', chain: false });
  extra.incremental.unchainedBundleBytes = unchained.bundle.size;
  t = now();
  const ap2 = apply(tool, w.dst, move2, { landing: 'worktree', worktreeRoot: w.appdata + '-2' });
  const v2 = verify(tool, ap2.path, move2);
  extra.incremental.applyVerifyMs = since(t);
  extra.incremental.outcome = v2.outcome;
  if (v2.outcome !== 'pass') row.problems.push(`incremental move: ${v2.outcome} ${v2.fails.join('; ')}`);
  row.large = extra;
}

// ---------------------------------------------------------------- matrix runner

function environmentInfo(u) {
  const t = tmpDir('mc-s2-env-');
  try {
    fs.writeFileSync(path.join(t, 'CaseTest'), 'x');
    const caseInsensitive = fs.existsSync(path.join(t, 'casetest'));
    let symlinks = true;
    try { fs.symlinkSync('CaseTest', path.join(t, 'link')); } catch { symlinks = false; }
    let execBit = false;
    if (!IS_WIN) {
      fs.writeFileSync(path.join(t, 'x.sh'), 'x');
      fs.chmodSync(path.join(t, 'x.sh'), 0o755);
      execBit = !!(fs.statSync(path.join(t, 'x.sh')).mode & 0o100);
    }
    const lfs = u.tryGit(t, ['lfs', 'version']) ? u.git(t, ['lfs', 'version']) : null;
    return {
      git: u.git(t, ['--version']), node: process.version, platform: process.platform, arch: process.arch,
      os: `${os.type()} ${os.release()}`, caseInsensitive, symlinks, execBit, lfs, tmp: os.tmpdir(),
    };
  } finally { rmrf(t); }
}

/** Do plain `git write-tree` / `git status` rewrite the index they read? */
function diagnostics(u, root) {
  const w = world(root, '_diagnostics');
  seedOrigin(u, w, [{ files: { 'a.txt': 'a\n', 'b.txt': 'b\n', 'sub/c.txt': 'c\n' } }]);
  clone(u, w, 'src');
  put(w.src, 'a.txt', 'a2\n'); u.git(w.src, ['add', 'a.txt']);
  put(w.src, 'b.txt', 'b2\n');
  const info = repoInfo(u, w.src);
  const copy = (name) => { const p = path.join(w.dir, name); fs.copyFileSync(info.indexPath, p); return p; };
  const i1 = copy('index-wt');
  const before1 = sha256(fs.readFileSync(i1));
  u.git(w.src, ['write-tree'], { env: { GIT_INDEX_FILE: i1 } });
  const writeTreeRewrites = sha256(fs.readFileSync(i1)) !== before1;
  const i2 = copy('index-st');
  const future = new Date(Date.now() + 5000);
  fs.utimesSync(at(w.src, 'sub/c.txt'), future, future);
  const before2 = sha256(fs.readFileSync(i2));
  u.git(w.src, ['status', '--porcelain'], { env: { GIT_INDEX_FILE: i2, GIT_OPTIONAL_LOCKS: '1' } });
  const statusRewrites = sha256(fs.readFileSync(i2)) !== before2;
  const i3 = copy('index-st-nolock');
  const before3 = sha256(fs.readFileSync(i3));
  u.git(w.src, ['status', '--porcelain'], { env: { GIT_INDEX_FILE: i3, GIT_OPTIONAL_LOCKS: '0' } });
  const statusNoLocksRewrites = sha256(fs.readFileSync(i3)) !== before3;
  return {
    writeTreeRewritesIndex: writeTreeRewrites,
    statusRewritesIndex: statusRewrites,
    statusWithoutOptionalLocksRewritesIndex: statusNoLocksRewrites,
  };
}

function expectationFor(c, landing, env) {
  const e = typeof c.expect === 'function' ? c.expect(env) : c.expect;
  return { want: e[landing], limits: e.limits || [] };
}

function runCase(c, transfer, landing, run) {
  if (fs.readdirSync(ensureToolDirs().emptyHooks).length) throw new Error('the empty hooks dir is not empty');
  const w = world(run.root, `${c.id}-${transfer}-${landing}`);
  const u = run.u;
  const tool = makeCtx({ isolated: true, hooks: !!c.hooksDiag });
  const exp = expectationFor(c, landing, run.env);
  const row = { case: c.id, title: c.title, transfer, landing, expected: exp.want, outcome: null, limits: [], fails: [], problems: [], notes: [], timings: {} };
  const t0 = now();
  let meta = null;
  let ap = null;
  try {
    c.build(w, u, run);
    installHooks(u, w.src, w.markers);
    installHooks(u, w.dst, w.markers);
    const h = transfer === 'bundle' ? haves(tool, w.dst) : { haves: [], shallow: false };
    if (h.shallow) row.notes.push('target is shallow');
    let t = now();
    meta = snapshot(tool, w.src, w.move, { session: `ses_${c.id.replace(/[^a-z0-9]/gi, '')}`, epoch: 1, haves: h.haves, transfer, remote: 'origin', machine: 'src-machine', carryLfs: c.carryLfs });
    row.timings.snapshotMs = since(t);
    row.bundleBytes = meta.bundle?.size ?? null;
    row.warnings = meta.warnings.map((x) => x.kind);
    if (meta.wtCheck?.recomputed) row.notes.push(`working tree re-hashed: the index copy missed ${meta.wtCheck.first.missing.join(', ')}`);
    if (!meta.untouched.ok) row.problems.push(`source checkout changed by the snapshot: ${meta.untouched.changed.join(', ')}`);
    const dstBefore = probe(u, w.dst);
    t = now();
    ap = apply(tool, w.dst, w.move, { landing, worktreeRoot: w.appdata });
    row.timings.applyMs = since(t);
    row.targetPath = ap.path ?? null;
    if (ap.outcome === 'stop') {
      row.outcome = 'stop';
      row.notes.push(`stop: ${ap.reason}; offered: ${ap.offer}`);
      const changed = compareProbes(dstBefore, probe(u, w.dst), ['status', 'indexSha', 'sym', 'head', 'refs', 'filesHash', 'worktrees']);
      if (changed.length) row.problems.push(`target changed although the landing stopped: ${changed.join(', ')}`);
    } else {
      t = now();
      const v = verify(tool, ap.path, w.move);
      row.timings.verifyMs = since(t);
      row.outcome = v.outcome;
      row.limits = v.limits;
      row.fails = v.fails;
      row.rawKinds = v.rawKinds;
      if (ap.submodules?.length) row.notes.push(`submodules: ${ap.submodules.map((s) => `${s.path} ${s.state}`).join(', ')}`);
      if (landing === 'worktree') {
        const changed = compareProbes(dstBefore, probe(u, w.dst), ['status', 'indexSha', 'sym', 'head', 'filesHash']);
        if (changed.length) row.problems.push(`target checkout changed by a worktree landing: ${changed.join(', ')}`);
      }
      if (c.undo && v.outcome === 'pass') {
        const r = undo(tool, w.dst, w.move);
        const changed = compareProbes(dstBefore, probe(u, w.dst), ['status', 'indexTree', 'sym', 'head', 'refs', 'filesHash', 'worktrees'], { ignoreRefPrefixes: ['refs/mother-claude/'] });
        row.undo = changed.length ? `NOT restored: ${changed.join(', ')}` : r.kept.length ? `restored, kept ${r.kept.length}` : 'restored';
        if (changed.length) row.problems.push(`undo left the target different: ${changed.join(', ')}`);
      }
    }
    if (transfer === 'remote') {
      const left = u.git(w.dir, ['ls-remote', w.origin, 'refs/mother-claude/*']);
      const fetched = ap?.fetched === true;
      if (left && fetched) row.problems.push('hidden refs left on the remote after the target fetched them');
      if (left && !fetched) row.notes.push('hidden refs kept on the remote: the move is pending');
    }
    const fired = firedHooks(w.markers);
    if (c.hooksDiag) row.hooksFired = fired;
    else if (fired.length) row.problems.push(`hooks ran: ${fired.join(', ')}`);
    c.after?.(w, row, { u, tool, meta, ap });
  } catch (e) {
    row.outcome = 'error';
    row.error = e.message.split('\n').slice(0, 6).join(' | ');
    if (run.verbose) console.error(e);
  }
  row.timings.totalMs = since(t0);
  const want = exp.want;
  if (want === 'observe') row.ok = row.problems.length === 0;
  else row.ok = row.problems.length === 0 && row.outcome === want && (row.outcome !== 'limit' || row.limits.every((l) => exp.limits.includes(l)));
  if (!run.keep) rmrf(w.dir);
  return row;
}

function pad(s, n) { s = String(s ?? ''); return s.length >= n ? s : s + ' '.repeat(n - s.length); }

function runMatrix(opt) {
  const only = typeof opt.only === 'string' ? new Set(opt.only.split(',')) : null;
  const root = tmpDir('mc-s2-matrix-');
  const u = makeCtx({ isolated: true });
  const env = environmentInfo(u);
  const run = { root, u, env, keep: !!opt.keep, verbose: !!opt.verbose };
  console.log(`S2 matrix — ${env.git}, node ${env.node}, ${env.platform}-${env.arch} (${env.os}); case-insensitive FS: ${env.caseInsensitive}; symlinks: ${env.symlinks}; exec bits: ${env.execBit}; git-lfs: ${env.lfs ?? 'no'}`);
  console.log(`scratch: ${root}${run.keep ? ' (kept)' : ''}\n`);
  const diag = diagnostics(u, root);
  const rows = [];
  const t0 = now();
  for (const c of CASES) {
    if (only && !only.has(c.id)) continue;
    const skip = c.skip?.(env);
    for (const transfer of ['bundle', 'remote']) {
      for (const landing of ['checkout', 'worktree']) {
        if (skip) {
          rows.push({ case: c.id, title: c.title, transfer, landing, expected: '-', outcome: 'skip', ok: true, notes: [skip], problems: [], fails: [], limits: [], timings: {} });
          continue;
        }
        const row = runCase(c, transfer, landing, run);
        rows.push(row);
        const mark = row.ok ? 'ok ' : 'BAD';
        const detail = [
          row.limits.length ? `limits: ${row.limits.join(',')}` : '',
          row.rawKinds?.length ? `raw: ${row.rawKinds.join(',')}` : '',
          row.undo ? `undo: ${row.undo}` : '',
          row.bundleBytes != null ? `bundle ${row.bundleBytes} B` : '',
          row.hooksFired ? `hooks: ${row.hooksFired.join(', ') || 'none'}` : '',
          ...row.notes, ...row.problems.map((p) => `PROBLEM ${p}`), ...row.fails.map((f) => `FAIL ${f}`),
          row.error ? `ERROR ${row.error}` : '',
        ].filter(Boolean).join('; ');
        console.log(`${mark} ${pad(c.id, 28)} ${pad(transfer, 7)} ${pad(landing, 9)} want ${pad(row.expected, 8)} got ${pad(row.outcome, 7)} ${pad(row.timings.totalMs + 'ms', 8)} ${detail}`);
      }
    }
  }
  const summary = {
    total: rows.length,
    ok: rows.filter((r) => r.ok).length,
    bad: rows.filter((r) => !r.ok).map((r) => `${r.case}/${r.transfer}/${r.landing}`),
    skipped: rows.filter((r) => r.outcome === 'skip').length,
    ms: since(t0),
  };
  console.log(`\n${summary.ok}/${summary.total} as designed (${summary.skipped} skipped) in ${(summary.ms / 1000).toFixed(1)} s`);
  console.log(`diagnostics: ${JSON.stringify(diag)}`);
  if (summary.bad.length) console.log(`not as designed: ${summary.bad.join(', ')}`);
  const file = path.join(HERE, `results-${process.platform}-${process.arch}${only ? '-partial' : ''}.json`);
  writeJson(file, { when: new Date().toISOString(), environment: env, diagnostics: diag, summary, rows });
  console.log(`results: ${file}`);
  if (!run.keep) rmrf(root);
  return summary.bad.length ? 1 : 0;
}

// ---------------------------------------------------------------- demo / dirty (real round trip)

function demo(ctx, dir) {
  const repo = path.join(path.resolve(dir), 'mc-demo');
  if (fs.existsSync(repo)) throw new Error(`${repo} exists`);
  fs.mkdirSync(repo, { recursive: true });
  ctx.git(repo, ['init', '-q', '-b', 'main']);
  const id = identityEnv(ctx, repo);
  writeFiles(repo, {
    'README.md': '# Mother Claude S2 demo\n\nA scratch repo for a real Mac ↔ Windows round trip.\n',
    'src/app.js': 'export const answer = 42;\n',
    'docs/notes.txt': 'one\ntwo\nthree\n',
    'win.txt': 'committed\r\nwith CRLF\r\n',
    'data/blob.bin': binBlob(42, 65_536),
    'ünïcödé/naïve café.md': 'non-ASCII path\n',
    'scripts/eol-lf/run.sh': { content: '#!/bin/sh\necho run\n', exec: true },
    'scripts/eol-lf/.gitattributes': '*.sh text eol=lf\n',
    '.gitignore': '*.log\nbuild/\n',
  });
  if (!IS_WIN) writeFiles(repo, { 'link-to-readme': { symlink: 'README.md' } });
  ctx.git(repo, ['add', '-A']);
  ctx.git(repo, ['commit', '-q', '-m', 'demo: first'], { env: id });
  put(repo, 'README.md', '# Mother Claude S2 demo\n\nSecond commit.\n');
  ctx.git(repo, ['commit', '-q', '-am', 'demo: second'], { env: id });
  const bundle = path.join(path.resolve(dir), 'mc-demo.bundle');
  ctx.git(repo, ['bundle', 'create', '-q', bundle, '--all']);
  console.log(`created ${repo}\nto get the same repo on the other machine: git clone ${bundle} mc-demo`);
}

function dirty(ctx, repo) {
  const top = repoInfo(ctx, repo).top;
  put(top, 'src/app.js', 'export const answer = 43;\n'); ctx.git(top, ['add', 'src/app.js']); // staged
  put(top, 'src/app.js', 'export const answer = 44;\n'); // and changed again
  put(top, 'docs/notes.txt', 'one\ntwo\nthree\nfour\n'); // unstaged
  put(top, 'win.txt', 'committed\r\nwith CRLF, changed\r\n'); // CRLF edit
  put(top, 'data/blob.bin', binBlob(43, 65_536)); // binary
  put(top, 'new/untracked.txt', 'untracked\n'); put(top, 'ünïcödé/neu ö.txt', 'untracked non-ASCII\n');
  put(top, 'debug.log', 'ignored, must not travel\n');
  if (fs.existsSync(at(top, 'README.md'))) ctx.git(top, ['mv', 'README.md', 'READ-ME.md']); // staged rename
  if (!IS_WIN) {
    put(top, 'scripts/new-tool.sh', '#!/bin/sh\necho tool\n', { exec: true });
    try { fs.symlinkSync('docs/notes.txt', at(top, 'link-to-notes')); } catch { /* exists */ }
  }
  console.log(`dirty state written in ${top}:`);
  console.log(ctx.git(top, ['status', '--short', '-uall']));
}

// ---------------------------------------------------------------- CLI

const BOOL = new Set(['keep', 'verbose', 'push-branch', 'keep-remote-refs', 'no-carry-lfs', 'json']);
function parseArgs(argv) {
  const pos = [];
  const opt = {};
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (a.startsWith('--')) {
      const k = a.slice(2);
      const n = argv[i + 1];
      if (!BOOL.has(k) && n !== undefined && !n.startsWith('--')) { opt[k] = n; i++; } else opt[k] = true;
    } else pos.push(a);
  }
  return { pos, opt };
}

function reportVerify(v) {
  console.log(`verify: ${v.outcome.toUpperCase()}${v.limits?.length ? ` (known limitation: ${v.limits.join(', ')})` : ''}`);
  for (const c of v.checks || []) {
    console.log(`  ${c.ok ? 'ok  ' : 'DIFF'} ${c.check}${c.want ? ` — want ${c.want}, got ${c.got}` : ''}`);
    for (const d of (c.diffs || []).slice(0, 15)) console.log(`         ${d.kind ?? ''} ${d.path}${d.want !== undefined ? ` (want ${d.want}, got ${d.got})` : ''}`);
  }
  for (const f of v.fails || []) console.log(`  FAIL ${f}`);
}

function main() {
  const { pos, opt } = parseArgs(process.argv.slice(2));
  const [cmd, a, b] = pos;
  const real = () => makeCtx({ isolated: false, hooks: false });
  switch (cmd) {
    case 'matrix':
      return runMatrix(opt);
    case 'haves': {
      if (!a) break;
      console.log(JSON.stringify(haves(real(), a), null, 2));
      return 0;
    }
    case 'snapshot': {
      if (!a || !b) break;
      const h = typeof opt.haves === 'string' ? readJson(opt.haves) : null;
      if (h?.shallow) console.error('note: the target is a shallow clone — prefer --transfer remote, or the bundle may miss objects');
      const meta = snapshot(real(), a, b, {
        haves: h?.haves || [], transfer: opt.transfer, remote: opt.remote, session: opt.session,
        pushBranch: !!opt['push-branch'], carryLfs: !opt['no-carry-lfs'],
      });
      console.log(`snapshot ${short(meta.snap)} of ${meta.branch ?? 'a detached HEAD'} @ ${short(meta.base)} → ${path.resolve(b)}`);
      console.log(`  staged tree ${short(meta.idxTree)}, working tree ${short(meta.wtTree)}, ${meta.manifest.length} files`);
      if (meta.bundle) console.log(`  bundle ${meta.bundle.size} bytes (${meta.bundle.havesUsed}/${meta.bundle.havesOffered} of the target's commits known here)`);
      if (meta.remote) console.log(`  pushed ${meta.remote.hidden.snap} and …/index to ${meta.remote.remote}`);
      console.log(`  source untouched: ${meta.untouched.ok ? 'yes' : `NO (${meta.untouched.changed.join(', ')})`}`);
      for (const w of meta.warnings) console.log(`  warning: ${w.kind}${w.detail ? ` ${JSON.stringify(w.detail)}` : ''}`);
      return meta.untouched.ok ? 0 : 1;
    }
    case 'apply': {
      if (!a || !b) break;
      const r = apply(real(), a, b, { landing: opt.landing, worktreeRoot: opt['worktree-root'], keepRemoteRefs: !!opt['keep-remote-refs'] });
      if (r.outcome === 'stop') { console.log(`stopped: ${r.reason}${r.offer ? ` — offer: ${r.offer}` : ''}`); return 3; }
      console.log(`applied (${r.landing}${r.action ? `, ${r.action}` : ''}) in ${r.path}`);
      for (const s of r.submodules || []) console.log(`  submodule ${s.path}: ${s.state}`);
      console.log(`next: node ${path.relative(process.cwd(), fileURLToPath(import.meta.url))} verify "${r.path}" "${path.resolve(b)}"`);
      return 0;
    }
    case 'verify': {
      if (!a || !b) break;
      const v = verify(real(), a, b);
      reportVerify(v);
      writeJson(path.join(b, `verify-${safeName(os.hostname())}.json`), v);
      return v.outcome === 'pass' ? 0 : v.outcome === 'limit' ? 2 : 1;
    }
    case 'undo': {
      if (!a || !b) break;
      const r = undo(real(), a, b);
      console.log(`undone (${r.landing})${r.kept.length ? `; kept, changed since the move: ${r.kept.join(', ')}` : ''}`);
      return 0;
    }
    case 'demo':
      if (!a) break;
      demo(real(), a);
      return 0;
    case 'dirty':
      if (!a) break;
      dirty(real(), a);
      return 0;
    default:
  }
  console.error(fs.readFileSync(fileURLToPath(import.meta.url), 'utf8').split('*/')[0].replace(/^#!.*\n\/\*\n?/, '').replace(/^ \* ?/gm, ''));
  return 1;
}

try {
  process.exitCode = main();
} catch (e) {
  console.error(`error: ${e.message}`);
  process.exitCode = 1;
}
