// The visual template editor's model (docs/TEAMS.md "Templates"): pure
// functions over a parsed `TeamTemplate` — where its nodes go, what an edit
// changes, which node or edge a problem is about. Every edit returns a new
// template and changes only what it is about: fields this version does not
// know (on the template, a role, the integration, a handoff) stay as they
// were, and a template typed as JSON may hold anything, so every read is
// tolerant.

import { tr } from '../../core/i18n/i18n';
import {
  HANDOFF_NOTE_MAX,
  Handoff,
  Integration,
  MERGE_NODE,
  NodePosition,
  ROLE_ID_MAX,
  RoleDef,
  TeamTemplate,
  TemplateIssue,
} from '../../core/teams';

/** The grid nodes snap to. */
export const GRID = 16;
/** Every node's size: a role's, and the merge node's. */
export const NODE_W = 184;
export const NODE_H = 104;
/** Room between two layers, and between the nodes of one layer. */
const GAP_X = 104;
const GAP_Y = 24;
/** Where the first layer starts. */
const ORIGIN = 32;
/** The arrowhead's length. */
const ARROW = 9;

export interface Box {
  x: number;
  y: number;
  w: number;
  h: number;
}

/** What the editor has selected. */
export type Selection =
  | { kind: 'role'; id: string }
  | { kind: 'merge' }
  | { kind: 'handoff'; from: string; to: string }
  | null;

/** What a problem is about. */
export type IssueTarget =
  | { kind: 'role'; id: string; field?: string }
  | { kind: 'handoff'; from: string; to: string; field?: string }
  | { kind: 'merge'; field?: string }
  | { kind: 'template'; field?: string };

/** An edge's curve, its arrowhead, its middle (where its note goes) and how
 * wide a note may be there. */
export interface EdgeGeometry {
  d: string;
  head: string;
  mid: NodePosition;
  room: number;
}

type Fields = Record<string, unknown>;

const isObject = (v: unknown): v is Fields => !!v && typeof v === 'object' && !Array.isArray(v);

/** `o` with `patch` applied: a key set to `undefined` is removed, the rest
 * are set (new keys go last, the others keep their place). */
function withFields<T extends object>(o: T, patch: Fields): T {
  const next: Fields = Object.assign<Fields, T>({}, o);
  for (const [k, v] of Object.entries(patch)) {
    if (v === undefined) delete next[k];
    else next[k] = v;
  }
  return next as T;
}

function without<T extends object>(o: T, key: string): T {
  const next: Fields = Object.assign<Fields, T>({}, o);
  delete next[key];
  return next as T;
}

// ───────────────────────────────────────────────────────────── reading

/** The roles the editor draws: objects with an id, the first of each id. */
export function rolesOf(t: TeamTemplate): RoleDef[] {
  if (!Array.isArray(t.roles)) return [];
  const seen = new Set<string>();
  return t.roles.filter((r): r is RoleDef => {
    if (!isObject(r) || typeof r.id !== 'string' || seen.has(r.id)) return false;
    seen.add(r.id);
    return true;
  });
}

/** The handoffs that name two roles (whether or not those exist). */
export function flowOf(t: TeamTemplate): Handoff[] {
  if (!Array.isArray(t.flow)) return [];
  return t.flow.filter(
    (h): h is Handoff => isObject(h) && typeof h.from === 'string' && typeof h.to === 'string',
  );
}

/** The handoffs the canvas draws: between two roles of the template, two
 * different ones, the first entry of each pair. */
export function drawnFlow(t: TeamTemplate): Handoff[] {
  const ids = new Set(rolesOf(t).map((r) => r.id));
  const seen = new Set<string>();
  return flowOf(t).filter((h) => {
    const key = `${h.from}\u0000${h.to}`;
    if (h.from === h.to || !ids.has(h.from) || !ids.has(h.to) || seen.has(key)) return false;
    seen.add(key);
    return true;
  });
}

export function roleOf(t: TeamTemplate, id: string): RoleDef | undefined {
  return rolesOf(t).find((r) => r.id === id);
}

/** A role's title, else its id. */
export function titleOf(t: TeamTemplate, id: string): string {
  const title = roleOf(t, id)?.title;
  return typeof title === 'string' && title.trim() ? title : id;
}

