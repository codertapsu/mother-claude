#!/usr/bin/env node
// sidecar-npm.mjs — run npm for sidecar/ from inside an npm script.
//
//   node scripts/sidecar-npm.mjs install        # = npm --prefix sidecar install
//   node scripts/sidecar-npm.mjs run build
//   node scripts/sidecar-npm.mjs prune --omit=dev
//
// Why not just `npm --prefix sidecar install`: npm hands its own configuration
// to the scripts it runs as `npm_config_*` variables, and npm 12 refuses an
// `allow-scripts` that arrives that way — "--allow-scripts is not allowed in
// project-scoped installs" — even when it came from the user's own ~/.npmrc.
// So on any machine whose ~/.npmrc sets allow-scripts, `npm run sidecar:bundle`
// (and with it `tauri build`, whose beforeBuildCommand runs it) failed before
// building anything. The nested npm reads ~/.npmrc itself, so dropping the
// variable loses nothing.
//
// Pure Node, no dependencies; works the same on macOS and Windows.

import { spawnSync } from 'node:child_process';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');

const env = Object.fromEntries(
  Object.entries(process.env).filter(([key]) => key.toLowerCase() !== 'npm_config_allow_scripts'),
);

// The npm that is running this script, when there is one: npm sets
// npm_execpath to its own CLI entry point. Running that file with this Node
// avoids spawning `npm.cmd`, which Node refuses without a shell on Windows.
// Outside npm, fall back to whatever `npm` is on PATH.
const cli = process.env.npm_execpath;
const viaNode = Boolean(cli) && /\.c?js$/.test(cli);
const command = viaNode ? process.execPath : 'npm';
const args = [...(viaNode ? [cli] : []), '--prefix', 'sidecar', ...process.argv.slice(2)];

const result = spawnSync(command, args, {
  cwd: root,
  env,
  stdio: 'inherit',
  shell: !viaNode && process.platform === 'win32',
});
if (result.error) {
  console.error(`sidecar-npm: could not run npm: ${result.error.message}`);
  process.exit(1);
}
process.exit(result.status ?? 1);
