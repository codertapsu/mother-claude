#!/usr/bin/env node
// prune-releases.mjs — delete the published releases older than the newest few.
//
// Only the newest release is ever downloaded by the updater: every installed
// copy, back to v0.1.0, polls releases/latest/download/latest.json
// (docs/AUTOUPDATE.md). An older release holds a few hundred megabytes of
// installers nobody is sent to.
//
//   node scripts/release/prune-releases.mjs --dry-run       # what would go; deletes nothing
//   node scripts/release/prune-releases.mjs                 # asks, then deletes
//   node scripts/release/prune-releases.mjs --keep 3 --yes  # keeps three, asks nothing
//   node scripts/release/prune-releases.mjs --cleanup-tag   # their git tags go too
//
// It keeps the newest --keep published versions (default 2), and never deletes,
// whatever --keep says:
//
//   - the release GitHub marks Latest: it is the update feed;
//   - a draft: it is a release still being built — both legs upload into it —
//     so it does not count towards --keep either;
//   - a release whose tag is not a version (v0.4.0, 1.2.0-beta.1): not one of
//     ours to judge.
//
// "Newest" is by version, not by date, so a fix to an older line published
// last does not push the current version out — and v0.10.0 is newer than
// v0.9.0, which a sort by text gets wrong.
//
// The tags stay unless --cleanup-tag: `git checkout v0.3.0` needs them, as does
// every link to a file at that version, and they cost nothing.
//
// Repo: GH_REPO if exported, else codertapsu/mother-claude — never derived from
// `origin`, as in release-upload.sh. Needs the `gh` CLI, signed in with write
// access to the repo's releases.
//
// Pure Node and `gh`, no dependencies.

import { execFileSync } from 'node:child_process';
import { createInterface } from 'node:readline/promises';
import { pathToFileURL } from 'node:url';

/** Where published installers live (RELEASE_REPO in release-upload.sh). */
const RELEASE_REPO = 'codertapsu/mother-claude';

const DEFAULT_KEEP = 2;

const USAGE =
  'usage: node scripts/release/prune-releases.mjs [--keep N] [--dry-run] [--yes] [--cleanup-tag]';

/** `v0.4.0`, `0.4.0` or `v1.2.0-beta.1` (build metadata ignored); null for any other tag. */
export function parseVersion(tag) {
  const m = /^v?(\d+)\.(\d+)\.(\d+)(?:-([0-9A-Za-z.-]+))?(?:\+[0-9A-Za-z.-]+)?$/.exec(tag ?? '');
  return m ? { core: [Number(m[1]), Number(m[2]), Number(m[3])], pre: m[4] ?? null } : null;
}

const byNumbers = new Intl.Collator('en', { numeric: true });

/** Negative when `a` is the older version; a prerelease is older than its release. */
export function compareVersions(a, b) {
  for (let i = 0; i < 3; i += 1) {
    if (a.core[i] !== b.core[i]) {
      return a.core[i] - b.core[i];
    }
  }
  if (a.pre === b.pre) {
    return 0;
  }
  if (a.pre === null || b.pre === null) {
    return a.pre === null ? 1 : -1;
  }
  return byNumbers.compare(a.pre, b.pre);
}

/**
 * Sort releases, as `gh release list --json` gives them, into what a prune
 * keeps, deletes and leaves alone (`keep` and `remove` newest first).
 *
 * Throws rather than guess when no release is marked Latest: an older `gh`
 * leaves `isLatest` out, and Latest is the one release that must never go.
 */
export function planPrune(releases, keep = DEFAULT_KEEP) {
  if (!Number.isInteger(keep) || keep < 1) {
    throw new Error(`--keep must be a whole number, 1 or more (got ${keep})`);
  }
  const published = releases.filter((r) => !r.isDraft);
  if (published.some((r) => !r.isPrerelease) && !published.some((r) => r.isLatest === true)) {
    throw new Error(
      'no release is marked Latest, so which one feeds the updater is unknown (is gh up to date?)',
    );
  }
  const leave = [];
  const versions = [];
  for (const release of releases) {
    const version = release.isDraft ? null : parseVersion(release.tagName);
    if (version === null) {
      leave.push(release);
    } else {
      versions.push({ release, version });
    }
  }
  versions.sort((a, b) => compareVersions(b.version, a.version));
  const plan = { keep: [], remove: [], leave };
  versions.forEach(({ release }, i) => {
    (i < keep || release.isLatest === true ? plan.keep : plan.remove).push(release);
  });
  return plan;
}