export function leadOf(t: TeamTemplate): RoleDef | undefined {
  return rolesOf(t).find((r) => r.lead === true);
}

function integrationOf(t: TeamTemplate): Integration {
  return isObject(t.integration) ? (t.integration as Integration) : {};
}

/** The role that merges: `integration.mergeBy` when set (`explicit`), else
 * the lead. */
export function mergerOf(t: TeamTemplate): { id: string; explicit: boolean } | null {
  const m = integrationOf(t).mergeBy;
  if (typeof m === 'string') return { id: m, explicit: true };
  const lead = leadOf(t);
  return lead ? { id: lead.id, explicit: false } : null;
}

/** A role id: lower case letters, digits and inner `-`, at most 32. */
export function isRoleId(id: string): boolean {
  return (
    id.length > 0 &&
    id.length <= ROLE_ID_MAX &&
    /^[a-z0-9-]+$/.test(id) &&
    !id.startsWith('-') &&
    !id.endsWith('-')
  );
}

/** A template id: as a role's, at most 64. */
export function isTemplateId(id: string): boolean {
  return /^[a-z0-9](?:[a-z0-9-]{0,62}[a-z0-9])?$/.test(id);
}

/** A name as a template id ("Weekly newsletter" → `weekly-newsletter`). */
export function slugId(name: string): string {
  return name
    .toLowerCase()
    .replace(/đ/g, 'd')
    .normalize('NFD')
    .replace(/[\u0300-\u036f]/g, '')
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-+/, '')
    .slice(0, 64)
    .replace(/-+$/, '');
}

// ───────────────────────────────────────────────────────────── layout

export const snap = (v: number): number => Math.round(v / GRID) * GRID;

export function snapPos(p: NodePosition): NodePosition {
  return { x: snap(p.x), y: snap(p.y) };
}

function position(v: unknown): NodePosition | undefined {
  if (!isObject(v)) return undefined;
  const { x, y } = v;
  return typeof x === 'number' && Number.isFinite(x) && typeof y === 'number' && Number.isFinite(y)
    ? { x, y }
    : undefined;
}

export function boxOf(p: NodePosition): Box {
  return { x: p.x, y: p.y, w: NODE_W, h: NODE_H };
}

/** The box around `boxes` (one node's at the origin when there are none). */
export function bounds(boxes: readonly Box[]): Box {
  if (!boxes.length) return { x: 0, y: 0, w: NODE_W, h: NODE_H };
  const x = Math.min(...boxes.map((b) => b.x));
  const y = Math.min(...boxes.map((b) => b.y));
  const right = Math.max(...boxes.map((b) => b.x + b.w));
  const bottom = Math.max(...boxes.map((b) => b.y + b.h));
  return { x, y, w: right - x, h: bottom - y };
}

/** The nodes: every role, then the merge node. */
export function nodeKeys(t: TeamTemplate): string[] {
  return [...rolesOf(t).map((r) => r.id), MERGE_NODE];
}

/**
 * Where every node goes when nothing placed it: layers from left to right by
 * the flow — the lead first, each role after the roles that hand work to it
 * (a cycle is broken where it closes; work handed back to the lead does not
 * move it), the merge node last — each layer centred, its roles in the
 * template's order and then near the roles they get work from.
 */
