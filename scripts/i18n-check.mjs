#!/usr/bin/env node
// Checks the interface's translations (docs/I18N.md).
//
//   node scripts/i18n-check.mjs            every key used has a translation,
//                                          placeholders match, no key is
//                                          translated two ways
//   node scripts/i18n-check.mjs --text     also lists template text that is
//                                          not translated yet, per file
//   node scripts/i18n-check.mjs --unused   also lists translations no code uses
//   node scripts/i18n-check.mjs --files a,b only those source files (paths
//                                          relative to the repository)
//   node scripts/i18n-check.mjs --help-labels
//                                          also lists **bold** labels of the
//                                          Vietnamese Help that no translated
//                                          interface text matches (a hint: bold
//                                          is not always a label)
//
// Keys are the English text: `'…' | t` in templates, `tr('…')` in code.
// Exit status 1 when something is missing or wrong.
import { readFileSync, readdirSync, statSync } from 'node:fs';
import { join, relative, sep } from 'node:path';
import { fileURLToPath } from 'node:url';

// A path, not a URL's: on Windows `URL.pathname` is `/D:/…`.
const root = fileURLToPath(new URL('..', import.meta.url));
/** A path with `/` between its parts, whatever the system's separator. */
const slashed = (p) => p.split(sep).join('/');
const args = process.argv.slice(2);
const wantText = args.includes('--text');
const wantUnused = args.includes('--unused');
const wantHelpLabels = args.includes('--help-labels');
const onlyAt = args.indexOf('--files');
const only = onlyAt >= 0 ? new Set(args[onlyAt + 1].split(',').map((f) => f.trim())) : null;

const SRC = join(root, 'src/app');
const DICTS = join(SRC, 'i18n/vi');

function walk(dir) {
  return readdirSync(dir).flatMap((name) => {
    const p = join(dir, name);
    return statSync(p).isDirectory() ? walk(p) : [slashed(p)];
  });
}

const sources = walk(SRC).filter(
  (p) =>
    (p.endsWith('.ts') || p.endsWith('.html')) &&
    !p.endsWith('.spec.ts') &&
    !p.endsWith('.testing.ts') &&
    !p.includes('/mock/') &&
    !p.includes('/i18n/vi/') &&
    !p.includes('/pages/help/vi/') &&
    !p.includes('/core/i18n/') &&
    !p.endsWith('help-topics.ts'),
);

/** A JavaScript string literal's body, unescaped. */
function unescape(body) {
  return body.replace(/\\(u\{[0-9a-fA-F]+\}|u[0-9a-fA-F]{4}|.)/g, (_, e) => {
    if (e === 'n') return '\n';
    if (e === 't') return '\t';
    if (e.startsWith('u{')) return String.fromCodePoint(parseInt(e.slice(2, -1), 16));
    if (e.startsWith('u') && e.length === 5) return String.fromCharCode(parseInt(e.slice(1), 16));
    return e;
  });
}

const STR = String.raw`(?:'((?:\\.|[^'\\])*)'|"((?:\\.|[^"\\])*)")`;
const PIPE = new RegExp(STR + String.raw`\s*\|\s*t\b(?!\w)`, 'g');
const CALL = new RegExp(
  String.raw`\btr\(\s*(?:` +
    STR +
    String.raw`|` +
    '`' +
    String.raw`((?:\\.|[^` +
    '`' +
    String.raw`\\$])*)` +
    '`' +
    String.raw`)`,
  'g',
);

