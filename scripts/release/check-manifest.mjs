#!/usr/bin/env node
/**
 * Checks that a release's update manifest would actually deliver the update.
 *
 *   GH_TOKEN=… node scripts/release/check-manifest.mjs \
 *     --tag v0.4.0 --repo codertapsu/mother-claude --release <release id> \
 *     --require darwin-aarch64,windows-x86_64,windows-x86_64-nsis,windows-x86_64-msi
 *
 * downloads what it needs from the release itself — latest.json, each asset it
 * names, and that asset's `.sig` — through the API, because a draft's assets
 * have no public URL. `release-upload.{sh,ps1} verify` run it this way.
 *
 * `--manifest <dir>/latest.json --assets <dir>` in place of `--release` checks
 * files already on disk instead, named as they are on the release.
 *
 * The public key is read from src-tauri/tauri.conf.json — the one compiled into
 * every installed copy — unless `--pubkey` names another.
 *
 * # Why names are not enough
 *
 * `release-upload.sh verify` used to count asset names, and a release can have
 * every name right and still deliver nothing. The case that prompted this:
 * rebuilding a release replaces the update archive, and the manifest step that
 * follows uploads the archive, then its `.sig`, then `latest.json`. Interrupt it
 * after the first upload and the draft holds the NEW archive beside the OLD
 * signature. Every name is present; `verify` said "complete"; and once
 * published, every installed copy downloads the archive, checks it against a
 * signature for a file that no longer exists, and refuses the update without a
 * word to anyone.
 *
 * So this checks what the updater checks, the way the updater checks it:
 *
 *   - `version` is the tag without its "v";
 *   - `pub_date` is RFC 3339, which the updater requires of the whole file;
 *   - every required platform is present;
 *   - every platform's `url` names an asset of this tag in this repository,
 *     and that asset is here;
 *   - every platform's `signature` verifies against that asset under the
 *     public key — see minisign.mjs;
 *   - a signature that records the version it was made for (a `version:` in
 *     its trusted comment) records this one — updaters from
 *     tauri-plugin-updater 2.13 on refuse anything else;
 *   - wherever a `<asset>.sig` was uploaded, it is that same signature, so the
 *     file a person downloads to check by hand agrees with the manifest.
 *
 * Exits 0 only when every check passes. Prints one line per platform either
 * way, so a half-finished release shows which half is missing.
 */

import { Buffer } from 'node:buffer';
import { readFileSync, existsSync } from 'node:fs';
import { join } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

import { MalformedError, sameVersion, signedVersion, verify } from './minisign.mjs';

const ROOT = join(fileURLToPath(new URL('.', import.meta.url)), '..', '..');

const RFC3339 =
  /^(\d{4})-(\d{2})-(\d{2})[\x00-\x7f](\d{2}):(\d{2}):(\d{2})(?:\.\d+)?(?:[Zz]|[+-](\d{2}):(\d{2}))$/;

/**
 * Whether the updater can read `text` as a `pub_date`: RFC 3339 as the `time`
 * crate parses it (0.3.51, `parsing/parsable.rs`) — a four-digit year, any one
 * ASCII character between date and time, an optional fraction, "Z", "z" or an
 * offset — and then only a date and time that exist. A culture-formatted
 * '2026-10-01T09.15.42Z' fails here as it fails there, and so does
 * '2026-02-30T09:15:42Z'. Leap seconds, which the crate accepts in narrow
 * cases, are refused; nothing here writes one.
 */
export function isRfc3339(text) {
  const match = RFC3339.exec(text);
  if (match === null) {
    return false;
  }
  const [year, month, day, hour, minute, second, offsetHour = 0, offsetMinute = 0] = match
    .slice(1)
    .map((part) => (part === undefined ? undefined : Number(part)));
  const leap = (year % 4 === 0 && year % 100 !== 0) || year % 400 === 0;
  const days = [31, leap ? 29 : 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31][month - 1];
  return (
    days !== undefined &&
    day >= 1 &&
    day <= days &&
    hour <= 23 &&
    minute <= 59 &&
    second <= 59 &&
    offsetHour <= 23 &&
    offsetMinute <= 59
  );
}

/** The asset name a release download URL points at, or null. */
function assetName(url, repo, tag) {
  const prefix = `https://github.com/${repo}/releases/download/${tag}/`;
  if (typeof url !== 'string' || !url.startsWith(prefix)) {
    return null;
  }
  let name;
  try {
    name = decodeURIComponent(url.slice(prefix.length));
  } catch {
    return null;
  }
  return name.includes('/') || name === '' ? null : name;
}