export function autoLayout(t: TeamTemplate): Record<string, NodePosition> {
  const ids = rolesOf(t).map((r) => r.id);
  const lead = leadOf(t)?.id;
  const next = new Map<string, string[]>(ids.map((id) => [id, []]));
  for (const h of drawnFlow(t)) {
    if (h.to !== lead) next.get(h.from)?.push(h.to);
  }
  // Depth first from the lead, then from each role in order: an edge back to
  // a role on the current path closes a cycle and is left out.
  const state = new Map<string, 'path' | 'done'>();
  const kept = new Map<string, string[]>(ids.map((id) => [id, []]));
  const finished: string[] = [];
  const visit = (id: string) => {
    state.set(id, 'path');
    for (const to of next.get(id) ?? []) {
      if (state.get(to) === 'path') continue;
      kept.get(id)?.push(to);
      if (!state.has(to)) visit(to);
    }
    state.set(id, 'done');
    finished.push(id);
  };
  for (const id of lead ? [lead, ...ids] : ids) {
    if (!state.has(id)) visit(id);
  }
  // The longest path to each role, in topological order (finish order
  // reversed): the lead in layer 0, every other role from layer 1.
  const layer = new Map<string, number>(ids.map((id) => [id, lead && id !== lead ? 1 : 0]));
  for (const id of finished.reverse()) {
    for (const to of kept.get(id) ?? []) {
      layer.set(to, Math.max(layer.get(to) ?? 0, (layer.get(id) ?? 0) + 1));
    }
  }
  const depth = ids.length ? Math.max(...layer.values()) : -1;
  const layers: string[][] = Array.from({ length: depth + 1 }, () => []);
  for (const id of ids) layers[layer.get(id) ?? 0].push(id);
  const from = new Map<string, string[]>(ids.map((id) => [id, []]));
  for (const [a, tos] of kept) for (const b of tos) from.get(b)?.push(a);

  const height = (n: number) => n * NODE_H + Math.max(0, n - 1) * GAP_Y;
  const tallest = Math.max(NODE_H, ...layers.map((l) => height(l.length)));
  const middle = ORIGIN + tallest / 2;
  const column = (i: number) => snap(ORIGIN + i * (NODE_W + GAP_X));
  const centre = new Map<string, number>();
  const out: Record<string, NodePosition> = {};
  layers.forEach((members, i) => {
    const top = middle - height(members.length) / 2;
    const slot = (j: number) => top + j * (NODE_H + GAP_Y) + NODE_H / 2;
    const keyed = members.map((id, j) => {
      const ys = (from.get(id) ?? [])
        .map((p) => centre.get(p))
        .filter((y): y is number => y !== undefined);
      return { id, j, key: ys.length ? ys.reduce((a, b) => a + b, 0) / ys.length : slot(j) };
    });
    keyed.sort((a, b) => a.key - b.key || a.j - b.j);
    keyed.forEach(({ id }, j) => {
      centre.set(id, slot(j));
      out[id] = { x: column(i), y: snap(slot(j) - NODE_H / 2) };
    });
  });
  out[MERGE_NODE] = { x: column(depth + 1), y: snap(middle - NODE_H / 2) };
  return out;
}

/** Where every node is: its place in `layout`, else where nothing it holds
 * overlaps — the whole auto layout when `layout` places none of them. */
export function resolvedLayout(t: TeamTemplate): Record<string, NodePosition> {
  const keys = nodeKeys(t);
  const stored: Fields = isObject(t.layout) ? t.layout : {};
  const placed = new Map<string, NodePosition>();
  for (const k of keys) {
    const p = position(stored[k]);
    if (p) placed.set(k, p);
  }
  if (placed.size === keys.length) return Object.fromEntries(placed);
  const auto = autoLayout(t);
  if (!placed.size) return Object.fromEntries(keys.map((k) => [k, auto[k]]));
  const taken = [...placed.values()].map(boxOf);
  const out: Record<string, NodePosition> = {};
  for (const k of keys) {
    let p = placed.get(k);
    if (!p) {
      p = freeSpot(taken, auto[k]);
      taken.push(boxOf(p));
    }
    out[k] = p;
  }
  return out;
}

/** `t` with every node's place in its `layout` (the places it shows now), so
 * that a change to its roles or flow moves nothing. Other keys stay. */
export function withLayout(t: TeamTemplate): TeamTemplate {
  const stored = isObject(t.layout) ? (t.layout as Fields) : undefined;
  if (stored && nodeKeys(t).every((k) => position(stored[k]))) return t;
  return { ...t, layout: { ...(stored ?? {}), ...resolvedLayout(t) } } as TeamTemplate;
}

/** *Auto-arrange*: every node at its auto-layout place. */
export function arrange(t: TeamTemplate): TeamTemplate {
  const auto = autoLayout(t);
  const stored = isObject(t.layout) ? t.layout : {};
  const placed = Object.fromEntries(nodeKeys(t).map((k) => [k, auto[k]]));
  return { ...t, layout: { ...stored, ...placed } } as TeamTemplate;
}

/** A node moved to `p` (on the grid). */
export function moveNode(t: TeamTemplate, key: string, p: NodePosition): TeamTemplate {
  const base = withLayout(t);
  return { ...base, layout: { ...base.layout, [key]: snapPos(p) } };
}

