// Calling into a tab: inject content.js (isolated world) on demand, the page
// hook (MAIN world) for dialogs and open requests, and map Chrome's scripting
// errors to protocol codes.

import type { CallContext, CallResponse } from '../content/index.ts';
import { installPageHook } from '../content/hook.ts';
import { CommandError, isErrorShape, messageOf, permissionNeeded } from '../shared/errors.ts';
import { hostPattern, originOf } from '../shared/site.ts';
import { hasHostAccess } from './env.ts';
import type { Store } from './state.ts';

declare const __MC_CONTENT_KEY__: string;
const CONTENT_KEY = __MC_CONTENT_KEY__;

interface ContentApi {
  call(method: string, params: Record<string, unknown>, ctx: CallContext): Promise<CallResponse>;
}

/** Runs in the page (isolated world); must be self-contained. */
function contentCall(
  key: string,
  method: string,
  params: Record<string, unknown>,
  ctx: CallContext,
): unknown {
  const api = (globalThis as unknown as Record<string, ContentApi | undefined>)[key];
  if (!api) return { missing: true };
  return api.call(method, params, ctx);
}

export function mapScriptingError(e: unknown, url: string | undefined): CommandError {
  if (e instanceof CommandError) return e;
  const msg = messageOf(e);
  if (/No tab with id|tab was closed|Tabs cannot be edited/i.test(msg)) {
    return new CommandError('no_tab', 'the tab is gone');
  }
  if (/chrome-error:|error page/i.test(msg)) {
    return new CommandError(
      'navigation',
      'the page failed to load (the browser shows an error page)',
    );
  }
  if (
    /Frame with ID 0 (was removed|is showing)|frame was removed|document was unloaded|context invalidated|navigated/i.test(
      msg,
    )
  ) {
    return new CommandError(
      'navigation',
      'the page navigated during the action: take a new snapshot',
    );
  }
  if (
    /Missing host permission|Cannot access contents of (url|the page)|must request permission/i.test(
      msg,
    )
  ) {
    const pattern = hostPattern(url);
    if (pattern) {
      return permissionNeeded(
        pattern,
        `the user has not let Mother Claude into ${originOf(url) ?? pattern}: grant it in the side panel`,
      );
    }
  }
  if (
    /Cannot access a chrome|chrome:\/\/|extensions gallery|cannot be scripted|Cannot access a about/i.test(
      msg,
    )
  ) {
    return new CommandError('unsupported', 'the browser does not let extensions into this page');
  }
  return new CommandError('internal', msg);
}

export class PageBridge {
  private readonly store: Store;

  constructor(store: Store) {
    this.store = store;
  }

  /** Throws `permission_needed` / `unsupported` when the tab's page cannot be scripted. */
  async assertAccess(tabId: number, url: string | undefined): Promise<void> {
    if (!url || !originOf(url)) {
      if (url === 'about:blank') return;
      throw new CommandError(
        'unsupported',
        `the browser does not let extensions into ${url ?? 'this page'}`,
      );
    }
    if (await hasHostAccess(url)) return;
    const shared = this.store.session.shared;
    if (shared && shared.tabId === tabId && shared.origin === originOf(url)) return;
    const pattern = hostPattern(url) ?? url;
    throw permissionNeeded(
      pattern,
      `the user has not let Mother Claude into ${originOf(url) ?? url}: grant it in the side panel`,
    );
  }

  private async exec(
    tabId: number,
    method: string,
    params: Record<string, unknown>,
    ctx: CallContext,
  ): Promise<unknown> {
    const results = await chrome.scripting.executeScript({
      target: { tabId, frameIds: [0] },
      func: contentCall,
      args: [CONTENT_KEY, method, params, ctx],
    });
    return results[0]?.result;
  }

  /** Make sure content.js and the page hook are in the tab's current document. */
  async inject(tabId: number): Promise<void> {
    await chrome.scripting.executeScript({
      target: { tabId, frameIds: [0] },
      files: ['content.js'],
      injectImmediately: true,
    });
    const nonce = this.store.session.nonce;
    await this.exec(
      tabId,
      'init',
      { nonce },
      { nextRef: 1, nextFrame: 1, deadline: Date.now() + 5000 },
    );
    await this.installHook(tabId);
  }

  async installHook(tabId: number): Promise<void> {
    try {
      await chrome.scripting.executeScript({
        target: { tabId, frameIds: [0] },
        world: 'MAIN',
        injectImmediately: true,
        func: installPageHook,
        args: [this.store.session.nonce],
      });
    } catch {
      // a page that refuses MAIN-world scripts still works, without the hook
    }
  }

  /** Stop whatever the tab's page script is waiting for (Stop, a closed lease). Best effort. */
  async cancel(tabId: number): Promise<void> {
    try {
      await this.exec(
        tabId,
        'cancel',
        {},
        { nextRef: 1, nextFrame: 1, deadline: Date.now() + 2000 },
      );
    } catch {
      // no page script there: nothing to stop
    }
  }

  /** Call a content-script method; refs continue the tab's numbering. */
  async call(
    tabId: number,
    url: string | undefined,
    method: string,
    params: Record<string, unknown>,
    opts: { deadline: number; allowSecret?: boolean; refuseSecret?: boolean },
  ): Promise<Record<string, unknown>> {
    await this.assertAccess(tabId, url);
    const counters = this.store.counters(tabId);
    const ctx: CallContext = {
      nextRef: counters.nextRef,
      nextFrame: counters.nextFrame,
      deadline: opts.deadline,
      ...(opts.allowSecret ? { allowSecret: true } : {}),
      ...(opts.refuseSecret ? { refuseSecret: true } : {}),
    };
    let raw: unknown;
    try {
      raw = await this.exec(tabId, method, params, ctx);
      if (isMissing(raw)) {
        await this.inject(tabId);
        raw = await this.exec(tabId, method, params, ctx);
      }
    } catch (e) {
      throw mapScriptingError(e, url);
    }
    if (isMissing(raw)) throw new CommandError('internal', 'the page script did not load');
    const res = raw as CallResponse;
    if (typeof res !== 'object' || res === null || !('ok' in res)) {
      // The document went away while the call ran: the page navigated.
      throw new CommandError(
        'navigation',
        'the page navigated during the action: take a new snapshot',
      );
    }
    this.store.setCounters(tabId, { nextRef: res.nextRef, nextFrame: res.nextFrame });
    if (!res.ok) {
      const err = res.error;
      if (isErrorShape(err)) throw new CommandError(err.code, err.message, err.data);
      throw new CommandError('internal', 'the page script failed');
    }
    return (res.data ?? {}) as Record<string, unknown>;
  }
}

function isMissing(v: unknown): boolean {
  return typeof v === 'object' && v !== null && (v as { missing?: unknown }).missing === true;
}
