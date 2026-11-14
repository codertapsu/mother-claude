// Permissions: what Claude may do without asking you (`/api/permissions`).
// Each one is an answer you remembered on a card (*Allow for…*): a tool in a
// session, in a project or everywhere on this computer; a command or a task
// a session sends to another computer. Anything else, and anything
// dangerous, still asks. The words for them live here, shared by the card,
// the Permissions page and its sheet.

import { computerNoun, tr } from './i18n/i18n';
import type { GrantKind, GrantOffer, GrantScope, PatternKind } from './models';

/** A remembered answer (`GET /api/permissions`). */
export interface PermissionRule {
  /** `pr_` and 12 hex digits. */
  id: string;
  kind: GrantKind;
  /** A tool's, as in {@link GrantOffer.tool}. */
  tool?: string;
  /** Empty: any. */
  patterns: string[];
  patternKind?: PatternKind;
  scope: GrantScope;
  session?: string;
  sessionName?: string;
  project?: string;
  projectName?: string;
  /** A run's or a task's: the computer it runs on. */
  machine?: string;
  machineName?: string;
  /** Epoch ms. */
  createdAt: number;
  /** Who saved it ("Mac", a device's name). */
  createdBy: string;
  /** Epoch ms, once it allowed something. */
  usedAt?: number;
  uses: number;
}

/** `GET /api/permissions` (and the answer to every change of the list). */
export interface PermissionsView {
  rules: PermissionRule[];
  /** Mother Claude asks for everything; the rules are kept. */
  paused: boolean;
}

/** `PATCH /api/permissions/{id}`. */
export interface PermissionRulePatch {
  patterns?: string[];
  scope?: GrantScope;
}

/** `{t:'permissions'}`: they changed — refetch. */
export interface PermissionsFrame {
  t: 'permissions';
}

/** Patterns a permission holds at most. */
export const MAX_PATTERNS = 20;

/** The scopes, narrowest first. */
export const SCOPES: readonly GrantScope[] = ['session', 'project', 'computer'];

/** A project or the whole computer needs the dangerous right; this
 * session is open to anyone who may answer. */
export function needsDangerous(scope: GrantScope): boolean {
  return scope !== 'session';
}

/** `to` reaches further than `from`. */
export function wider(from: GrantScope, to: GrantScope): boolean {
  return SCOPES.indexOf(to) > SCOPES.indexOf(from);
}

/** The patterns typed: trimmed, without the empty ones and repeats. */
export function cleanPatterns(rows: readonly string[]): string[] {
  const out: string[] = [];
  for (const row of rows) {
    const p = row.trim();
    if (p && !out.includes(p)) out.push(p);
  }
  return out;
}

// ──────────────────────────────────────────────────────────── Reading

const str = (v: unknown): string | undefined => (typeof v === 'string' && v ? v : undefined);
const num = (v: unknown): number | undefined =>
  typeof v === 'number' && Number.isFinite(v) ? v : undefined;
const oneOf = <T extends string>(v: unknown, list: readonly T[]): T | undefined =>
  list.includes(v as T) ? (v as T) : undefined;

const KINDS: readonly GrantKind[] = ['tool', 'run', 'task'];
const PATTERN_KINDS: readonly PatternKind[] = ['command', 'path', 'domain'];

/** One rule as the server sent it, read field by field (null: not one). */
export function normalizeRule(v: unknown): PermissionRule | null {
  if (!v || typeof v !== 'object') return null;
  const o = v as Record<string, unknown>;
  const id = str(o['id']);
  const kind = oneOf(o['kind'], KINDS);
  const scope = oneOf(o['scope'], SCOPES);
  if (!id || !kind || !scope) return null;
  const rule: PermissionRule = {
    id,
    kind,
    patterns: Array.isArray(o['patterns'])
      ? o['patterns'].filter((p): p is string => typeof p === 'string' && !!p)
      : [],
    scope,
    createdAt: num(o['createdAt']) ?? 0,
    createdBy: str(o['createdBy']) ?? '',
    uses: num(o['uses']) ?? 0,
  };
  const patternKind = oneOf(o['patternKind'], PATTERN_KINDS);
  if (patternKind) rule.patternKind = patternKind;
  for (const key of [
    'tool',
    'session',
    'sessionName',
    'project',
    'projectName',
    'machine',
    'machineName',
  ] as const) {
    const s = str(o[key]);
    if (s) rule[key] = s;
  }
  const usedAt = num(o['usedAt']);
  if (usedAt) rule.usedAt = usedAt;
  return rule;
}