/** The free place on the grid nearest `near` for a new node, clear of every
 * box in `taken` by a grid step. */
export function freeSpot(taken: readonly Box[], near: NodePosition): NodePosition {
  const start = snapPos(near);
  const free = (p: NodePosition) =>
    taken.every(
      (b) =>
        p.x + NODE_W + GRID <= b.x ||
        b.x + b.w + GRID <= p.x ||
        p.y + NODE_H + GRID <= b.y ||
        b.y + b.h + GRID <= p.y,
    );
  if (free(start)) return start;
  const step = GRID * 2;
  for (let ring = 1; ring <= 48; ring++) {
    const spots: NodePosition[] = [];
    for (let i = -ring; i <= ring; i++) {
      for (let j = -ring; j <= ring; j++) {
        if (Math.max(Math.abs(i), Math.abs(j)) === ring) {
          spots.push({ x: start.x + i * step, y: start.y + j * step });
        }
      }
    }
    const d = (p: NodePosition) => Math.hypot(p.x - start.x, p.y - start.y);
    spots.sort((a, b) => d(a) - d(b));
    const spot = spots.find(free);
    if (spot) return spot;
  }
  const right = Math.max(start.x, ...taken.map((b) => b.x + b.w));
  return { x: snap(right + GAP_X), y: start.y };
}

/** The curve from `a`'s right edge into `b`'s left edge. */
export function edgePath(a: Box, b: Box): EdgeGeometry {
  return curve(a.x + a.w, a.y + a.h / 2, b.x - 2, b.y + b.h / 2);
}

/** The curve from `a`'s right edge to a point (a handoff being drawn). */
export function linkPath(a: Box, p: NodePosition): EdgeGeometry {
  return curve(a.x + a.w, a.y + a.h / 2, p.x, p.y);
}

const r1 = (v: number) => Math.round(v * 10) / 10;

/** How far a curve's handles reach out of its ends. */
function reach(x1: number, y1: number, x2: number, y2: number): number {
  const span = x2 - x1;
  return span >= 24
    ? Math.max(40, span / 2)
    : Math.min(240, 80 + Math.abs(span) / 2 + Math.abs(y2 - y1) / 4);
}

function arrowHead(tipX: number, y: number): string {
  const x = tipX - ARROW;
  return `${r1(tipX)},${r1(y)} ${r1(x)},${r1(y - 5)} ${r1(x)},${r1(y + 5)}`;
}

function curve(x1: number, y1: number, tipX: number, y2: number): EdgeGeometry {
  const x2 = tipX - ARROW;
  const dx = reach(x1, y1, x2, y2);
  return {
    d: `M${r1(x1)},${r1(y1)} C${r1(x1 + dx)},${r1(y1)} ${r1(x2 - dx)},${r1(y2)} ${r1(x2)},${r1(y2)}`,
    head: arrowHead(tipX, y2),
    mid: { x: r1((x1 + x2) / 2), y: r1((y1 + y2) / 2) },
    room: Math.max(56, Math.min(160, x2 - x1 - 10)),
  };
}

/**
 * The edge from `a` into `b`, around the nodes in its way: when the plain
 * curve would run behind one of `others`, it leaves `a`, runs above (or
 * below) every node between them, and drops into `b` — so a long edge (the
 * lead assigning tasks to a role three layers on, who merges) never reads as
 * passing through another role. An edge back to a node on the left (work
 * sent back) loops out of `a`, runs under (or over) both, and comes into
 * `b` from its left.
 */
