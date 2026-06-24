#!/usr/bin/env node
// set-version.mjs — sync the app version across the three files that must agree:
//   package.json  ·  src-tauri/tauri.conf.json  ·  src-tauri/Cargo.toml
//
// Usage:  node scripts/release/set-version.mjs 0.2.0   (a leading "v" is ok)
//
// Run this, commit the result, then tag `vX.Y.Z` to cut a release
// (see docs/RELEASING.md). Pure Node, no dependencies.

import { readFileSync, writeFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';

const root = join(dirname(fileURLToPath(import.meta.url)), '..', '..');

const raw = process.argv[2];
if (!raw) {
  console.error('usage: node scripts/release/set-version.mjs <version>   e.g. 0.2.0');
  process.exit(1);
}
const version = raw.replace(/^v/, '');
if (!/^\d+\.\d+\.\d+(?:[-+].+)?$/.test(version)) {
  console.error(`error: "${raw}" is not a valid x.y.z version`);
  process.exit(1);
}

function patchJson(rel, label) {
  const path = join(root, rel);
  const text = readFileSync(path, 'utf8');
  // Replace only the FIRST top-level "version" key (indented two spaces).
  const next = text.replace(/^( {2}"version":\s*")[^"]*(")/m, `$1${version}$2`);
  if (next === text) throw new Error(`could not find a version field in ${rel}`);
  writeFileSync(path, next);
  console.log(`  ${label.padEnd(18)} -> ${version}  (${rel})`);
}

function patchCargo(rel) {
  const path = join(root, rel);
  const lines = readFileSync(path, 'utf8').split('\n');
  const pkg = lines.indexOf('[package]');
  if (pkg === -1) throw new Error(`no [package] table in ${rel}`);
  // The package version is the first `version = "..."` at line start after [package].
  for (let i = pkg + 1; i < lines.length; i++) {
    if (lines[i].startsWith('[') && i !== pkg) break; // left the [package] table
    if (/^version\s*=\s*"/.test(lines[i])) {
      lines[i] = `version = "${version}"`;
      writeFileSync(path, lines.join('\n'));
      console.log(`  ${'Cargo.toml'.padEnd(18)} -> ${version}  (${rel})`);
      return;
    }
  }
  throw new Error(`no package version line in ${rel}`);
}

console.log(`Setting version ${version}:`);
patchJson('package.json', 'package.json');
patchJson('src-tauri/tauri.conf.json', 'tauri.conf.json');
patchCargo('src-tauri/Cargo.toml');
console.log('\nNext: review `git diff`, run `cargo update -p mother-claude` if you keep Cargo.lock,');
console.log('then commit and tag (e.g. `git tag v' + version + ' && git push --tags`).');
