#!/usr/bin/env node
// update-manifest.mjs — merge one platform's entry into the updater manifest.
//
//   cat latest.json | node scripts/release/update-manifest.mjs \
//     --version 0.4.0 --tag v0.4.0 --now 2026-10-13T06:00:00Z \
//     --url https://github.com/…/Mother.Claude.app.tar.gz \
//     --signature "$(cat '…/Mother Claude.app.tar.gz.sig')" \
//     --archive '…/Mother Claude.app.tar.gz' \
//     --platform darwin-aarch64 \
//     > merged.json
//
// Reads the existing manifest on stdin (an empty stdin means "there isn't one
// yet") and writes the merged manifest to stdout. Touches nothing else.
//
// Why it verifies the signature before writing
// --------------------------------------------
// `--archive` is the file the signature is for, and nothing is written unless
// the signature verifies against it under the public key in
// src-tauri/tauri.conf.json — the key every installed copy checks with. A
// `.sig` left over from an earlier build next to a new archive is otherwise
// indistinguishable from a good pair, and the first anyone hears of it is an
// update that every installed copy refuses, silently. `--pubkey` overrides the
// key, for the tests. A signature that records the version it was made for
// must record this one, as updaters from tauri-plugin-updater 2.13 on insist.
//
// Why a merge rather than a write
// -------------------------------
// The macOS and Windows halves of a release are built on different machines, in
// whichever order suits, and both have to end up in ONE `latest.json` — the
// updater fetches a single file and looks up its own platform in it. A script
// that wrote the file fresh would mean whichever machine finished last silently
// deleted the other's entry, and the symptom would be an update that works on
// one operating system and reports "no update available" on the other.
//
// So this reads what is already published, replaces only the platforms it was
// given, and leaves every other key alone.
//
// Why the timestamp is passed in
// ------------------------------
// So a re-run with the same inputs produces the same bytes. A `new Date()` here
// would make every invocation a diff.

import { existsSync, readFileSync } from 'node:fs';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';

import { isRfc3339 } from './check-manifest.mjs';
import { MalformedError, sameVersion, signedVersion, verify } from './minisign.mjs';

const ROOT = join(fileURLToPath(new URL('.', import.meta.url)), '..', '..');
const args = process.argv.slice(2);

/** Reads a repeatable `--name value` flag. */
function all(name) {
  const found = [];
  for (let index = 0; index < args.length - 1; index += 1) {
    if (args[index] === `--${name}`) found.push(args[index + 1]);
  }
  return found;
}

/** Reads a `--name value` flag that may appear once. */
function one(name) {
  return all(name)[0];
}

const version = one('version');
const tag = one('tag');
const now = one('now');
const url = one('url');
// Whitespace around base64 is never meaningful, and the updater's decoder
// rejects it — `Get-Content -Raw` keeps a trailing newline that `$(cat …)` drops.
const signature = one('signature')?.trim();
const archive = one('archive');
const platforms = all('platform');

for (const [flag, value] of Object.entries({ version, tag, now, url, signature, archive })) {
  if (!value) {
    console.error(`update-manifest: --${flag} is required`);
    process.exit(2);
  }
}
if (platforms.length === 0) {
  console.error('update-manifest: at least one --platform is required');
  process.exit(2);
}

// A culture-formatted clock — '2026-10-01T09.15.42Z' from a Windows machine
// whose time separator is '.' — makes the updater reject the whole manifest,
// every platform's entry with it.
if (!isRfc3339(now)) {
  console.error(
    `update-manifest: --now ${JSON.stringify(now)} is not RFC 3339 (e.g. 2026-10-01T06:00:00Z).`,
  );
  process.exit(2);
}

if (!existsSync(archive)) {
  console.error(`update-manifest: no such archive: ${archive}`);
  process.exit(2);
}

// The check the updater makes, made here first. See the header.
const publicKey =
  one('pubkey') ??
  JSON.parse(readFileSync(join(ROOT, 'src-tauri', 'tauri.conf.json'), 'utf8')).plugins.updater
    .pubkey;
let result;
try {
  result = verify(readFileSync(archive), signature, publicKey);
} catch (error) {
  if (!(error instanceof MalformedError)) {
    throw error;
  }
  console.error(
    `update-manifest: --signature is not a signature the updater can read: ${error.message}.\n` +
      '  Pass the CONTENTS of the .sig file, not its path — base64, as Tauri writes it.',
  );
  process.exit(2);
}
if (!result.valid) {
  console.error(
    `update-manifest: the signature does not verify against ${archive}:\n` +
      result.failures.map((failure) => `  - ${failure}\n`).join('') +
      '  The .sig and the archive are not from the same build. Rebuild; nothing was written.',
  );
  process.exit(1);
}
const signed = signedVersion(result.trustedComment);
if (signed !== null && !sameVersion(signed, version)) {
  console.error(
    `update-manifest: ${archive} was signed for version ${signed}, not ${version}.\n` +
      '  Updaters from tauri-plugin-updater 2.13 on refuse that pair. It is an archive from\n' +
      '  another build: rebuild this version; nothing was written.',
  );
  process.exit(1);
}

const stdin = await new Promise((resolve) => {
  let text = '';
  process.stdin.setEncoding('utf8');
  process.stdin.on('data', (chunk) => (text += chunk));
  process.stdin.on('end', () => resolve(text.trim()));
});

let manifest = {};
if (stdin) {
  try {
    manifest = JSON.parse(stdin);
  } catch (error) {
    console.error(`update-manifest: the existing manifest is not valid JSON — ${String(error)}`);
    process.exit(1);
  }
  // Valid JSON is not enough. Invoke-RestMethod hands back raw text for a file
  // it cannot parse, ConvertTo-Json turns that text into one JSON *string*, and
  // a string has no version — so the merge used to read it as "no manifest
  // yet" and write one holding only this platform. The other machine's entries
  // vanished, it exited 0, and every name was still on the release.
  if (manifest === null || typeof manifest !== 'object' || Array.isArray(manifest)) {
    console.error(
      `update-manifest: the existing manifest is not a JSON object (got ${
        Array.isArray(manifest) ? 'an array' : typeof manifest
      }) — refusing to write over the other platforms' entries.`,
    );
    process.exit(1);
  }
}

// A manifest left over from an earlier version keeps none of its platform
// entries: they point at that version's archives, and an updater that found one
// would install the wrong thing. Only entries for THIS version survive a merge.
const existing = manifest.version === version ? (manifest.platforms ?? {}) : {};

const merged = {
  version,
  pub_date: now,
  // Shown as plain text under "update available" in Settings → Software
  // updates (updater.component.ts), so a sentence rather than a bare link.
  notes: `What's new in ${version}: https://github.com/codertapsu/mother-claude/releases/tag/${tag}`,
  platforms: { ...existing },
};

for (const platform of platforms) {
  merged.platforms[platform] = { signature, url };
}

// Sorted, so a diff between two releases shows what changed rather than how the
// keys happened to be ordered.
merged.platforms = Object.fromEntries(
  Object.entries(merged.platforms).sort(([a], [b]) => (a < b ? -1 : 1)),
);

process.stdout.write(`${JSON.stringify(merged, null, 2)}\n`);