/**
 * The checks, as data. `read(name)` returns an asset's bytes or null, which is
 * what lets the tests run without a directory.
 */
export function checkManifest({ manifest, tag, repo, publicKey, require = [], read }) {
  const problems = [];
  const rows = [];

  const expectedVersion = tag.replace(/^v/, '');
  if (manifest.version !== expectedVersion) {
    problems.push(
      `version is ${JSON.stringify(manifest.version)}, but the tag says ${expectedVersion}`,
    );
  }
  // `Option<String>` to the updater: absent or null is fine, anything else must parse.
  if (manifest.pub_date != null && !isRfc3339(String(manifest.pub_date))) {
    problems.push(
      `pub_date ${JSON.stringify(manifest.pub_date)} is not RFC 3339 — the updater rejects the whole manifest`,
    );
  }

  const platforms = manifest.platforms ?? {};
  for (const platform of require) {
    if (!(platform in platforms)) {
      problems.push(
        `${platform} is not in the manifest — that leg has not run, or its entry was lost`,
      );
    }
  }

  for (const [platform, entry] of Object.entries(platforms)) {
    const name = assetName(entry?.url, repo, tag);
    const row = { platform, asset: name, valid: false, detail: '' };
    rows.push(row);

    if (name === null) {
      row.detail = `url ${JSON.stringify(entry?.url)} is not an asset of ${repo} ${tag}`;
      problems.push(`${platform}: ${row.detail}`);
      continue;
    }
    const archive = read(name);
    if (archive === null) {
      row.detail = `${name} is not on the release`;
      problems.push(`${platform}: ${row.detail}`);
      continue;
    }
    if (typeof entry.signature !== 'string' || entry.signature.length === 0) {
      row.detail = 'no signature';
      problems.push(`${platform}: no signature in the manifest`);
      continue;
    }

    let result;
    try {
      result = verify(archive, entry.signature, publicKey);
    } catch (error) {
      if (!(error instanceof MalformedError)) {
        throw error;
      }
      row.detail = `unreadable signature: ${error.message}`;
      problems.push(`${platform}: ${row.detail}`);
      continue;
    }
    if (!result.valid) {
      row.detail = result.failures.join('; ');
      problems.push(`${platform}: ${name} does not verify — ${row.detail}`);
      continue;
    }

    // Read only now that the signature holds: before that, the trusted comment
    // is whatever anybody wrote. A signature with no version passes, as it
    // does in the updater.
    const signed = signedVersion(result.trustedComment);
    if (signed !== null && !sameVersion(signed, manifest.version)) {
      row.detail = `signed for version ${signed}, but the manifest announces ${String(manifest.version)}`;
      problems.push(
        `${platform}: ${name} was ${row.detail} — updaters from tauri-plugin-updater 2.13 on refuse it`,
      );
      continue;
    }

    // Only the file is trimmed. The manifest's string is what the updater
    // decodes, exactly as it stands, and it has already verified above.
    const sigFile = read(`${name}.sig`);
    if (sigFile !== null && sigFile.toString('utf8').trim() !== entry.signature) {
      row.detail = `${name}.sig differs from the manifest's signature`;
      problems.push(`${platform}: ${row.detail} — the two were not written by the same build`);
      continue;
    }

    row.valid = true;
    row.detail = `verifies under key ${result.keyId}`;
  }

  return { ok: problems.length === 0, rows, problems };
}

const API = 'https://api.github.com';

/**
 * Downloads what `checkManifest` needs from a release: its latest.json, every
 * asset that names, and each such asset's `.sig` where one was uploaded —
 * nothing else, so the installers people download by hand are not fetched.
 *
 * Through the API rather than the public download URL, because a draft's
 * assets have none. `fetchImpl` is for the tests.
 *
 * Returns `{ manifest, assets }`; `manifest` is null when the release has no
 * latest.json yet.
 */