/** `{rules, paused}` as the server sent it (tolerant: a newer server's
 * rule of an unknown kind is left out). */
export function normalizePermissions(v: unknown): PermissionsView {
  const o = (v && typeof v === 'object' ? v : {}) as Record<string, unknown>;
  const rules = Array.isArray(o['rules'])
    ? o['rules'].map(normalizeRule).filter((r): r is PermissionRule => !!r)
    : [];
  return { rules, paused: o['paused'] === true };
}

// ──────────────────────────────────────────────────────────── Words

/** What a permission is for (an offer or a rule). */
export interface GrantTarget {
  kind: GrantKind;
  tool?: string;
  machineName?: string;
}

/** The tools a permission names by what they do. */
type Family = 'shell' | 'edit' | 'read' | 'fetch' | 'search' | 'other';

function familyOf(tool: string | undefined): Family {
  switch (tool) {
    case 'Bash':
    case 'PowerShell':
      return 'shell';
    case 'Edit':
    case 'Write':
    case 'MultiEdit':
    case 'NotebookEdit':
      return 'edit';
    case 'Read':
    case 'Glob':
    case 'Grep':
    case 'LS':
      return 'read';
    case 'WebFetch':
      return 'fetch';
    case 'WebSearch':
      return 'search';
    default:
      return 'other';
  }
}

const machineOf = (t: GrantTarget) => t.machineName || tr('the other computer');

/** Everything a permission without patterns allows: "Any Bash command",
 * "Changing any file", "Any command on MacBook". */
export function anyOf(t: GrantTarget): string {
  if (t.kind === 'run') return tr('Any command on {name}', { name: machineOf(t) });
  if (t.kind === 'task') return tr('Any task handed to {name}', { name: machineOf(t) });
  const tool = t.tool || tr('this tool');
  switch (familyOf(t.tool)) {
    case 'shell':
      return tr('Any {tool} command', { tool });
    case 'edit':
      return tr('Changing any file');
    case 'read':
      return tr('Reading any file');
    case 'fetch':
      return tr('Fetching any web page');
    case 'search':
      return tr('Any web search');
    default:
      return tr('Every use of {tool}', { tool });
  }
}

/** What a permission with patterns allows, its patterns after it: "Bash
 * commands that start with", "Changing files that match". */
export function matchingOf(t: GrantTarget & { patternKind?: PatternKind }): string {
  if (t.kind === 'run') return tr('Commands on {name} that start with', { name: machineOf(t) });
  const tool = t.tool || tr('this tool');
  const family = t.kind === 'task' ? 'other' : familyOf(t.tool);
  switch (family) {
    case 'shell':
      return tr('{tool} commands that start with', { tool });
    case 'edit':
      return tr('Changing files that match');
    case 'read':
      return tr('Reading files that match');
    case 'fetch':
      return tr('Fetching pages on');
    case 'search':
      return tr('Web searches on');
    default:
      break;
  }
  if (t.kind === 'task') return tr('Tasks handed to {name} that match', { name: machineOf(t) });
  switch (t.patternKind) {
    case 'command':
      return tr('{tool} with commands that start with', { tool });
    case 'path':
      return tr('{tool} on files that match', { tool });
    case 'domain':
      return tr('{tool} on these sites', { tool });
    default:
      return tr('{tool}, when it matches', { tool });
  }
}

/** What a rule allows, before its patterns (shown as chips). */
export function ruleAction(r: PermissionRule): string {
  return r.patterns.length ? matchingOf(r) : anyOf(r);
}

/** The heading over the pattern rows. */
export function patternsHeading(kind: PatternKind): string {
  switch (kind) {
    case 'path':
      return tr('Only files that match:');
    case 'domain':
      return tr('Only pages on these sites:');
    default:
      return tr('Only commands that start with:');
  }
}

/** What a pattern of this kind means, in one line (`code` marks). */
export function patternHint(kind: PatternKind): string {
  switch (kind) {
    case 'path':
      return tr('Files matching these, relative to the project (`src/**`, `*.md`).');
    case 'domain':
      return tr('Pages on these sites (`docs.rs`, `*.github.com`).');
    default:
      return tr(
        'A command that starts with these words: `cargo test` covers `cargo test --lib`. One that chains or redirects (`;` `&&` `|` `>` `$(`) always asks.',
      );
  }
}

