#!/usr/bin/env node
// set-version.mjs — the single authority on where Mother Claude's version lives.
//
// Six spots in five files must agree, and nothing in the build makes them:
//
//   package.json                "version"
//   package-lock.json           "version", and packages[""].version
//   src-tauri/Cargo.toml        [package] version
//   src-tauri/Cargo.lock        the version of [[package]] "mother-claude"
//   src-tauri/tauri.conf.json   "version"   (what the installer reports)
//
// The two lockfiles are here because the first `npm install` or `cargo build`
// after a bump rewrites them otherwise — which leaves the release checkout
// dirty halfway through the release, and the next run of either release
// script refuses to start.
//
// Two modes:
//
//   node scripts/release/set-version.mjs 0.4.0     # write all of them (a leading "v" is fine)
//   node scripts/release/set-version.mjs --check   # verify they agree; non-zero if not
//   node scripts/release/set-version.mjs --check --expect v0.4.0
//
// `--check` exists to run BEFORE a release build, not after. Signing and
// notarizing a macOS bundle takes minutes and cannot be undone once uploaded;
// discovering there that tauri.conf.json still said 0.3.0 means the installer
// reports the wrong version to every user who already downloaded it. The check
// costs milliseconds and is wired into both release scripts.
//
// Pure Node, no dependencies.

import { readFileSync, writeFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';

const root = join(dirname(fileURLToPath(import.meta.url)), '..', '..');

/** The first top-level, two-space-indented "version" key of a JSON file. */
const JSON_VERSION = /^( {2}"version":\s*")([^"]*)(")/m;

/**
 * package-lock.json's own entry under "packages": the `"": {` object, and the
 * first "version" inside it (six spaces in, as npm writes it).
 */
const LOCK_ROOT_VERSION = /(\n {4}"": \{\n(?: {6}.*\n)*? {6}"version":\s*")([^"]*)(")/;

/** Every place the version is written, and how to find it in that file. */
const SPOTS = [
  { label: 'package.json', file: 'package.json', find: jsonVersion(JSON_VERSION) },
  { label: 'package-lock.json', file: 'package-lock.json', find: jsonVersion(JSON_VERSION) },
  {
    label: 'package-lock.json ""',
    file: 'package-lock.json',
    find: jsonVersion(LOCK_ROOT_VERSION),
  },
  { label: 'src-tauri/Cargo.toml', file: 'src-tauri/Cargo.toml', find: cargoToml },
  { label: 'src-tauri/Cargo.lock', file: 'src-tauri/Cargo.lock', find: cargoLock },
  {
    label: 'src-tauri/tauri.conf.json',
    file: 'src-tauri/tauri.conf.json',
    find: jsonVersion(JSON_VERSION),
  },
];

/**
 * A finder returns `{ version, replace(text, version) }` for a file's text, or
 * throws when the spot is not there — a spot that quietly went missing would
 * otherwise "agree" by not being checked at all.
 */
function jsonVersion(pattern) {
  return (text, file) => {
    const match = pattern.exec(text);
    if (!match) throw new Error(`no version where it belongs in ${file}`);
    return {
      version: match[2],
      replace: (current, version) => current.replace(pattern, `$1${version}$3`),
    };
  };
}

/** The version inside [package], not a dependency's. */
function cargoToml(text, file) {
  return cargoLine(text, file, (lines) => {
    const table = lines.indexOf('[package]');
    if (table === -1) throw new Error(`no [package] table in ${file}`);
    return table;
  });
}

/** The version of the [[package]] whose name is "mother-claude". */
function cargoLock(text, file) {
  return cargoLine(text, file, (lines) => {
    const name = lines.indexOf('name = "mother-claude"');
    if (name === -1) throw new Error(`no [[package]] "mother-claude" in ${file}`);
    return name;
  });
}

/** The first `version = "…"` line after `start(lines)`, inside the same table. */
function cargoLine(text, file, start) {
  const lines = text.split('\n');
  const from = start(lines);
  for (let i = from + 1; i < lines.length; i++) {
    if (lines[i].startsWith('[') || lines[i] === '') break; // left the table
    const match = /^version\s*=\s*"([^"]*)"/.exec(lines[i]);
    if (match) {
      return {
        version: match[1],
        replace: (current, version) => {
          const next = current.split('\n');
          next[i] = `version = "${version}"`;
          return next.join('\n');
        },
      };
    }
  }
  throw new Error(`no version line where it belongs in ${file}`);
}

