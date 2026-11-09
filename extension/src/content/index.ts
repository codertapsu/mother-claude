// The content script: injected on demand into a lease tab's top frame
// (isolated world), it answers the service worker's calls. Same-origin
// iframes are reached from here; cross-origin ones are reported, not read.

import { CommandError, type ErrorShape } from '../shared/errors.ts';
import { parseKey } from '../shared/keys.ts';
import {
  type Button,
  click,
  drag,
  fillField,
  highlight,
  hover,
  isOurs,
  press,
  pressEnterOn,
  scroll,
  scrollIntoViewIfNeeded,
  selectOptions,
  setChecked,
  ariaChecked,
  toTopPoint,
  trustedPoint,
  type as typeText,
  upload,
  type UploadFile,
} from './actions.ts';
import { ChangeTracker, deltaSnapshot } from './delta.ts';
import { deepActiveElement, deepQueryAll } from './dom.ts';
import { find } from './find.ts';
import { read, type ReadMode } from './read.ts';
import { isSecretField } from './redact.ts';
import { RefRegistry } from './refs.ts';
import { SelectorNotFound, isDisabled, snapshot } from './snapshot.ts';
import {
  areaOf,
  clearStorage,
  describeDatabase,
  listDatabases,
  readStorage,
  readStore,
  removeStorage,
  setStorage,
} from './storage.ts';
import { waitForNetworkIdle, waitForSelector, waitForText } from './wait.ts';

declare const __MC_CONTENT_KEY__: string;

export interface CallContext {
  nextRef: number;
  nextFrame: number;
  /** Epoch ms by which the call must have answered. */
  deadline: number;
  /** The desktop says the user granted typing into a secret field. */
  allowSecret?: boolean;
  /** The desktop asks for `secret_field` when the target is a secret field. */
  refuseSecret?: boolean;
}

export type CallResponse =
  | { ok: true; data: unknown; nextRef: number; nextFrame: number }
  | { ok: false; error: ErrorShape; nextRef: number; nextFrame: number };

type Params = Record<string, unknown>;
type Method = (p: Params, ctx: CallContext) => unknown;

const refs = new RefRegistry();
let interactiveOnlyDeltas = false;
let nonce: string | null = null;
let pending: ChangeTracker | null = null;
/** Aborted by `cancel` (Stop, a closed lease): ends the waits in progress. */
let cancellation = new AbortController();

const str = (v: unknown): string | undefined => (typeof v === 'string' ? v : undefined);
const num = (v: unknown, fallback: number): number =>
  typeof v === 'number' && Number.isFinite(v) ? v : fallback;

function target(p: Params): { el: Element; what: string } {
  const ref = str(p['ref']);
  if (ref) {
    const r = refs.resolve(ref);
    if (!r.ok) throw new CommandError(r.code, r.message);
    return { el: r.el, what: ref.trim() };
  }
  const selector = str(p['selector']);
  if (selector) {
    let el: Element | undefined;
    try {
      el = deepQueryAll(document, selector, 1)[0];
    } catch {
      throw new CommandError('not_found', `invalid selector: ${selector}`);
    }
    if (!el) throw new CommandError('not_found', `nothing matches ${selector}`);
    return { el, what: selector };
  }
  throw new CommandError('not_found', 'give a ref from a snapshot, or a selector');
}

/**
 * Typing into a secret field: refused with `secret_field` when the command
 * says `refuseSecret`, with `unsupported` unless it says the user granted it
 * (`allowSecret`), allowed otherwise.
 */
function secretRefusal(what: string, ctx: CallContext): CommandError | null {
  if (ctx.refuseSecret) {
    return new CommandError(
      'secret_field',
      `${what} is a password or other secret field: its value comes from the user, never from a session`,
    );
  }
  if (ctx.allowSecret) return null;
  return new CommandError(
    'unsupported',
    `${what} is a password or other secret field: the extension types into it only when the user granted it on the desktop`,
  );
}

function assertSecretAllowed(el: Element, what: string, ctx: CallContext): void {
  if (!isSecretField(el)) return;
  const refusal = secretRefusal(what, ctx);
  if (refusal) throw refusal;
}

function page(): { url: string; title: string } {
  return { url: location.href, title: document.title };
}

async function settled(
  tracker: ChangeTracker,
  ctx: CallContext,
  extra: Record<string, unknown> = {},
): Promise<Record<string, unknown>> {
  await tracker.settle(100, 600, ctx.deadline - 100, cancellation.signal);
  const delta = deltaSnapshot(document, refs, tracker.stop(), interactiveOnlyDeltas);
  return { ...(delta ? { snapshot: delta } : {}), ...page(), ...extra };
}