/** An example for an empty pattern row. */
export function patternExample(kind: PatternKind): string {
  return kind === 'path' ? 'src/**' : kind === 'domain' ? 'docs.rs' : 'npm test';
}

/** The names a scope is told by. */
export interface ScopeNames {
  /** The project's name. */
  project?: string;
  /** The session's name: "In the session …" (absent: "this session"). */
  session?: string;
  /** Another computer the request waits at: "Everywhere on …" (absent:
   * this one). */
  host?: string | null;
}

/** A scope as a choice: a tool's "In this session", "In shop (every
 * session)", "Everywhere on this Mac"; a run's or a task's "From this
 * session", "From shop", "From every project". */
export function scopeLabel(kind: GrantKind, scope: GrantScope, names: ScopeNames = {}): string {
  const project = names.project || tr('this project');
  if (kind === 'tool') {
    switch (scope) {
      case 'session':
        return names.session
          ? tr('In the session {name}', { name: names.session })
          : tr('In this session');
      case 'project':
        return tr('In {project} (every session)', { project });
      default:
        return names.host
          ? tr('Everywhere on {name}', { name: names.host })
          : tr('Everywhere on this {computer}', { computer: computerNoun() });
    }
  }
  switch (scope) {
    case 'session':
      return names.session
        ? tr('From the session {name}', { name: names.session })
        : tr('From this session');
    case 'project':
      return tr('From {project}', { project });
    default:
      return tr('From every project');
  }
}

/** A one-tap choice of *Allow for…*: its label, and all it covers. */
export interface QuickChoice {
  scope: GrantScope;
  label: string;
  hint: string;
}

/** The one-tap choices an offer has, narrowest first (no project: none
 * for it). Worded for what is asked: a Bash command in shop — "In this
 * session", "In shop (every session)", "Everywhere on this Mac"; a run on
 * MacBook — "Commands from this session", "Commands from shop", "Every
 * command on MacBook". */
export function quickChoices(offer: GrantOffer, host: string | null = null): QuickChoice[] {
  const project = offer.projectName || tr('this project');
  const name = machineOf(offer);
  const scopes = SCOPES.filter((s) => s !== 'project' || !!offer.project);
  return scopes.map((scope) => {
    switch (offer.kind) {
      case 'run':
        return {
          scope,
          label:
            scope === 'session'
              ? tr('Commands from this session')
              : scope === 'project'
                ? tr('Commands from {project}', { project })
                : tr('Every command on {name}', { name }),
          hint:
            scope === 'session'
              ? tr('Any command this session runs on {name}.', { name })
              : scope === 'project'
                ? tr('Any command a session of {project} runs on {name}.', { project, name })
                : tr('From every session, in every project.'),
        };
      case 'task':
        return {
          scope,
          label:
            scope === 'session'
              ? tr('Tasks from this session')
              : scope === 'project'
                ? tr('Tasks from {project}', { project })
                : tr('Every task handed to {name}', { name }),
          hint:
            scope === 'session'
              ? tr('Any task this session hands to {name}.', { name })
              : scope === 'project'
                ? tr('Any task a session of {project} hands to {name}.', { project, name })
                : tr('From every session, in every project.'),
        };
      default: {
        const what = anyOf(offer);
        return {
          scope,
          label: scopeLabel('tool', scope, { project, host }),
          hint:
            scope === 'session'
              ? tr('{what}, in this session only.', { what })
              : scope === 'project'
                ? tr('{what}, in every session of {project}.', { what, project })
                : host
                  ? tr('{what}, in every project on {name}.', { what, name: host })
                  : tr('{what}, in every project on this {computer}.', {
                      what,
                      computer: computerNoun(),
                    }),
        };
      }
    }
  });
}

/** What the card says once the answer is allowed and remembered. */
export function rememberedText(
  offer: GrantOffer,
  scope: GrantScope,
  patterns: readonly string[] = [],
  host: string | null = null,
): string {
  if (patterns.length) {
    return tr('Allowed and remembered: {where}, matching {patterns}', {
      where: scopeLabel(offer.kind, scope, { project: offer.projectName, host }),
      patterns: patterns.join(', '),
    });
  }
  const choice = quickChoices(offer, host).find((c) => c.scope === scope);
  return tr('Allowed and remembered: {where}', {
    where: choice?.label ?? scopeLabel(offer.kind, scope, { project: offer.projectName, host }),
  });
}