export function routeEdge(
  a: Box,
  b: Box,
  others: readonly Box[],
  side: 'above' | 'below',
  clearance = 24,
): EdgeGeometry {
  const plain = edgePath(a, b);
  const x1 = a.x + a.w;
  const y1 = a.y + a.h / 2;
  const tipX = b.x - 2;
  const y2 = b.y + b.h / 2;
  const x2 = tipX - ARROW;
  const g = 22;
  const level = (boxes: readonly Box[]) =>
    side === 'above'
      ? Math.min(...boxes.map((o) => o.y)) - clearance
      : Math.max(...boxes.map((o) => o.y + o.h)) + clearance;
  if (x2 < x1) {
    const lo = x2 - 2 * g;
    const hi = x1 + 2 * g;
    const yd = level([a, b, ...others.filter((o) => o.x + o.w > lo && o.x < hi)]);
    return {
      d:
        `M${r1(x1)},${r1(y1)} C${r1(hi)},${r1(y1)} ${r1(hi)},${r1(yd)} ${r1(x1)},${r1(yd)}` +
        ` L${r1(x2)},${r1(yd)} C${r1(lo)},${r1(yd)} ${r1(lo)},${r1(y2)} ${r1(x2)},${r1(y2)}`,
      head: arrowHead(tipX, y2),
      mid: { x: r1((x1 + x2) / 2), y: r1(yd) },
      room: Math.max(56, Math.min(160, x1 - x2 - 16)),
    };
  }
  if (x2 - x1 < 5 * g) return plain;
  const between = others.filter((o) => o !== a && o !== b && o.x + o.w > x1 && o.x < x2);
  const dx = reach(x1, y1, x2, y2);
  const at = (t: number) => {
    const u = 1 - t;
    return {
      x: u * u * u * x1 + 3 * u * u * t * (x1 + dx) + 3 * u * t * t * (x2 - dx) + t * t * t * x2,
      y: u * u * u * y1 + 3 * u * u * t * y1 + 3 * u * t * t * y2 + t * t * t * y2,
    };
  };
  const blocked = between.some((o) =>
    Array.from({ length: 15 }, (_, i) => at((i + 1) / 16)).some(
      (p) => p.x > o.x - 2 && p.x < o.x + o.w + 2 && p.y > o.y - 2 && p.y < o.y + o.h + 2,
    ),
  );
  if (!blocked) return plain;
  const yd = level(between);
  const clear =
    side === 'above'
      ? Math.min(yd, y1 - clearance, y2 - clearance)
      : Math.max(yd, y1 + clearance, y2 + clearance);
  return {
    d:
      `M${r1(x1)},${r1(y1)} C${r1(x1 + g)},${r1(y1)} ${r1(x1 + g)},${r1(clear)} ${r1(x1 + 2 * g)},${r1(clear)}` +
      ` L${r1(x2 - 2 * g)},${r1(clear)} C${r1(x2 - g)},${r1(clear)} ${r1(x2 - g)},${r1(y2)} ${r1(x2)},${r1(y2)}`,
    head: arrowHead(tipX, y2),
    mid: { x: r1((x1 + x2) / 2), y: r1(clear) },
    room: Math.max(56, Math.min(160, x2 - x1 - 4 * g - 8)),
  };
}

// ───────────────────────────────────────────────────────────── roles

/** The role a new one starts as. */
export function newRole(id: string, title: string): RoleDef {
  return { id, title, model: 'sonnet', permissionMode: 'default' };
}

/** `base`, or `base-2`, `base-3`… — the first no role has (at most 32). */
export function uniqueRoleId(t: TeamTemplate, base: string): string {
  const taken = new Set(rolesOf(t).map((r) => r.id));
  const clean = base.slice(0, ROLE_ID_MAX).replace(/-+$/, '') || 'role';
  if (!taken.has(clean)) return clean;
  for (let n = 2; ; n++) {
    const suffix = `-${n}`;
    const id = clean.slice(0, ROLE_ID_MAX - suffix.length).replace(/-+$/, '') + suffix;
    if (!taken.has(id)) return id;
  }
}

/** A new role's id, `role-<n>` from the count of roles up, and its `n`. */
export function newRoleId(t: TeamTemplate): { id: string; n: number } {
  const taken = new Set(rolesOf(t).map((r) => r.id));
  let n = rolesOf(t).length + 1;
  while (taken.has(`role-${n}`)) n++;
  return { id: `role-${n}`, n };
}

/** Why `id` can't name a role (`current` is its own id), or ''. */
export function roleIdProblem(t: TeamTemplate, id: string, current?: string): string {
  if (!isRoleId(id)) {
    return tr('Use lower case letters, digits and - (at most {n}).', { n: ROLE_ID_MAX });
  }
  if (id !== current && rolesOf(t).some((r) => r.id === id)) {
    return tr('Another role is called {id}.', { id });
  }
  return '';
}

/** The roles as they are, whatever each is (JSON may hold anything). */
function rolesList(t: TeamTemplate): RoleDef[] {
  return Array.isArray(t.roles) ? t.roles : [];
}

