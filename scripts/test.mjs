#!/usr/bin/env node
// test.mjs — run the Angular unit tests: `ng test` (Vitest) in a real browser.
//
//   node scripts/test.mjs --watch=false     # = npm test
//   node scripts/test.mjs                   # watch mode
//
// The specs run in Chromium through Playwright (angular.json `browsers`), not
// in a simulated DOM: the app lives in real webviews, and the specs read
// computed styles, observers and colours as a browser reports them. Rather
// than download Playwright's own Chromium, they use the Google Chrome
// installed on this machine, as Karma did: Angular's Vitest runner launches
// CHROME_BIN when it is set, and this script sets it to that Chrome. Set
// CHROME_BIN yourself to use another Chromium; on a machine without Chrome,
// `npx playwright install chromium` gives Playwright its own.
//
// Pure Node, no dependencies; works the same on macOS and Windows.

import { spawnSync } from 'node:child_process';
import { existsSync } from 'node:fs';
import { homedir } from 'node:os';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');

/** Where Google Chrome is installed, the usual places first. */
function installedChrome() {
  const env = process.env;
  const candidates =
    process.platform === 'darwin'
      ? [
          '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome',
          join(homedir(), 'Applications/Google Chrome.app/Contents/MacOS/Google Chrome'),
        ]
      : process.platform === 'win32'
        ? [env['PROGRAMFILES'], env['PROGRAMFILES(X86)'], env['LOCALAPPDATA']]
            .filter(Boolean)
            .map((dir) => join(dir, 'Google', 'Chrome', 'Application', 'chrome.exe'))
        : [
            '/usr/bin/google-chrome',
            '/usr/bin/google-chrome-stable',
            '/usr/bin/chromium',
            '/usr/bin/chromium-browser',
          ];
  return candidates.find((p) => existsSync(p));
}

const env = { ...process.env };
if (!env['CHROME_BIN']) {
  const chrome = installedChrome();
  if (chrome) env['CHROME_BIN'] = chrome;
}

// The CLI's own entry point, run with this Node: no `ng.cmd`, which Node
// refuses to spawn without a shell on Windows.
const ng = join(root, 'node_modules', '@angular', 'cli', 'bin', 'ng.js');
const result = spawnSync(process.execPath, [ng, 'test', ...process.argv.slice(2)], {
  cwd: root,
  env,
  stdio: 'inherit',
});
if (result.error) {
  console.error(`test: could not run ng: ${result.error.message}`);
  process.exit(1);
}
process.exit(result.status ?? 1);