/**
 * Reads a spot from its file. A checkout made with Windows line endings (a
 * clone from before .gitattributes, or core.autocrlf on a file it does not
 * cover) is read as LF and written back as CRLF, so the finders only ever see
 * "\n" and the file keeps the endings it had.
 */
function readSpot(spot) {
  const raw = readFileSync(join(root, spot.file), 'utf8');
  const crlf = raw.includes('\r\n');
  const text = crlf ? raw.replace(/\r\n/g, '\n') : raw;
  return { text, crlf, ...spot.find(text, spot.file) };
}

function writeSpot(spot, version) {
  const current = readSpot(spot);
  const next = current.replace(current.text, version);
  writeFileSync(join(root, spot.file), current.crlf ? next.replace(/\n/g, '\r\n') : next);
  console.log(`  ${spot.label.padEnd(27)} ${current.version} -> ${version}`);
}

function normalize(raw) {
  const version = raw.replace(/^v/, '');
  if (!/^\d+\.\d+\.\d+(?:[-+].+)?$/.test(version)) {
    console.error(`error: "${raw}" is not a valid x.y.z version`);
    process.exit(1);
  }
  return version;
}

const args = process.argv.slice(2);
const check = args.includes('--check');
const expectFlag = args.indexOf('--expect');
const expected = expectFlag !== -1 ? normalize(args[expectFlag + 1] ?? '') : null;
// `indexOf` is -1 when `--expect` is absent, and -1 + 1 is 0: without this the
// first positional argument would be taken for --expect's value and dropped.
const expectValue = expectFlag === -1 ? -1 : expectFlag + 1;
const positional = args.filter((a, i) => !a.startsWith('--') && i !== expectValue);

if (check) {
  const found = SPOTS.map((spot) => ({ spot, ...readSpot(spot) }));
  for (const { spot, version } of found) {
    console.log(`  ${spot.label.padEnd(27)} ${version}`);
  }

  const distinct = [...new Set(found.map((f) => f.version))];
  if (distinct.length > 1) {
    console.error(
      `\nerror: the version spots disagree (${distinct.join(', ')}).\n` +
        `Fix with: node scripts/release/set-version.mjs ${distinct[0]}`,
    );
    process.exit(1);
  }

  if (expected && distinct[0] !== expected) {
    console.error(
      `\nerror: the repository is at ${distinct[0]} but the release tag says ${expected}.\n` +
        `Bump first: node scripts/release/set-version.mjs ${expected}`,
    );
    process.exit(1);
  }

  console.log(`\nAll ${SPOTS.length} version spots agree: ${distinct[0]}`);
  process.exit(0);
}

if (positional.length !== 1) {
  console.error('usage: node scripts/release/set-version.mjs <version>   e.g. 0.4.0');
  console.error('       node scripts/release/set-version.mjs --check [--expect v0.4.0]');
  process.exit(1);
}

const version = normalize(positional[0]);
console.log(`Setting version ${version}:`);
for (const spot of SPOTS) writeSpot(spot, version);

console.log(
  '\nNext: review `git diff`, then commit and tag:\n' +
    `  git commit -am "chore(release): v${version}"\n` +
    `  git tag v${version}\n` +
    '\nThen build on each machine — see docs/RELEASING.md. Installers are NOT built in CI.',
);