/** `role` added at `at`. */
export function addRole(t: TeamTemplate, role: RoleDef, at: NodePosition): TeamTemplate {
  const base = withLayout(t);
  return {
    ...base,
    roles: [...rolesList(t), role] as RoleDef[],
    layout: { ...base.layout, [role.id]: snapPos(at) },
  };
}

/** A copy of role `id` (never the lead) at `at`, with the new id. */
export function duplicateRole(
  t: TeamTemplate,
  id: string,
  title: string,
  at: NodePosition,
): { template: TeamTemplate; id: string } | null {
  const src = roleOf(t, id);
  if (!src) return null;
  const copyId = uniqueRoleId(t, `${id}-copy`);
  const copy = without({ ...(structuredClone(src) as RoleDef), id: copyId, title }, 'lead');
  return { template: addRole(t, copy, at), id: copyId };
}

/** Whether role `id` may be deleted: any but the only lead. */
export function canRemove(t: TeamTemplate, id: string): boolean {
  const r = roleOf(t, id);
  if (!r) return false;
  return !(r.lead === true && rolesOf(t).filter((x) => x.lead === true).length === 1);
}

/** Role `id` gone, with its handoffs, its place, and `mergeBy` when it named
 * it (the lead merges then). */
export function removeRole(t: TeamTemplate, id: string): TeamTemplate {
  const next: TeamTemplate = {
    ...t,
    roles: rolesList(t).filter((r) => !(isObject(r) && r.id === id)) as RoleDef[],
  };
  if (Array.isArray(t.flow)) {
    next.flow = t.flow.filter((h) => !(isObject(h) && (h.from === id || h.to === id)));
  }
  if (isObject(t.integration) && t.integration.mergeBy === id) {
    next.integration = without(t.integration as Integration, 'mergeBy');
  }
  if (isObject(t.layout) && id in t.layout) next.layout = without(t.layout, id);
  return next;
}

/** Role `id` the lead, and no other: a lead runs here, so its `machine` goes. */
export function makeLead(t: TeamTemplate, id: string): TeamTemplate {
  if (!roleOf(t, id)) return t;
  const base = withLayout(t);
  return {
    ...base,
    roles: rolesList(t).map((r) => {
      if (!isObject(r)) return r;
      if (r.id === id) return without({ ...r, lead: true }, 'machine');
      return 'lead' in r ? without(r, 'lead') : r;
    }) as RoleDef[],
  };
}

/** Role `from` renamed `to`, and every handoff, `mergeBy` and the layout
 * with it. Unchanged when `to` is taken or `from` missing. */
export function renameRole(t: TeamTemplate, from: string, to: string): TeamTemplate {
  if (from === to || !roleOf(t, from) || rolesOf(t).some((r) => r.id === to)) return t;
  const next: TeamTemplate = {
    ...t,
    roles: rolesList(t).map((r) =>
      isObject(r) && r.id === from ? { ...r, id: to } : r,
    ) as RoleDef[],
  };
  if (Array.isArray(t.flow)) {
    next.flow = t.flow.map((h) => {
      if (!isObject(h)) return h;
      let n = h;
      if (h.from === from) n = { ...n, from: to };
      if (h.to === from) n = { ...n, to };
      return n;
    }) as Handoff[];
  }
  if (isObject(t.integration) && t.integration.mergeBy === from) {
    next.integration = { ...(t.integration as Integration), mergeBy: to };
  }
  if (isObject(t.layout) && from in t.layout) {
    next.layout = Object.fromEntries(
      Object.entries(t.layout).map(([k, v]) => [k === from ? to : k, v]),
    ) as Record<string, NodePosition>;
  }
  return next;
}

/** Role `id` with `patch` (a field set to `undefined` is removed). */
export function updateRole(t: TeamTemplate, id: string, patch: Partial<RoleDef>): TeamTemplate {
  let found = false;
  const roles = rolesList(t).map((r) => {
    if (found || !isObject(r) || r.id !== id) return r;
    found = true;
    return withFields(r, patch);
  });
  return found ? { ...t, roles: roles as RoleDef[] } : t;
}

/** The integration with `patch` (made when there was none). */
export function updateIntegration(t: TeamTemplate, patch: Partial<Integration>): TeamTemplate {
  if (!isObject(t.integration) && Object.values(patch).every((v) => v === undefined)) return t;
  return { ...t, integration: withFields(integrationOf(t), patch) };
}

