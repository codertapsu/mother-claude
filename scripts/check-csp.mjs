#!/usr/bin/env node
/**
 * Fails the build when the production `index.html` would break under the
 * server's Content-Security-Policy (`script-src 'self'`, no inline allowances):
 *
 *   - an inline `<script>` (one without `src`), or
 *   - an inline event-handler attribute (`onload=`, `onclick=`, …), or
 *   - a `javascript:` URL.
 *
 * Angular's critical-CSS inlining emits `<link … media="print"
 * onload="this.media='all'">`, which the CSP blocks — so `inlineCritical` is off
 * in `angular.json`, and this check keeps it that way.
 *
 *   node scripts/check-csp.mjs [dist-dir]
 *
 * Runs as `postbuild`, so `npm run build` (and the Tauri bundle) fail loudly.
 */
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const here = path.dirname(fileURLToPath(import.meta.url));
const root = path.resolve(process.argv[2] ?? path.join(here, '..', 'dist'));

/** Every `index.html` under `dir`. */
function findIndexFiles(dir) {
  const out = [];
  let entries = [];
  try {
    entries = fs.readdirSync(dir, { withFileTypes: true });
  } catch {
    return out;
  }
  for (const entry of entries) {
    const full = path.join(dir, entry.name);
    if (entry.isDirectory()) {
      out.push(...findIndexFiles(full));
    } else if (entry.name === 'index.html') {
      out.push(full);
    }
  }
  return out;
}

/** Problems in one HTML document, as human-readable strings. */
export function cspProblems(html) {
  const problems = [];
  // Comments cannot execute; ignore them.
  const text = html.replace(/<!--[\s\S]*?-->/g, '');
  for (const match of text.matchAll(/<script\b([^>]*)>/gi)) {
    if (!/\bsrc\s*=/i.test(match[1])) {
      problems.push(`inline <script>: ${match[0]}`);
    }
  }
  for (const match of text.matchAll(/<[a-z][^>]*>/gi)) {
    const tag = match[0];
    const handler = tag.match(/\son[a-z]+\s*=/i);
    if (handler) {
      problems.push(`inline event handler ${handler[0].trim()} in ${tag}`);
    }
    if (/=\s*["']?\s*javascript:/i.test(tag)) {
      problems.push(`javascript: URL in ${tag}`);
    }
  }
  return problems;
}

const isMain = process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url);
if (isMain) {
  const files = findIndexFiles(root);
  if (files.length === 0) {
    console.error(`check-csp: no index.html under ${root}`);
    process.exit(1);
  }
  let failed = false;
  for (const file of files) {
    const problems = cspProblems(fs.readFileSync(file, 'utf8'));
    for (const p of problems) {
      console.error(`check-csp: ${path.relative(process.cwd(), file)}: ${p}`);
      failed = true;
    }
  }
  if (failed) {
    console.error(
      "check-csp: the server sends `script-src 'self'`; remove inline scripts and handlers " +
        '(keep `inlineCritical: false` in angular.json).',
    );
    process.exit(1);
  }
  console.log(`check-csp: ${files.length} index.html file(s) are CSP-clean.`);
}