/** `@if (…)`, `@for (…)` and the like removed, parentheses balanced. */
function stripControlFlow(t) {
  let out = '';
  for (let i = 0; i < t.length;) {
    const m = /^@(if|for|switch|case|else if|defer|let)\b\s*\(/.exec(t.slice(i));
    if (!m) {
      out += t[i++];
      continue;
    }
    let depth = 0;
    let j = i + m[0].length - 1;
    for (; j < t.length; j++) {
      if (t[j] === '(') depth++;
      else if (t[j] === ')' && --depth === 0) break;
    }
    out += ' ';
    i = j + 1;
  }
  return out;
}

/** Text that is a name, the same in every language. */
const NAMES = new Set(['Mother Claude', 'Claude', 'Claude Code', 'API', 'MCP', 'tmux', 'OK']);

/** key → files that use it */
const used = new Map();
const textLeft = [];

for (const file of sources) {
  const rel = slashed(relative(root, file));
  if (only && !only.has(rel)) continue;
  const src = readFileSync(file, 'utf8');
  for (const re of [PIPE, CALL]) {
    re.lastIndex = 0;
    for (const m of src.matchAll(re)) {
      const body = m[1] ?? m[2] ?? m[3];
      if (body === undefined) continue;
      const key = unescape(body);
      if (!used.has(key)) used.set(key, new Set());
      used.get(key).add(rel);
    }
  }
  if (wantText) {
    const templates = file.endsWith('.html')
      ? [src]
      : [
          ...src.matchAll(
            /template:\s*`([\s\S]*?)`,\s*\n\s*(?:styles|styleUrl|host|providers|\})/g,
          ),
        ].map((m) => m[1]);
    for (const t of templates) {
      const bare = stripControlFlow(
        t.replace(/<!--[\s\S]*?-->/g, ' ').replace(/\{\{[\s\S]*?\}\}/g, ' '),
      ).replace(/@(else|default|empty|placeholder|loading)\b/g, ' ');
      for (const m of bare.matchAll(/>([^<>]+)</g)) {
        const text = m[1].replace(/[{}]/g, ' ').replace(/\s+/g, ' ').trim();
        if (/[A-Za-z]{2,}/.test(text) && !/^&[a-z]+;$/.test(text) && !NAMES.has(text)) {
          textLeft.push(`${rel}: ${text}`);
        }
      }
      for (const m of t.matchAll(
        /\s(aria-label|title|placeholder|label)="([^"{}]*[A-Za-z]{3,}[^"{}]*)"/g,
      )) {
        textLeft.push(`${rel}: ${m[1]}="${m[2]}"`);
      }
    }
  }
}

/** key → { text, file } from every dictionary file */
const dict = new Map();
const conflicts = [];
for (const name of readdirSync(DICTS).filter((n) => n.endsWith('.json'))) {
  const entries = JSON.parse(readFileSync(join(DICTS, name), 'utf8'));
  for (const [key, text] of Object.entries(entries)) {
    const had = dict.get(key);
    if (had && had.text !== text)
      conflicts.push(`"${key}": ${had.file} "${had.text}" / ${name} "${text}"`);
    if (!had) dict.set(key, { text, file: name });
  }
}

const placeholders = (s) =>
  [...s.matchAll(/\{(\w+)\}/g)]
    .map((m) => m[1])
    .sort()
    .join(',');
const missing = [];
const mismatched = [];
for (const [key, files] of used) {
  if (!/[A-Za-z]/.test(key.split('||')[0])) continue;
  const d = dict.get(key);
  if (!d) missing.push(`"${key}"  (${[...files].join(', ')})`);
  else if (placeholders(key.split('||')[0]) !== placeholders(d.text)) {
    mismatched.push(`"${key}" → "${d.text}" (${d.file})`);
  }
}

const report = (title, list) => {
  if (!list.length) return;
  console.log(`\n${title} (${list.length}):`);
  for (const line of list.sort()) console.log(`  ${line}`);
};
report('Used but not translated', missing);
report('Placeholders differ', mismatched);
report('Translated two ways', conflicts);
if (wantText) report('Template text not translated yet', textLeft);
if (wantHelpLabels) {
  const shown = new Set([...dict.values()].map((d) => d.text.replace(/[….:]+$/, '').trim()));
  const helpDir = join(SRC, 'pages/help/vi');
  const labels = new Map();
  for (const name of readdirSync(helpDir).filter(
    (n) => /^topics-.*\.ts$/.test(n) && !n.endsWith('.spec.ts'),
  )) {
    for (const m of readFileSync(join(helpDir, name), 'utf8').matchAll(/\*\*([^*]+)\*\*/g)) {
      const label = m[1].replace(/[….:]+$/, '').trim();
      if (/[a-zA-ZÀ-ỹ]/.test(label) && !shown.has(label)) labels.set(label, name);
    }
  }
  report(
    'Bold in the Vietnamese Help that no interface text matches',
    [...labels].map(([l, f]) => `${l} (${f})`),
  );
}
if (wantUnused) {
  report(
    'Translations no code uses',
    [...dict.keys()].filter((k) => !used.has(k)).map((k) => `"${k}" (${dict.get(k).file})`),
  );
}
console.log(
  `\n${used.size} keys used, ${dict.size} translated, ${missing.length} missing, ${mismatched.length} with other placeholders, ${conflicts.length} translated two ways.`,
);
process.exit(missing.length || mismatched.length || conflicts.length ? 1 : 0);