/** The template's own fields with `patch`. */
export function updateTemplate(t: TeamTemplate, patch: Partial<TeamTemplate>): TeamTemplate {
  return withFields(t, patch);
}

// ───────────────────────────────────────────────────────────── flow

export function hasHandoff(t: TeamTemplate, from: string, to: string): boolean {
  return flowOf(t).some((h) => h.from === from && h.to === to);
}

/** Whether role `from` can hand its work to `to` (a role, or the merge node:
 * it merges then). */
export function canConnect(t: TeamTemplate, from: string, to: string): boolean {
  if (!roleOf(t, from)) return false;
  if (to === MERGE_NODE) return true;
  return from !== to && !!roleOf(t, to) && !hasHandoff(t, from, to);
}

/** `from` → `to` added to the flow; to the merge node, `from` merges. */
export function connect(t: TeamTemplate, from: string, to: string): TeamTemplate {
  if (to === MERGE_NODE) return roleOf(t, from) ? setMergeBy(t, from) : t;
  if (!canConnect(t, from, to)) return t;
  const base = withLayout(t);
  const flow = Array.isArray(t.flow) ? t.flow : [];
  return { ...base, flow: [...flow, { from, to }] };
}

/** Every `from` → `to` entry gone from the flow. */
export function disconnect(t: TeamTemplate, from: string, to: string): TeamTemplate {
  if (!Array.isArray(t.flow)) return t;
  const flow = t.flow.filter((h) => !(isObject(h) && h.from === from && h.to === to));
  if (flow.length === t.flow.length) return t;
  return { ...withLayout(t), flow };
}

/** The note on `from` → `to` (removed when blank). */
export function setHandoffNote(
  t: TeamTemplate,
  from: string,
  to: string,
  note: string,
): TeamTemplate {
  if (!Array.isArray(t.flow)) return t;
  return {
    ...t,
    flow: t.flow.map((h) =>
      isObject(h) && h.from === from && h.to === to
        ? withFields(h, { note: note.trim() ? note : undefined })
        : h,
    ) as Handoff[],
  };
}

/** Who merges: role `id`, or the lead when `undefined`. */
export function setMergeBy(t: TeamTemplate, id: string | undefined): TeamTemplate {
  return updateIntegration(t, { mergeBy: id });
}

/** "Planner → Writer → QA": the flow as chains of role titles, joined by
 * " · " when it branches. Empty without a flow. */
export function flowSummary(t: TeamTemplate): string {
  const edges = drawnFlow(t);
  if (!edges.length) return '';
  const used = new Set<Handoff>();
  const into = new Set(edges.map((h) => h.to));
  const order = rolesOf(t).map((r) => r.id);
  const starts = [...order.filter((id) => !into.has(id)), ...order];
  const chains: string[][] = [];
  for (const start of starts) {
    for (;;) {
      const first = edges.find((h) => h.from === start && !used.has(h));
      if (!first) break;
      const chain = [start];
      let edge: Handoff | undefined = first;
      while (edge) {
        used.add(edge);
        chain.push(edge.to);
        const at: string = edge.to;
        edge =
          chain.indexOf(at) < chain.length - 1
            ? undefined
            : edges.find((h) => h.from === at && !used.has(h));
      }
      chains.push(chain);
    }
  }
  return chains.map((c) => c.map((id) => titleOf(t, id)).join(' → ')).join(' · ');
}

// ───────────────────────────────────────────────────────────── problems

/** What the editor sees at once, before the server's check: the template's
 * id and name, one lead, role ids, titles, the lead's computer, the flow's
 * ends and notes, and whom `mergeBy` names. */