export async function fetchRelease({ repo, tag, releaseId, token, fetchImpl = fetch }) {
  const get = async (path, accept) => {
    const response = await fetchImpl(`${API}${path}`, {
      headers: {
        Accept: accept,
        Authorization: `Bearer ${token}`,
        'User-Agent': 'mother-claude-release-check',
        'X-GitHub-Api-Version': '2022-11-28',
      },
    });
    if (!response.ok) {
      throw new Error(
        `GET ${path} answered ${String(response.status)} — check the token can read ${repo}`,
      );
    }
    return response;
  };

  const listing = await (
    await get(
      `/repos/${repo}/releases/${String(releaseId)}/assets?per_page=100`,
      'application/vnd.github+json',
    )
  ).json();
  const ids = new Map(listing.map((asset) => [asset.name, asset.id]));
  const download = async (name) =>
    Buffer.from(
      await (
        await get(
          `/repos/${repo}/releases/assets/${String(ids.get(name))}`,
          'application/octet-stream',
        )
      ).arrayBuffer(),
    );

  const assets = new Map();
  if (!ids.has('latest.json')) {
    return { manifest: null, assets };
  }
  const manifest = parseManifest((await download('latest.json')).toString('utf8'));
  if (typeof manifest === 'string') {
    return { manifest: null, unreadable: manifest, assets };
  }

  const wanted = new Set();
  for (const entry of Object.values(manifest.platforms ?? {})) {
    const name = assetName(entry?.url, repo, tag);
    if (name !== null) {
      wanted.add(name);
      wanted.add(`${name}.sig`);
    }
  }
  for (const name of wanted) {
    if (ids.has(name)) {
      assets.set(name, await download(name));
    }
  }
  return { manifest, assets };
}

/**
 * The manifest as the updater would read it, or — when it cannot be read at
 * all — a string saying why. A byte-order mark counts: serde_json does not
 * skip one, and nothing here writes one.
 */
function parseManifest(text) {
  try {
    return JSON.parse(text);
  } catch (error) {
    return `latest.json is not valid JSON (${error.message}) — the updater cannot read it, on any platform`;
  }
}

/** `--name value` pairs; repeated names are not needed here. */
function parseArgs(argv) {
  const args = {};
  for (let i = 0; i < argv.length; i += 2) {
    const name = argv[i];
    if (!name.startsWith('--') || argv[i + 1] === undefined) {
      throw new Error(`expected --name value pairs, got ${JSON.stringify(name)}`);
    }
    args[name.slice(2)] = argv[i + 1];
  }
  return args;
}

async function main(argv) {
  const args = parseArgs(argv);
  const remote = args.release !== undefined;
  for (const required of ['tag', 'repo', ...(remote ? [] : ['manifest', 'assets'])]) {
    if (!args[required]) {
      throw new Error(`--${required} is required${remote ? '' : ' (or --release <id>)'}`);
    }
  }

  const publicKey =
    args.pubkey ??
    JSON.parse(readFileSync(join(ROOT, 'src-tauri', 'tauri.conf.json'), 'utf8')).plugins.updater
      .pubkey;

  let manifest;
  let read;
  if (remote) {
    const token = process.env.GH_TOKEN ?? process.env.GITHUB_TOKEN;
    if (!token) {
      throw new Error('--release needs a token in GH_TOKEN');
    }
    const fetched = await fetchRelease({
      repo: args.repo,
      tag: args.tag,
      releaseId: args.release,
      token,
    });
    if (fetched.unreadable !== undefined) {
      console.error(`  problem: ${fetched.unreadable}`);
      return 1;
    }
    if (fetched.manifest === null) {
      console.error(
        `latest.json for ${args.tag}: not on the release yet — neither leg has published its manifest.`,
      );
      return 1;
    }
    manifest = fetched.manifest;
    read = (name) => fetched.assets.get(name) ?? null;
  } else {
    manifest = parseManifest(readFileSync(args.manifest, 'utf8'));
    if (typeof manifest === 'string') {
      console.error(`  problem: ${manifest}`);
      return 1;
    }
    read = (name) => {
      const path = join(args.assets, name);
      return existsSync(path) ? readFileSync(path) : null;
    };
  }

  const result = checkManifest({
    manifest,
    tag: args.tag,
    repo: args.repo,
    publicKey,
    require: (args.require ?? '').split(',').filter(Boolean),
    read,
  });

  console.log(
    `latest.json for ${args.tag}: version ${String(manifest.version)}, pub_date ${String(manifest.pub_date)}`,
  );
  for (const row of result.rows) {
    console.log(`  ${row.valid ? 'ok     ' : 'INVALID'} ${row.platform.padEnd(20)} ${row.detail}`);
  }
  for (const problem of result.problems) {
    console.error(`  problem: ${problem}`);
  }
  console.log(
    result.ok ? '  every platform verifies.' : '  NOT publishable: see the problems above.',
  );
  return result.ok ? 0 : 1;
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  try {
    process.exitCode = await main(process.argv.slice(2));
  } catch (error) {
    console.error(`check-manifest: ${error.message}`);
    process.exitCode = 2;
  }
}