function inflightProbe(): { inflight: number; lastChange: number } | null {
  if (!nonce) return null;
  let answer: { inflight: number; lastChange: number } | null = null;
  const onAnswer = (e: Event): void => {
    try {
      const d = JSON.parse(String((e as CustomEvent).detail)) as {
        inflight: number;
        lastChange: number;
      };
      answer = d;
    } catch {
      answer = null;
    }
  };
  document.addEventListener(`mc-a-${nonce}`, onAnswer);
  document.dispatchEvent(new CustomEvent(`mc-q-${nonce}`));
  document.removeEventListener(`mc-a-${nonce}`, onAnswer);
  return answer;
}

function relayDialog(open: boolean) {
  return (e: Event): void => {
    let detail: { kind?: string; message?: string } = {};
    try {
      detail = JSON.parse(String((e as CustomEvent).detail)) as typeof detail;
    } catch {
      detail = {};
    }
    chrome.runtime
      .sendMessage({ type: 'mc.dialog', open, kind: detail.kind, message: detail.message })
      .catch(() => undefined);
  };
}

function actionable(el: Element, what: string): void {
  if (!el.isConnected) throw new CommandError('stale_ref', `${what} is gone: take a new snapshot`);
  if (isDisabled(el)) throw new CommandError('unsupported', `${what} is disabled`);
}