export function localIssues(t: TeamTemplate): TemplateIssue[] {
  const out: TemplateIssue[] = [];
  const add = (path: string, message: string) => out.push({ path, message });
  if (typeof t.id !== 'string' || !isTemplateId(t.id)) {
    add('id', tr('Use lower case letters, digits and - (at most {n}).', { n: 64 }));
  }
  if (typeof t.name !== 'string' || !t.name.trim()) add('name', tr('Give the template a name.'));
  const all = rolesList(t);
  const leads = all.filter((r) => isObject(r) && r['lead'] === true).length;
  if (!all.length) add('roles', tr('A team needs at least one role.'));
  else if (!leads) add('roles', tr('Make one role the lead.'));
  else if (leads > 1) add('roles', tr('Only one role can be the lead.'));
  const seen = new Set<string>();
  all.forEach((r, i) => {
    if (!isObject(r)) return;
    const id = typeof r['id'] === 'string' ? r['id'] : '';
    if (!isRoleId(id)) {
      add(
        `roles[${i}].id`,
        tr('Use lower case letters, digits and - (at most {n}).', { n: ROLE_ID_MAX }),
      );
    } else if (seen.has(id)) {
      add(`roles[${i}].id`, tr('Two roles are called {id}.', { id }));
    }
    seen.add(id);
    if (typeof r['title'] !== 'string' || !r['title'].trim()) {
      add(`roles[${i}].title`, tr('Give the role a title.'));
    }
    if (r['lead'] === true && typeof r['machine'] === 'string' && r['machine'].trim()) {
      add(`roles[${i}].machine`, tr('The lead runs on the computer that runs the team.'));
    }
  });
  const pairs = new Set<string>();
  if (Array.isArray(t.flow)) {
    t.flow.forEach((h, i) => {
      if (!isObject(h)) {
        add(`flow[${i}]`, tr('A handoff names the role it comes from and the one it goes to.'));
        return;
      }
      const from = typeof h['from'] === 'string' ? h['from'] : '';
      const to = typeof h['to'] === 'string' ? h['to'] : '';
      if (!seen.has(from)) add(`flow[${i}].from`, tr('No role is called "{id}".', { id: from }));
      if (!seen.has(to)) add(`flow[${i}].to`, tr('No role is called "{id}".', { id: to }));
      if (from && from === to) add(`flow[${i}]`, tr('A role cannot hand its work to itself.'));
      const key = `${from}\u0000${to}`;
      if (pairs.has(key)) add(`flow[${i}]`, tr('This handoff is there twice.'));
      pairs.add(key);
      const note = h['note'];
      if (note !== undefined && (typeof note !== 'string' || note.length > HANDOFF_NOTE_MAX)) {
        add(`flow[${i}].note`, tr('At most {n} characters.', { n: HANDOFF_NOTE_MAX }));
      }
    });
  }
  const m = integrationOf(t).mergeBy;
  if (m !== undefined && !seen.has(m)) {
    add('integration.mergeBy', tr('No role is called "{id}".', { id: String(m) }));
  }
  return out;
}

/** What a problem at `path` (`roles[2].paths[0]`, `flow[1].to`,
 * `integration.mergeBy`) is about, in `t` as it was checked. */
export function issueTarget(t: TeamTemplate, path: string): IssueTarget {
  const role = /^roles\[(\d+)\](?:\.(\w+))?/.exec(path);
  if (role) {
    const r = rolesList(t)[Number(role[1])];
    return isObject(r) && typeof r['id'] === 'string'
      ? { kind: 'role', id: r['id'], ...(role[2] ? { field: role[2] } : {}) }
      : { kind: 'template', field: 'roles' };
  }
  const step = /^flow\[(\d+)\](?:\.(\w+))?/.exec(path);
  if (step) {
    const h = Array.isArray(t.flow) ? t.flow[Number(step[1])] : undefined;
    return isObject(h) && typeof h['from'] === 'string' && typeof h['to'] === 'string'
      ? { kind: 'handoff', from: h['from'], to: h['to'], ...(step[2] ? { field: step[2] } : {}) }
      : { kind: 'template', field: 'flow' };
  }
  if (path === 'integration' || path.startsWith('integration.')) {
    const field = path.slice('integration.'.length);
    return field && path !== 'integration' ? { kind: 'merge', field } : { kind: 'merge' };
  }
  if (path === 'idleStopMinutes') return { kind: 'merge', field: path };
  const field = path.split(/[.[]/)[0];
  return field ? { kind: 'template', field } : { kind: 'template' };
}

/** The selection a problem leads to. */
export function selectionOf(target: IssueTarget): Selection {
  switch (target.kind) {
    case 'role':
      return { kind: 'role', id: target.id };
    case 'handoff':
      return { kind: 'handoff', from: target.from, to: target.to };
    case 'merge':
      return { kind: 'merge' };
    default:
      return null;
  }
}