export function parseArgs(argv) {
  const opts = { keep: DEFAULT_KEEP, dryRun: false, yes: false, cleanupTag: false, help: false };
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    if (arg === '--keep' || arg.startsWith('--keep=')) {
      let value = arg.slice('--keep='.length);
      if (arg === '--keep') {
        i += 1;
        value = argv[i] ?? '';
      }
      if (!/^\d+$/.test(value) || Number(value) < 1) {
        throw new Error(`--keep needs a whole number, 1 or more (got ${JSON.stringify(value)})`);
      }
      opts.keep = Number(value);
    } else if (arg === '--dry-run') {
      opts.dryRun = true;
    } else if (arg === '--yes' || arg === '-y') {
      opts.yes = true;
    } else if (arg === '--cleanup-tag') {
      opts.cleanupTag = true;
    } else if (arg === '--help' || arg === '-h') {
      opts.help = true;
    } else {
      throw new Error(`unknown argument ${JSON.stringify(arg)}\n${USAGE}`);
    }
  }
  return opts;
}

/** `gh`, its errors on the terminal; throws when it fails. */
function gh(args) {
  return execFileSync('gh', args, { encoding: 'utf8', stdio: ['ignore', 'pipe', 'inherit'] });
}

function listReleases(repo, run) {
  const fields = 'tagName,isDraft,isPrerelease,isLatest,publishedAt';
  return JSON.parse(run(['release', 'list', '--repo', repo, '--limit', '1000', '--json', fields]));
}

/** The answer typed on the terminal, or null when nobody is there to type one. */
async function askOnTerminal(question) {
  if (!process.stdin.isTTY) {
    return null;
  }
  const rl = createInterface({ input: process.stdin, output: process.stdout });
  try {
    return await rl.question(question);
  } finally {
    rl.close();
  }
}

function row(action, release, note = '') {
  const date = release.isDraft ? '' : String(release.publishedAt ?? '').slice(0, 10);
  return `  ${action.padEnd(7)} ${String(release.tagName).padEnd(16)} ${date.padEnd(10)}  ${note}`.trimEnd();
}

/** The command; `io` stands in for `gh`, the terminal and the environment in tests. */
export async function main(
  argv,
  {
    run = gh,
    ask = askOnTerminal,
    log = console.log,
    error = console.error,
    env = process.env,
  } = {},
) {
  const opts = parseArgs(argv);
  if (opts.help) {
    log(USAGE);
    return 0;
  }
  const repo = env.GH_REPO || RELEASE_REPO;
  if (!/^[\w.-]+\/[\w.-]+$/.test(repo)) {
    throw new Error(`GH_REPO must be owner/name (got ${JSON.stringify(repo)})`);
  }

  const releases = listReleases(repo, run);
  const plan = planPrune(releases, opts.keep);
  log(`${repo}: ${releases.length} release(s), keeping the newest ${opts.keep}`);
  for (const release of plan.keep) {
    log(row('keep', release, release.isLatest ? 'latest' : ''));
  }
  for (const release of plan.remove) {
    log(row('delete', release));
  }
  for (const release of plan.leave) {
    log(row('leave', release, release.isDraft ? 'draft' : 'not a version tag'));
  }

  if (plan.remove.length === 0) {
    log('Nothing to delete.');
    return 0;
  }
  const what = `${plan.remove.length} release(s)${opts.cleanupTag ? ' and their tags' : ''}`;
  if (opts.dryRun) {
    log(`Dry run: would delete ${what}. Nothing deleted.`);
    return 0;
  }
  if (!opts.yes) {
    const answer = await ask(`Delete ${what} from ${repo}? [y/N] `);
    if (answer === null) {
      throw new Error('nobody to ask: pass --yes to delete without a question');
    }
    if (!/^y(es)?$/i.test(answer.trim())) {
      log('Nothing deleted.');
      return 1;
    }
  }

  for (const release of plan.remove) {
    const tag = opts.cleanupTag ? ['--cleanup-tag'] : [];
    run(['release', 'delete', release.tagName, '--repo', repo, '--yes', ...tag]);
    log(`  deleted ${release.tagName}${opts.cleanupTag ? ' and its tag' : ''}`);
  }

  // Check, do not assume: list again, as `verify` does after an upload.
  const left = new Set(listReleases(repo, run).map((r) => r.tagName));
  const still = plan.remove.filter((r) => left.has(r.tagName)).map((r) => r.tagName);
  if (still.length > 0) {
    error(`  problem: still listed after deleting: ${still.join(', ')}`);
    return 1;
  }
  log(`Done: ${plan.remove.length} deleted, ${plan.keep.length} kept.`);
  return 0;
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  try {
    process.exitCode = await main(process.argv.slice(2));
  } catch (error) {
    console.error(`prune-releases: ${error.message}`);
    process.exitCode = 2;
  }
}