const methods: Record<string, Method> = {
  cancel() {
    cancellation.abort();
    cancellation = new AbortController();
    return {};
  },

  init(p) {
    const n = str(p['nonce']);
    if (n && n !== nonce) {
      nonce = n;
      document.addEventListener(`mc-dialog-${n}`, relayDialog(true));
      document.addEventListener(`mc-dialog-closed-${n}`, relayDialog(false));
    }
    return { ready: true };
  },

  ping() {
    return { ready: true, readyState: document.readyState, ...page() };
  },

  status() {
    const nav = performance.getEntriesByType('navigation')[0] as
      (PerformanceNavigationTiming & { responseStatus?: number }) | undefined;
    return {
      readyState: document.readyState,
      status: nav?.responseStatus || undefined,
      ...page(),
    };
  },

  snapshot(p, ctx) {
    const interactiveOnly = p['interactiveOnly'] === true;
    interactiveOnlyDeltas = interactiveOnly;
    const scope = p['scope'] === 'viewport' ? 'viewport' : 'page';
    const selector = str(p['selector']);
    const r = snapshot(document, refs, {
      scope,
      interactiveOnly,
      maxNodes: num(p['maxNodes'], 1500),
      deadline: ctx.deadline - 150,
      ...(selector ? { selector } : {}),
    });
    return { snapshot: r.snapshot, ...page(), truncated: r.truncated, nodes: r.nodes };
  },

  read(p) {
    const mode = (str(p['mode']) ?? 'text') as ReadMode;
    const selector = str(p['selector']);
    const r = read(document, {
      mode,
      ...(selector ? { selector } : {}),
      ...(typeof p['maxChars'] === 'number' ? { maxChars: p['maxChars'] } : {}),
    });
    return { content: r.content, ...page(), truncated: r.truncated };
  },

  find(p) {
    return {
      matches: find(document, refs, {
        ...(str(p['text']) ? { text: str(p['text']) } : {}),
        ...(str(p['role']) ? { role: str(p['role']) } : {}),
        ...(str(p['name']) ? { name: str(p['name']) } : {}),
        ...(str(p['selector']) ? { selector: str(p['selector']) } : {}),
        limit: num(p['limit'], 20),
      }),
    };
  },

  async click(p, ctx) {
    const { el, what } = target(p);
    actionable(el, what);
    scrollIntoViewIfNeeded(el);
    const tracker = new ChangeTracker(document, isOurs);
    const button = (str(p['button']) ?? 'left') as Button;
    const outcome = click(el, button === 'right' || button === 'middle' ? button : 'left', 1);
    return settled(tracker, ctx, outcome.openUrl ? { openUrl: outcome.openUrl } : {});
  },

  async dblclick(p, ctx) {
    const { el, what } = target(p);
    actionable(el, what);
    scrollIntoViewIfNeeded(el);
    const tracker = new ChangeTracker(document, isOurs);
    click(el, 'left', 2);
    return settled(tracker, ctx);
  },

  async hover(p, ctx) {
    const { el, what } = target(p);
    if (!el.isConnected)
      throw new CommandError('stale_ref', `${what} is gone: take a new snapshot`);
    scrollIntoViewIfNeeded(el);
    const tracker = new ChangeTracker(document, isOurs);
    hover(el);
    return settled(tracker, ctx);
  },

  async type(p, ctx) {
    const { el, what } = target(p);
    assertSecretAllowed(el, what, ctx);
    const text = str(p['text']) ?? '';
    scrollIntoViewIfNeeded(el);
    typeText(el, text, { clear: p['clear'] === true, what });
    if (p['submit'] === true) {
      const tracker = new ChangeTracker(document, isOurs);
      pressEnterOn(el);
      return settled(tracker, ctx);
    }
    return {};
  },

  async fill(p, ctx) {
    const fields = Array.isArray(p['fields']) ? (p['fields'] as unknown[]) : [];
    if (!fields.length) throw new CommandError('not_found', 'fill needs fields: [{ref, value}]');
    const resolved = fields.map((f, i) => {
      const field = (f ?? {}) as Params;
      const value = field['value'];
      if (typeof value !== 'string' && typeof value !== 'number' && typeof value !== 'boolean') {
        throw new CommandError('unsupported', `field ${i + 1} has no value`);
      }
      const t = target(field);
      assertSecretAllowed(t.el, t.what, ctx);
      return { ...t, value: String(value) };
    });
    for (const f of resolved) actionable(f.el, f.what);
    for (const f of resolved) fillField(f.el, f.value, f.what);
    const last = resolved[resolved.length - 1];
    if (p['submit'] === true && last) {
      const tracker = new ChangeTracker(document, isOurs);
      pressEnterOn(last.el);
      return settled(tracker, ctx, { filled: resolved.length });
    }
    return { filled: resolved.length };
  },

  select(p) {
    const { el, what } = target(p);
    const raw = p['values'] ?? p['value'];
    const values = (Array.isArray(raw) ? raw : [raw]).filter(
      (v): v is string => typeof v === 'string',
    );
    if (!values.length) throw new CommandError('not_found', 'select needs values: ["…"]');
    return { selected: selectOptions(el, values, what) };
  },

  async check(p) {
    const { el, what } = target(p);
    const wanted = p['checked'] !== false;
    setChecked(el, wanted, what);
    // ARIA widgets re-render after the click; give the page a moment.
    for (let i = 0; i < 10 && ariaChecked(el) !== wanted; i++) {
      await new Promise((r) => setTimeout(r, 30));
    }
    if (ariaChecked(el) !== null && ariaChecked(el) !== wanted) {
      throw new CommandError(
        'unsupported',
        `the page did not change ${what}: it may ignore synthetic clicks (try trusted: true)`,
      );
    }
    return {};
  },

  async press(p, ctx) {
    const key = str(p['key']);
    if (!key) throw new CommandError('not_found', 'press needs a key');
    const mods = Array.isArray(p['modifiers'])
      ? (p['modifiers'] as unknown[]).filter((m): m is string => typeof m === 'string')
      : [];
    const spec = parseKey(key, mods, /Mac/i.test(navigator.platform));
    if (!spec) throw new CommandError('unsupported', `unknown key "${key}"`);
    const focused = deepActiveElement(document);
    if (focused && spec.def.text && spec.def.key !== 'Enter') {
      assertSecretAllowed(focused, 'the focused field', ctx);
    }
    const tracker = new ChangeTracker(document, isOurs);
    press(document, spec);
    return settled(tracker, ctx);
  },

  scroll(p) {
    const ref = str(p['ref']);
    const el = ref || str(p['selector']) ? target(p).el : null;
    const amount = Math.max(1, Math.min(100_000, num(p['amount'], 600)));
    return scroll(document, el, str(p['direction']), amount);
  },

  async wait(p, ctx) {
    const kind = str(p['for']);
    const value = str(p['value']) ?? (typeof p['value'] === 'number' ? String(p['value']) : '');
    const own = typeof p['timeoutMs'] === 'number' ? Date.now() + p['timeoutMs'] : ctx.deadline;
    const deadline = Math.min(own, ctx.deadline - 50);
    let waited: number;
    if (kind === 'selector') {
      if (!value) throw new CommandError('not_found', 'wait for a selector needs a value');
      waited = await waitForSelector(document, value, deadline, cancellation.signal);
    } else if (kind === 'text') {
      waited = await waitForText(document, value, deadline, cancellation.signal);
    } else if (kind === 'networkidle') {
      waited = await waitForNetworkIdle(
        document,
        deadline,
        inflightProbe,
        500,
        cancellation.signal,
      );
    } else {
      throw new CommandError('unsupported', `cannot wait for "${kind ?? ''}" in the page`);
    }
    return { waitedMs: waited };
  },

  highlight(p) {
    const { el } = target(p);
    highlight(el, num(p['ms'], 800));
    return {};
  },

  upload(p) {
    const { el, what } = target(p);
    const files = Array.isArray(p['files']) ? (p['files'] as UploadFile[]) : [];
    const n = upload(
      el,
      files.filter((f) => f && typeof f.data === 'string'),
      what,
    );
    return { files: n };
  },

  /** Where a debugger click lands; starts collecting changes for `trustedDone`. */
  trustedPoint(p) {
    const { el, what } = target(p);
    if (p['hover'] !== true) actionable(el, what);
    const point = trustedPoint(el);
    pending?.stop();
    pending = new ChangeTracker(document, isOurs);
    return point;
  },

  async trustedDone(_p, ctx) {
    const tracker = pending ?? new ChangeTracker(document, isOurs);
    pending = null;
    return settled(tracker, ctx);
  },

  /** Before trusted typing: focus the field and say whether it is a secret. */
  focusForKeys(p, ctx) {
    if (str(p['ref']) || str(p['selector'])) {
      const { el, what } = target(p);
      assertSecretAllowed(el, what, ctx);
      (el as HTMLElement).focus?.();
      if (p['clear'] === true && 'select' in el && typeof el.select === 'function') {
        (el as HTMLInputElement).select();
      }
    } else {
      const focused = deepActiveElement(document);
      if (focused && p['typing'] === true) assertSecretAllowed(focused, 'the focused field', ctx);
    }
    pending?.stop();
    pending = new ChangeTracker(document, isOurs);
    return {};
  },

  /** The element's box for a clipped screenshot, in top-viewport CSS px. */
  rect(p) {
    const { el } = target(p);
    scrollIntoViewIfNeeded(el);
    const r = el.getBoundingClientRect();
    const topLeft = toTopPoint(el, { x: r.left, y: r.top });
    return {
      x: topLeft.x,
      y: topLeft.y,
      width: r.width,
      height: r.height,
      scrollX: window.scrollX,
      scrollY: window.scrollY,
      dpr: window.devicePixelRatio || 1,
    };
  },

  /** Drag `ref` onto `toRef` with the events a user's drag fires. */
  async drag(p, ctx) {
    const { el: from, what } = target(p);
    const { el: to, what: toWhat } = target({ ref: p['toRef'], selector: p['toSelector'] });
    actionable(from, what);
    if (!to.isConnected)
      throw new CommandError('stale_ref', `${toWhat} is gone: take a new snapshot`);
    pending?.stop();
    const tracker = new ChangeTracker(document, isOurs);
    const outcome = await drag(from, to);
    return settled(tracker, ctx, { ...outcome });
  },

  /** Where an element is, for the trusted drag's drop (no tracking). */
  pointOf(p) {
    const { el } = target(p);
    return trustedPoint(el);
  },

  async storage(p) {
    const values = p['values'] === true;
    if (p['area'] === 'indexeddb') {
      const db = str(p['db']);
      const store = str(p['store']);
      if (db && store) return readStore(db, store, num(p['limit'], 20), values);
      if (db) return describeDatabase(db);
      return { origin: location.origin, databases: await listDatabases() };
    }
    return readStorage(areaOf(p['area']), str(p['key']), values);
  },

  'storage.set'(p) {
    const key = str(p['key']);
    const value = str(p['value']);
    if (key === undefined || value === undefined) {
      throw new CommandError('not_found', 'storage.set needs a key and a value');
    }
    return setStorage(areaOf(p['area']), key, value);
  },

  'storage.remove'(p) {
    const key = str(p['key']);
    if (key === undefined) throw new CommandError('not_found', 'storage.remove needs a key');
    return removeStorage(areaOf(p['area']), key);
  },

  'storage.clear'(p) {
    return clearStorage(areaOf(p['area']));
  },

  viewport() {
    return {
      width: window.innerWidth,
      height: window.innerHeight,
      scrollX: window.scrollX,
      scrollY: window.scrollY,
      dpr: window.devicePixelRatio || 1,
    };
  },
};

function toError(e: unknown): ErrorShape {
  if (e instanceof CommandError) return e.toWire() as ErrorShape;
  if (e instanceof SelectorNotFound) return { code: 'not_found', message: e.message };
  if (e instanceof Error && e.name === 'SyntaxError')
    return { code: 'not_found', message: e.message };
  return { code: 'internal', message: e instanceof Error ? e.message : String(e) };
}

async function call(method: string, params: Params, ctx: CallContext): Promise<CallResponse> {
  refs.continueFrom(ctx.nextRef, ctx.nextFrame);
  const fn = methods[method];
  try {
    if (!fn) throw new CommandError('unsupported', `the page script has no "${method}"`);
    const data = await fn(params ?? {}, ctx);
    return { ok: true, data, ...refs.counters };
  } catch (e) {
    return { ok: false, error: toError(e), ...refs.counters };
  }
}

const api = { version: __MC_CONTENT_KEY__, call };
const g = globalThis as unknown as Record<string, unknown>;
if (!g[__MC_CONTENT_KEY__]) g[__MC_CONTENT_KEY__] = api;
