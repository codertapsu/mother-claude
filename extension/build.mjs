#!/usr/bin/env node
// Builds the Mother Claude browser extension.
//
//   node extension/build.mjs                    Chrome/Edge/Brave → dist-extension/chrome/ + zip
//   node extension/build.mjs --target firefox   Firefox            → dist-extension/firefox/ + zip
//   node extension/build.mjs --target chrome-e2e  the end-to-end test build (see test/e2e)
//   node extension/build.mjs --watch            rebuild on change (Chrome, no zip)
//   --sourcemap                                 inline source maps
//
// Everything is bundled (no remote code); esbuild comes with the Angular
// toolchain already in node_modules.

import { createHash } from 'node:crypto';
import {
  cpSync,
  existsSync,
  mkdirSync,
  readFileSync,
  readdirSync,
  rmSync,
  statSync,
  writeFileSync,
} from 'node:fs';
import { dirname, join, relative } from 'node:path';
import { fileURLToPath } from 'node:url';
import * as esbuild from 'esbuild';
import { decodePng, encodePng, resize } from './scripts/png.mjs';
import { zip } from './scripts/zip.mjs';

const here = dirname(fileURLToPath(import.meta.url));
const root = join(here, '..');
const args = process.argv.slice(2);
const flag = (name) => args.includes(name);
const option = (name, fallback) => {
  const i = args.indexOf(name);
  return i >= 0 && args[i + 1] ? args[i + 1] : fallback;
};

const target = option('--target', 'chrome');
const watch = flag('--watch');
const sourcemap = flag('--sourcemap') || watch ? 'inline' : false;
if (!['chrome', 'chrome-e2e', 'firefox'].includes(target)) {
  console.error(`unknown target ${target}: chrome | chrome-e2e | firefox`);
  process.exit(2);
}

const base = JSON.parse(readFileSync(join(here, 'manifest.json'), 'utf8'));
const version = base.version;
const outDir = join(root, 'dist-extension', target);

/** Every source file, hashed: the content script's global key changes with the code. */
function sourceHash() {
  const h = createHash('sha256');
  const walk = (dir) => {
    for (const name of readdirSync(dir).sort()) {
      const p = join(dir, name);
      if (statSync(p).isDirectory()) walk(p);
      else {
        h.update(relative(here, p));
        h.update(readFileSync(p));
      }
    }
  };
  walk(join(here, 'src'));
  return h.digest('hex').slice(0, 10);
}

function manifestFor(t) {
  const m = structuredClone(base);
  if (t === 'chrome-e2e') {
    // Optional permissions can only be granted through a browser prompt that
    // automation cannot click, so the test build asks for them up front: the
    // fixture hosts and the optional APIs. Everything else is the real manifest.
    m.name = 'Mother Claude (e2e)';
    m.host_permissions = ['http://127.0.0.1/*', 'http://localhost/*'];
    m.permissions = [...m.permissions, 'debugger', 'webNavigation', 'downloads'];
    m.optional_permissions = [];
  }
  if (t === 'firefox') {
    delete m.key;
    delete m.minimum_chrome_version;
    delete m.side_panel;
    m.background = { scripts: ['background.js'] };
    m.sidebar_action = {
      default_title: 'Mother Claude',
      default_panel: 'sidepanel.html',
      default_icon: m.action.default_icon,
    };
    m.permissions = m.permissions.filter((p) => p !== 'sidePanel');
    // Firefox has no chrome.debugger: trusted input, background screenshots,
    // console and dialogs answer `debugger_unavailable` / `permission_needed`.
    m.optional_permissions = m.optional_permissions.filter((p) => p !== 'debugger');
    m.browser_specific_settings = {
      gecko: { id: 'browser-extension@motherclaude.dev', strict_min_version: '128.0' },
    };
  }
  return m;
}

function icons(dir) {
  const source = join(root, 'public', 'icons', 'icon-512.png');
  const img = decodePng(readFileSync(source));
  mkdirSync(join(dir, 'icons'), { recursive: true });
  for (const size of [16, 32, 48, 128]) {
    writeFileSync(join(dir, 'icons', `icon-${size}.png`), encodePng(resize(img, size)));
  }
}

function statics(dir) {
  for (const f of ['sidepanel.html', 'options.html', 'lease.html', 'ui.css']) {
    cpSync(join(here, 'src', 'ui', f), join(dir, f));
  }
}

function listFiles(dir) {
  const out = [];
  const walk = (d) => {
    for (const name of readdirSync(d).sort()) {
      const p = join(d, name);
      if (statSync(p).isDirectory()) walk(p);
      else out.push(p);
    }
  };
  walk(dir);
  return out;
}

const buildId = `${version.replace(/\W/g, '_')}_${sourceHash()}`;

const options = {
  entryPoints: {
    background: join(here, 'src/background/index.ts'),
    content: join(here, 'src/content/index.ts'),
    sidepanel: join(here, 'src/ui/sidepanel.ts'),
    options: join(here, 'src/ui/options.ts'),
    lease: join(here, 'src/ui/lease.ts'),
  },
  outdir: outDir,
  bundle: true,
  format: 'iife',
  platform: 'browser',
  target: target === 'firefox' ? ['firefox128'] : ['chrome116'],
  charset: 'utf8',
  legalComments: 'none',
  sourcemap,
  logLevel: 'warning',
  define: {
    __MC_VERSION__: JSON.stringify(version),
    __MC_CONTENT_KEY__: JSON.stringify(`__motherClaude_${buildId}`),
    // The e2e build can never talk to a real app on 6725: until the harness
    // configures the mock's port, it points at port 1, where nothing listens.
    __MC_DEFAULT_PORT__: target === 'chrome-e2e' ? '1' : '6725',
  },
};

function finish() {
  statics(outDir);
  icons(outDir);
  writeFileSync(join(outDir, 'manifest.json'), `${JSON.stringify(manifestFor(target), null, 2)}\n`);
}

if (watch) {
  rmSync(outDir, { recursive: true, force: true });
  mkdirSync(outDir, { recursive: true });
  const ctx = await esbuild.context({
    ...options,
    plugins: [{ name: 'statics', setup: (b) => b.onEnd(() => finish()) }],
  });
  await ctx.watch();
  console.log(
    `watching → ${relative(root, outDir)} (reload the extension in chrome://extensions after changes)`,
  );
} else {
  rmSync(outDir, { recursive: true, force: true });
  mkdirSync(outDir, { recursive: true });
  await esbuild.build(options);
  finish();
  const files = listFiles(outDir);
  const bytes = files.reduce((n, f) => n + statSync(f).size, 0);
  console.log(
    `built ${relative(root, outDir)}: ${files.length} files, ${(bytes / 1024).toFixed(0)} KiB`,
  );
  if (target !== 'chrome-e2e') {
    const archive = join(root, 'dist-extension', `mother-claude-${target}-${version}.zip`);
    writeFileSync(
      archive,
      zip(files.map((f) => ({ name: relative(outDir, f), data: readFileSync(f) }))),
    );
    console.log(
      `zipped ${relative(root, archive)} (${(statSync(archive).size / 1024).toFixed(0)} KiB)`,
    );
  }
  if (!existsSync(join(outDir, 'manifest.json'))) process.exit(1);
}
