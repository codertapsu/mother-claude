// What each command does (docs/BROWSER-PROTOCOL.md, "Commands"). The router
// has already checked the lease, the tab, the site mode and any open dialog.

import { CommandError, messageOf, permissionNeeded } from '../shared/errors.ts';
import { charKey, modifierMask, parseKey, type KeyDef, type Modifier } from '../shared/keys.ts';
import { MAX_FRAME_BYTES } from '../shared/protocol.ts';
import { abortReason, sleep } from '../shared/queue.ts';
import { modeFor, originOf } from '../shared/site.ts';
import type { DebuggerManager } from './debugger.ts';
import type { DialogInfo, Dialogs } from './dialogs.ts';
import { hasHostAccess } from './env.ts';
import type { Leases } from './leases.ts';
import type { PageBridge } from './page.ts';
import type { LeaseRecord, Store } from './state.ts';

export interface Cmd {
  action: string;
  params: Record<string, unknown>;
  rec: LeaseRecord;
  /** The tab the command runs in (null for lease-level commands). */
  tabId: number | null;
  tab: chrome.tabs.Tab | null;
  deadline: number;
  signal: AbortSignal;
}

type Data = Record<string, unknown>;
type Handler = (c: Cmd) => Promise<Data>;

const str = (v: unknown): string | undefined => (typeof v === 'string' ? v : undefined);
const num = (v: unknown): number | undefined =>
  typeof v === 'number' && Number.isFinite(v)
    ? v
    : typeof v === 'string' && v.trim() && Number.isFinite(Number(v))
      ? Number(v)
      : undefined;

/** Leave room in a result frame for everything around the image. */
const MAX_IMAGE_BASE64 = MAX_FRAME_BYTES - 64 * 1024;

const LEVELS = ['debug', 'log', 'info', 'warning', 'error'];

export function urlMatches(url: string, pattern: string): boolean {
  if (!pattern) return true;
  if (pattern.includes('*')) {
    const re = new RegExp(
      `^${pattern
        .split('*')
        .map((p) => p.replace(/[.+?^${}()|[\]\\]/g, '\\$&'))
        .join('.*')}$`,
    );
    return re.test(url);
  }
  return url.includes(pattern);
}

/** Only web pages (and about:blank) are opened for a session. */
export function checkUrl(raw: string | undefined): string {
  const url = (raw ?? '').trim();
  if (!url || url === 'about:blank') return 'about:blank';
  let u: URL;
  try {
    u = new URL(url);
  } catch {
    // "github.com/foo" → https
    try {
      u = new URL(`https://${url}`);
    } catch {
      throw new CommandError('unsupported', `"${url}" is not a URL`);
    }
  }
  if (u.protocol !== 'http:' && u.protocol !== 'https:') {
    throw new CommandError(
      'unsupported',
      `only http and https pages can be opened (not ${u.protocol})`,
    );
  }
  return u.href;
}

/** Width and height of a PNG or JPEG, from its header. */
export function imageSize(base64: string): { width: number; height: number } {
  // The first 64 KiB is plenty for a PNG IHDR or a JPEG SOF marker.
  const chunk = base64.slice(0, 87_380).replace(/=+$/, '');
  let head: string;
  try {
    head = atob(chunk.slice(0, chunk.length - (chunk.length % 4)));
  } catch {
    return { width: 0, height: 0 };
  }
  const b = (i: number): number => head.charCodeAt(i) & 0xff;
  if (b(0) === 0x89 && b(1) === 0x50) {
    return {
      width: ((b(16) << 24) | (b(17) << 16) | (b(18) << 8) | b(19)) >>> 0,
      height: ((b(20) << 24) | (b(21) << 16) | (b(22) << 8) | b(23)) >>> 0,
    };
  }
  if (b(0) === 0xff && b(1) === 0xd8) {
    let i = 2;
    while (i + 9 < head.length) {
      if (b(i) !== 0xff) {
        i++;
        continue;
      }
      const marker = b(i + 1);
      const len = (b(i + 2) << 8) | b(i + 3);
      if (
        marker >= 0xc0 &&
        marker <= 0xcf &&
        marker !== 0xc4 &&
        marker !== 0xc8 &&
        marker !== 0xcc
      ) {
        return { height: (b(i + 5) << 8) | b(i + 6), width: (b(i + 7) << 8) | b(i + 8) };
      }
      i += 2 + len;
    }
  }
  return { width: 0, height: 0 };
}

export interface CommandDeps {
  store: Store;
  leases: Leases;
  page: PageBridge;
  dbg: DebuggerManager;
  dialogs: Dialogs;
}

export class Commands {
  private readonly d: CommandDeps;
  readonly handlers: Record<string, Handler>;

  constructor(deps: CommandDeps) {
    this.d = deps;
    this.handlers = {
      'tabs.list': (c) => this.tabsList(c),
      'tabs.open': (c) => this.tabsOpen(c),
      'tabs.select': (c) => this.tabsSelect(c),
      'tabs.close': (c) => this.tabsClose(c),
      navigate: (c) => this.navigate(c),
      back: (c) => this.history(c, 'back'),
      forward: (c) => this.history(c, 'forward'),
      reload: (c) => this.history(c, 'reload'),
      snapshot: (c) => this.content(c),
      read: (c) => this.content(c),
      find: (c) => this.content(c),
      highlight: (c) => this.content(c),
      scroll: (c) => this.content(c),
      click: (c) => this.mouse(c),
      dblclick: (c) => this.mouse(c),
      hover: (c) => this.mouse(c),
      type: (c) => (c.params['trusted'] === true ? this.trustedType(c) : this.raced(c)),
      fill: (c) => this.raced(c),
      select: (c) => this.raced(c),
      check: (c) => this.raced(c),
      upload: (c) => this.raced(c),
      press: (c) => (c.params['trusted'] === true ? this.trustedPress(c) : this.raced(c)),
      wait: (c) => this.wait(c),
      screenshot: (c) => this.screenshot(c),
      eval: (c) => this.evaluate(c),
      console: (c) => this.console(c),
      dialog: (c) => this.dialog(c),
    };
  }

  private tabId(c: Cmd): number {
    if (c.tabId === null)
      throw new CommandError('no_tab', 'the session has no tab open: use tabs.open');
    return c.tabId;
  }

  private url(c: Cmd): string | undefined {
    return c.tab?.url ?? c.tab?.pendingUrl;
  }

  private call(c: Cmd, method: string, params: Data = c.params): Promise<Data> {
    return this.d.page.call(this.tabId(c), this.url(c), method, params, {
      deadline: c.deadline,
      allowSecret: c.params['allowSecret'] === true,
      refuseSecret: c.params['refuseSecret'] === true,
    });
  }

  private async pageInfo(tabId: number): Promise<{ url: string; title: string }> {
    try {
      const t = await chrome.tabs.get(tabId);
      return { url: t.url ?? t.pendingUrl ?? '', title: t.title ?? '' };
    } catch {
      return { url: '', title: '' };
    }
  }

  /**
   * Run `work`, but answer as soon as a JavaScript dialog opens in the tab:
   * the page is blocked then, and the dialog itself is the result.
   */
  private async raceDialog(tabId: number, work: Promise<Data>): Promise<Data> {
    const waiter = this.d.dialogs.next(tabId);
    work.catch(() => undefined);
    try {
      const winner = await Promise.race([
        work.then((data) => ({ data })),
        waiter.promise.then((dialog: DialogInfo) => ({ dialog })),
      ]);
      if ('dialog' in winner) {
        return {
          dialog: { kind: winner.dialog.kind, message: winner.dialog.message },
          ...(await this.pageInfo(tabId)),
        };
      }
      return winner.data;
    } finally {
      waiter.cancel();
    }
  }

  private raced(c: Cmd): Promise<Data> {
    const tabId = this.tabId(c);
    return this.raceDialog(tabId, this.navigating(c, tabId, this.call(c, c.action)));
  }

  /**
   * An action that navigated the whole page took the page script with it:
   * that is a success, and the result is the page it ended on (no delta).
   */
  private async navigating(c: Cmd, tabId: number, work: Promise<Data>): Promise<Data> {
    const nav = this.watchNavigation(tabId);
    work.catch(() => undefined);
    try {
      const winner = await Promise.race([
        work.then(
          (data) => ({ data }),
          (error: unknown) => ({ error }),
        ),
        nav.promise.then(() => ({ navigated: true as const })),
      ]);
      if ('data' in winner) return winner.data;
      if ('error' in winner) {
        const e = winner.error;
        if (!(e instanceof CommandError) || e.code !== 'navigation') throw e;
      }
      const budget = { ...c, deadline: Math.min(c.deadline, Date.now() + 10_000) };
      await this.waitForLoad(tabId, budget, 'load', false).catch(() => undefined);
      return { ...(await this.pageInfo(tabId)), navigated: true };
    } finally {
      nav.cancel();
    }
  }

  /**
   * Resolves when the tab starts loading a new document and the action's own
   * answer has not come within a second after that (the page script went with
   * the old document). A frame loading inside the page leaves the action time
   * to answer normally.
   */
  private watchNavigation(tabId: number): { promise: Promise<void>; cancel(): void } {
    let timer: ReturnType<typeof setTimeout> | null = null;
    let resolve!: () => void;
    const promise = new Promise<void>((r) => (resolve = r));
    const onUpdated = (id: number, info: chrome.tabs.OnUpdatedInfo): void => {
      if (id !== tabId || info.status !== 'loading' || timer) return;
      timer = setTimeout(resolve, 1000);
    };
    chrome.tabs.onUpdated.addListener(onUpdated);
    return {
      promise,
      cancel: () => {
        chrome.tabs.onUpdated.removeListener(onUpdated);
        if (timer) clearTimeout(timer);
      },
    };
  }

  private content(c: Cmd): Promise<Data> {
    return this.call(c, c.action);
  }

  // ----------------------------------------------------------------- tabs

  private async tabsList(c: Cmd): Promise<Data> {
    const tabs = await this.d.leases.pageTabs(c.rec);
    const active = await this.d.leases.activeTab(c.rec);
    const out = tabs.map((t) => ({
      tabId: t.id,
      url: t.url ?? t.pendingUrl ?? '',
      title: t.title ?? '',
      active: t.id === active,
      inLease: true,
    }));
    const shared = this.d.store.session.shared;
    if (shared && !out.some((o) => o.tabId === shared.tabId)) {
      try {
        const t = await chrome.tabs.get(shared.tabId);
        if (originOf(t.url) === shared.origin) {
          out.push({
            tabId: t.id,
            url: t.url ?? '',
            title: t.title ?? '',
            active: false,
            inLease: false,
          });
        } else this.d.store.session.shared = null;
      } catch {
        this.d.store.session.shared = null;
      }
    }
    return { tabs: out };
  }

  private refuseTarget(url: string): void {
    if (url === 'about:blank') return;
    const { siteModes, settings } = this.d.store.session.desktop;
    if (modeFor(url, siteModes, settings) === 'off') {
      throw new CommandError(
        'site_off',
        `${originOf(url) ?? url} is off in Mother Claude: the user can allow it in the side panel`,
      );
    }
  }

  private async tabsOpen(c: Cmd): Promise<Data> {
    const url = checkUrl(str(c.params['url']));
    this.refuseTarget(url);
    const active = c.params['active'] !== false;
    const tab = await this.d.leases.openTab(c.rec, url, active);
    if (tab.id === undefined) throw new CommandError('internal', 'the new tab has no id');
    if (url !== 'about:blank') await this.waitForLoad(tab.id, c, 'load', true);
    const info = await this.pageInfo(tab.id);
    return { tabId: tab.id, url: info.url || url, title: info.title };
  }

  private async tabsSelect(c: Cmd): Promise<Data> {
    const tabId = this.tabId(c);
    await chrome.tabs.update(tabId, { active: true });
    c.rec.activeTabId = tabId;
    this.d.store.persist();
    return {};
  }

  private async tabsClose(c: Cmd): Promise<Data> {
    const tabId = this.tabId(c);
    await this.d.leases.closeTab(c.rec, tabId);
    await this.d.dbg.detach(tabId);
    return {};
  }

  // ------------------------------------------------------------ navigation

  /**
   * Wait for the tab to load. `expectNav`: a navigation was just started, so
   * the "complete" that counts is the one after a fresh "loading" — or, for a
   * same-document navigation (which never says "loading"), a URL other than
   * `fromUrl`.
   */
  private waitForLoad(
    tabId: number,
    c: Cmd,
    until: 'load' | 'domcontentloaded' | 'networkidle',
    expectNav: boolean,
    fromUrl?: string,
  ): Promise<void> {
    return new Promise<void>((resolve, reject) => {
      let sawLoading = false;
      let done = false;
      let netError: string | null = null;
      const finish = (err?: Error): void => {
        if (done) return;
        done = true;
        chrome.tabs.onUpdated.removeListener(onUpdated);
        if (navError) chrome.webNavigation?.onErrorOccurred.removeListener(navError);
        clearInterval(poll);
        clearTimeout(timer);
        c.signal.removeEventListener('abort', onAbort);
        if (err) reject(err);
        else resolve();
      };
      const onAbort = (): void => finish(abortReason(c.signal));
      c.signal.addEventListener('abort', onAbort, { once: true });
      const onUpdated = (id: number, info: chrome.tabs.OnUpdatedInfo): void => {
        if (id !== tabId) return;
        if (info.status === 'loading') sawLoading = true;
        if (info.status === 'complete' && (sawLoading || !expectNav)) {
          if (netError)
            finish(new CommandError('navigation', `the page did not load: ${netError}`));
          else finish();
        }
      };
      chrome.tabs.onUpdated.addListener(onUpdated);
      let navError:
        ((d: chrome.webNavigation.WebNavigationFramedErrorCallbackDetails) => void) | null = null;
      if (chrome.webNavigation?.onErrorOccurred) {
        navError = (d) => {
          if (d.tabId === tabId && d.frameId === 0 && !/ERR_ABORTED/.test(d.error))
            netError = d.error;
        };
        chrome.webNavigation.onErrorOccurred.addListener(navError);
      }
      const started = Date.now();
      // Same-document navigations never say "loading"; a page already there may be complete.
      const poll = setInterval(() => {
        void chrome.tabs.get(tabId).then(
          (t) => {
            const since = Date.now() - started;
            const moved = fromUrl !== undefined && t.url !== fromUrl && since > 100;
            if (t.status === 'complete' && (sawLoading || !expectNav || moved || since > 1500))
              finish();
            else if (until === 'domcontentloaded' && sawLoading && since > 100) {
              void this.d.page
                .call(tabId, t.url, 'ping', {}, { deadline: c.deadline })
                .then((r) => {
                  if (r['readyState'] !== 'loading' && r['url'] === t.url) finish();
                })
                .catch(() => undefined);
            }
          },
          () => finish(new CommandError('no_tab', 'the tab closed while loading')),
        );
      }, 150);
      const timer = setTimeout(
        () => finish(new CommandError('timeout', 'the page did not finish loading in time')),
        Math.max(0, c.deadline - Date.now()),
      );
    }).then(async () => {
      if (until === 'networkidle') await this.networkIdle(tabId, c, c.deadline);
    });
  }

  /**
   * A quiet network: exact when the debugger is attached (its Network events
   * count every request), else the page heuristic (the page hook's open
   * fetch/XHR count and resource timing).
   */
  private async networkIdle(
    tabId: number,
    c: Cmd,
    deadline: number,
    quietMs = 500,
  ): Promise<number> {
    const start = Date.now();
    if (this.d.dbg.isAttached(tabId)) {
      for (;;) {
        const net = this.d.dbg.network(tabId);
        if (!net) break;
        const now = Date.now();
        if (net.inflight === 0 && now - net.last >= quietMs) return now - start;
        if (now >= deadline) {
          throw new CommandError('timeout', `the network did not go quiet in ${now - start} ms`);
        }
        await sleep(50, c.signal);
      }
    }
    const t = await chrome.tabs.get(tabId);
    if (!(await hasHostAccess(t.url))) return Date.now() - start;
    const r = await this.d.page.call(tabId, t.url, 'wait', { for: 'networkidle' }, { deadline });
    return typeof r['waitedMs'] === 'number' ? r['waitedMs'] : Date.now() - start;
  }

  private async afterNavigation(tabId: number): Promise<Data> {
    const info = await this.pageInfo(tabId);
    const out: Data = { ...info };
    if (await hasHostAccess(info.url)) {
      try {
        const r = await this.d.page.call(
          tabId,
          info.url,
          'status',
          {},
          { deadline: Date.now() + 2000 },
        );
        if (typeof r['status'] === 'number' && r['status'] > 0) out['status'] = r['status'];
        if (typeof r['title'] === 'string' && r['title']) out['title'] = r['title'];
      } catch (e) {
        if (e instanceof CommandError && e.code === 'navigation') throw e;
      }
    }
    return out;
  }

  private async navigate(c: Cmd): Promise<Data> {
    const tabId = this.tabId(c);
    const url = checkUrl(str(c.params['url']));
    this.refuseTarget(url);
    const w = str(c.params['waitUntil']);
    const until = w === 'domcontentloaded' || w === 'networkidle' ? w : 'load';
    if (c.rec.placeholderTabId === tabId) c.rec.placeholderTabId = null;
    const loaded = this.waitForLoad(tabId, c, until, true);
    loaded.catch(() => undefined);
    await chrome.tabs.update(tabId, { url });
    await loaded;
    return this.afterNavigation(tabId);
  }

  private async history(c: Cmd, which: 'back' | 'forward' | 'reload'): Promise<Data> {
    const tabId = this.tabId(c);
    const before = this.url(c);
    const loaded = this.waitForLoad(
      tabId,
      c,
      'load',
      true,
      which === 'reload' ? undefined : before,
    );
    loaded.catch(() => undefined);
    try {
      if (which === 'reload') await chrome.tabs.reload(tabId);
      else await this.stepHistory(tabId, before, which === 'back' ? -1 : 1);
    } catch (e) {
      throw new CommandError('navigation', `cannot go ${which}: ${messageOf(e)}`);
    }
    await loaded;
    return this.pageInfo(tabId);
  }

  /**
   * One step through the session history, as the page's own history.go() —
   * chrome.tabs.goBack skips entries a page added without a user gesture
   * (Chrome's history intervention), which is every pushState after a
   * scripted click.
   */
  private async stepHistory(tabId: number, url: string | undefined, delta: -1 | 1): Promise<void> {
    if (await hasHostAccess(url)) {
      const [res] = await chrome.scripting.executeScript({
        target: { tabId, frameIds: [0] },
        func: (d: number) => {
          const length = history.length;
          history.go(d);
          return length;
        },
        args: [delta],
      });
      if (typeof res?.result === 'number' && res.result <= 1) {
        throw new Error('there is no page to go to in this tab’s history');
      }
      return;
    }
    if (delta < 0) await chrome.tabs.goBack(tabId);
    else await chrome.tabs.goForward(tabId);
  }

  // ------------------------------------------------------------ the mouse

  private async mouse(c: Cmd): Promise<Data> {
    const tabId = this.tabId(c);
    const result =
      c.params['trusted'] === true
        ? await this.raceDialog(tabId, this.navigating(c, tabId, this.trustedMouse(c)))
        : await this.raced(c);
    const openUrl = str(result['openUrl']);
    if (!openUrl) return result;
    delete result['openUrl'];
    // A link meant for a new tab: open it in the lease, as the browser would have.
    const { siteModes, settings } = this.d.store.session.desktop;
    if (modeFor(openUrl, siteModes, settings) === 'off') {
      return { ...result, newTab: { url: openUrl, refused: 'site_off' } };
    }
    const tab = await this.d.leases.openTab(c.rec, openUrl, true);
    return { ...result, newTab: { tabId: tab.id, url: openUrl } };
  }

  private async trustedMouse(c: Cmd): Promise<Data> {
    const tabId = this.tabId(c);
    const kind = c.action;
    await this.d.dbg.ensure(tabId, 'a trusted click');
    const point = await this.call(c, 'trustedPoint', { ...c.params, hover: kind === 'hover' });
    const x = num(point['x']) ?? 0;
    const y = num(point['y']) ?? 0;
    const name = str(c.params['button']);
    const button = name === 'right' || name === 'middle' ? name : 'left';
    const pressed = button === 'right' ? 2 : button === 'middle' ? 4 : 1;
    const send = (p: Data): Promise<Data> => this.d.dbg.send(tabId, 'Input.dispatchMouseEvent', p);
    await send({ type: 'mouseMoved', x, y, button: 'none', buttons: 0 });
    if (kind !== 'hover') {
      const clicks = kind === 'dblclick' ? 2 : 1;
      for (let i = 1; i <= clicks; i++) {
        await send({ type: 'mousePressed', x, y, button, buttons: pressed, clickCount: i });
        await send({ type: 'mouseReleased', x, y, button, buttons: 0, clickCount: i });
      }
    }
    try {
      return await this.call(c, 'trustedDone', {});
    } catch (e) {
      if (e instanceof CommandError && (e.code === 'navigation' || e.code === 'internal')) {
        return this.pageInfo(tabId);
      }
      throw e;
    }
  }

  // --------------------------------------------------------- the keyboard

  private async keys(tabId: number, def: KeyDef, mods: readonly Modifier[]): Promise<void> {
    const modifiers = modifierMask(mods);
    const command = mods.includes('Control') || mods.includes('Meta') || mods.includes('Alt');
    const text = command ? undefined : def.text;
    const base = {
      key: def.key,
      code: def.code,
      windowsVirtualKeyCode: def.keyCode,
      nativeVirtualKeyCode: def.keyCode,
      modifiers,
    };
    await this.d.dbg.send(tabId, 'Input.dispatchKeyEvent', {
      ...base,
      type: text ? 'keyDown' : 'rawKeyDown',
      ...(text ? { text, unmodifiedText: text } : {}),
    });
    await this.d.dbg.send(tabId, 'Input.dispatchKeyEvent', { ...base, type: 'keyUp' });
  }

  private async trustedType(c: Cmd): Promise<Data> {
    const tabId = this.tabId(c);
    await this.d.dbg.ensure(tabId, 'trusted typing');
    await this.call(c, 'focusForKeys', { ...c.params, typing: true });
    const work = (async (): Promise<Data> => {
      const backspace = parseKey('Backspace');
      if (c.params['clear'] === true && backspace) await this.keys(tabId, backspace.def, []);
      for (const ch of Array.from(str(c.params['text']) ?? '')) {
        if (c.signal.aborted) throw abortReason(c.signal);
        const def = charKey(ch);
        if (def.code) await this.keys(tabId, def, []);
        else await this.d.dbg.send(tabId, 'Input.insertText', { text: ch });
      }
      if (c.params['submit'] === true) {
        const enter = parseKey('Enter');
        if (enter) await this.keys(tabId, enter.def, []);
        return this.call(c, 'trustedDone', {}).catch(() => this.pageInfo(tabId));
      }
      return {};
    })();
    return this.raceDialog(tabId, work);
  }

  private async trustedPress(c: Cmd): Promise<Data> {
    const tabId = this.tabId(c);
    const key = str(c.params['key']) ?? '';
    const mods = Array.isArray(c.params['modifiers'])
      ? (c.params['modifiers'] as unknown[]).filter((m): m is string => typeof m === 'string')
      : [];
    const platform = await chrome.runtime.getPlatformInfo().catch(() => ({ os: '' }));
    const spec = parseKey(key, mods, platform.os === 'mac');
    if (!spec) throw new CommandError('unsupported', `unknown key "${key}"`);
    await this.d.dbg.ensure(tabId, 'a trusted key press');
    await this.call(c, 'focusForKeys', { typing: !!spec.def.text });
    const work = (async (): Promise<Data> => {
      await this.keys(tabId, spec.def, spec.modifiers);
      return this.call(c, 'trustedDone', {}).catch(() => this.pageInfo(tabId));
    })();
    return this.raceDialog(tabId, work);
  }

  // -------------------------------------------------------------- waiting

  private async wait(c: Cmd): Promise<Data> {
    const kind = str(c.params['for']);
    const own = num(c.params['timeoutMs']);
    const deadline = Math.min(c.deadline, own !== undefined ? Date.now() + own : c.deadline);
    const start = Date.now();
    if (kind === 'time') {
      const ms = Math.max(0, num(c.params['value']) ?? own ?? 1000);
      const end = Math.min(start + ms, c.deadline);
      await sleep(Math.max(0, end - Date.now()), c.signal);
      if (start + ms > c.deadline)
        throw new CommandError('timeout', `cannot wait ${ms} ms within the command's deadline`);
      return { waitedMs: Date.now() - start };
    }
    if (kind === 'url') {
      const tabId = this.tabId(c);
      const pattern = str(c.params['value']) ?? '';
      await new Promise<void>((resolve, reject) => {
        const check = (url: string | undefined): boolean => !!url && urlMatches(url, pattern);
        const done = (err?: Error): void => {
          chrome.tabs.onUpdated.removeListener(onUpdated);
          clearTimeout(timer);
          clearInterval(poll);
          c.signal.removeEventListener('abort', onAbort);
          if (err) reject(err);
          else resolve();
        };
        const onAbort = (): void => done(abortReason(c.signal));
        const onUpdated = (
          id: number,
          info: chrome.tabs.OnUpdatedInfo,
          tab: chrome.tabs.Tab,
        ): void => {
          if (id === tabId && (check(info.url) || check(tab.url))) done();
        };
        chrome.tabs.onUpdated.addListener(onUpdated);
        c.signal.addEventListener('abort', onAbort, { once: true });
        const poll = setInterval(() => {
          void chrome.tabs.get(tabId).then(
            (t) => {
              if (check(t.url) || check(t.pendingUrl)) done();
            },
            () => done(new CommandError('no_tab', 'the tab closed')),
          );
        }, 200);
        const timer = setTimeout(
          () => done(new CommandError('timeout', `the URL did not match ${pattern} in time`)),
          Math.max(0, deadline - Date.now()),
        );
        if (check(this.url(c))) done();
      });
      return { waitedMs: Date.now() - start };
    }
    if (kind === 'networkidle') {
      return { waitedMs: await this.networkIdle(this.tabId(c), c, deadline) };
    }
    if (kind === 'selector' || kind === 'text') {
      const params = { ...c.params, ...(own !== undefined ? { timeoutMs: own } : {}) };
      return this.call(c, 'wait', params);
    }
    throw new CommandError(
      'unsupported',
      `cannot wait for "${kind ?? ''}": use selector, text, url, networkidle or time`,
    );
  }

  // ---------------------------------------------------------- screenshots

  private async screenshot(c: Cmd): Promise<Data> {
    if (this.d.store.session.desktop.settings.screenshots === false) {
      throw new CommandError(
        'unsupported',
        'screenshots are turned off in Mother Claude’s settings',
      );
    }
    const tabId = this.tabId(c);
    const format = str(c.params['format']) === 'jpeg' ? 'jpeg' : 'png';
    const element = str(c.params['scope']) === 'element';
    if (element && !str(c.params['ref']) && !str(c.params['selector'])) {
      throw new CommandError('not_found', 'an element screenshot needs a ref');
    }
    if (await this.d.dbg.available()) {
      await this.d.dbg.ensure(tabId, 'a screenshot');
      return this.cdpScreenshot(c, tabId, format, element);
    }
    return this.visibleTabScreenshot(c, format, element);
  }

  private async cdpScreenshot(
    c: Cmd,
    tabId: number,
    format: 'png' | 'jpeg',
    element: boolean,
  ): Promise<Data> {
    let clip: Data | undefined;
    if (element) {
      const r = await this.call(c, 'rect', c.params);
      const width = num(r['width']) ?? 0;
      const height = num(r['height']) ?? 0;
      if (width < 1 || height < 1)
        throw new CommandError('not_found', 'the element has no size on screen');
      clip = {
        x: (num(r['x']) ?? 0) + (num(r['scrollX']) ?? 0),
        y: (num(r['y']) ?? 0) + (num(r['scrollY']) ?? 0),
        width,
        height,
        scale: 1,
      };
    }
    const shoot = async (fmt: 'png' | 'jpeg', quality?: number): Promise<string> => {
      const res = await this.d.dbg.send<{ data?: string }>(tabId, 'Page.captureScreenshot', {
        format: fmt,
        ...(quality ? { quality } : {}),
        ...(clip ? { clip } : {}),
        captureBeyondViewport: false,
        fromSurface: true,
      });
      if (!res.data) throw new CommandError('internal', 'the browser returned no image');
      return res.data;
    };
    let mimeType = `image/${format}`;
    let data = await shoot(format);
    if (data.length > MAX_IMAGE_BASE64) {
      data = await shoot('jpeg', 70);
      mimeType = 'image/jpeg';
    }
    if (data.length > MAX_IMAGE_BASE64) {
      throw new CommandError(
        'frame_too_large',
        'the screenshot is larger than a frame may be: take an element screenshot',
      );
    }
    return { data, mimeType, ...imageSize(data) };
  }

  private async visibleTabScreenshot(
    c: Cmd,
    format: 'png' | 'jpeg',
    element: boolean,
  ): Promise<Data> {
    const tab = c.tab;
    if (!tab || tab.id === undefined) throw new CommandError('no_tab', 'no tab');
    const win = await chrome.windows.get(tab.windowId).catch(() => null);
    if (!tab.active || !win || win.state === 'minimized') {
      throw permissionNeeded(
        'debugger',
        'a screenshot of a tab that is not on screen needs the "debugger" permission: grant it in the side panel',
      );
    }
    let rect: Data | null = null;
    if (element) rect = await this.call(c, 'rect', c.params);
    const capture = (): Promise<string> =>
      chrome.tabs.captureVisibleTab(tab.windowId, {
        format,
        ...(format === 'jpeg' ? { quality: 80 } : {}),
      });
    let dataUrl: string;
    try {
      dataUrl = await capture();
    } catch (e) {
      const msg = messageOf(e);
      if (/MAX_CAPTURE|quota|rate/i.test(msg)) {
        await sleep(600, c.signal);
        dataUrl = await capture();
      } else if (/permission|<all_urls>|activeTab/i.test(msg)) {
        throw permissionNeeded(
          'debugger',
          'screenshots need the "debugger" permission (or access to all sites): grant it in the side panel',
        );
      } else throw new CommandError('internal', msg);
    }
    let data = dataUrl.slice(dataUrl.indexOf(',') + 1);
    let mimeType = `image/${format}`;
    if (rect) {
      const dpr = num(rect['dpr']) ?? 1;
      const blob = await (await fetch(dataUrl)).blob();
      const bitmap = await createImageBitmap(blob);
      const sx = Math.max(0, Math.round((num(rect['x']) ?? 0) * dpr));
      const sy = Math.max(0, Math.round((num(rect['y']) ?? 0) * dpr));
      const sw = Math.max(
        1,
        Math.min(bitmap.width - sx, Math.round((num(rect['width']) ?? 1) * dpr)),
      );
      const sh = Math.max(
        1,
        Math.min(bitmap.height - sy, Math.round((num(rect['height']) ?? 1) * dpr)),
      );
      const canvas = new OffscreenCanvas(sw, sh);
      const ctx = canvas.getContext('2d');
      if (!ctx) throw new CommandError('internal', 'no canvas');
      ctx.drawImage(bitmap, sx, sy, sw, sh, 0, 0, sw, sh);
      const out = await canvas.convertToBlob({ type: mimeType });
      data = await blobBase64(out);
      mimeType = out.type || mimeType;
    }
    if (data.length > MAX_IMAGE_BASE64) {
      throw new CommandError('frame_too_large', 'the screenshot is larger than a frame may be');
    }
    return { data, mimeType, ...imageSize(data) };
  }

  // ----------------------------------------------------- debugger-only tools

  private async evaluate(c: Cmd): Promise<Data> {
    const tabId = this.tabId(c);
    const expression = str(c.params['expression']);
    if (!expression) throw new CommandError('not_found', 'eval needs an expression');
    await this.d.dbg.ensure(tabId, 'eval');
    const res = await this.d.dbg.send<{
      result?: {
        value?: unknown;
        type?: string;
        description?: string;
        unserializableValue?: string;
      };
      exceptionDetails?: { text?: string; exception?: { description?: string } };
    }>(tabId, 'Runtime.evaluate', {
      expression,
      awaitPromise: c.params['awaitPromise'] !== false,
      returnByValue: true,
      userGesture: true,
      timeout: Math.max(100, c.deadline - Date.now() - 100),
    });
    if (res.exceptionDetails) {
      const text =
        res.exceptionDetails.exception?.description ?? res.exceptionDetails.text ?? 'exception';
      throw new CommandError('internal', `the expression threw: ${text}`, { exception: text });
    }
    const r = res.result ?? {};
    const value =
      r.value !== undefined
        ? r.value
        : (r.unserializableValue ?? (r.type === 'undefined' ? null : (r.description ?? null)));
    const size = JSON.stringify(value ?? null).length;
    if (size > 1024 * 1024)
      throw new CommandError('frame_too_large', `the value is ${size} bytes: return less`);
    return { value: value ?? null };
  }

  private async console(c: Cmd): Promise<Data> {
    const tabId = this.tabId(c);
    await this.d.dbg.ensure(tabId, 'reading the console');
    const since = num(c.params['since']);
    const level = str(c.params['level']);
    const min = level ? LEVELS.indexOf(level === 'warn' ? 'warning' : level) : -1;
    const all = this.d.dbg
      .consoleMessages(tabId)
      .filter(
        (m) =>
          (since === undefined || m.at >= since) && (min < 0 || LEVELS.indexOf(m.level) >= min),
      );
    const messages = all.slice(-200).map((m) => ({ level: m.level, text: m.text, at: m.at }));
    return all.length > 200 ? { messages, truncated: true } : { messages };
  }

  private async dialog(c: Cmd): Promise<Data> {
    const tabId = this.tabId(c);
    await this.d.dbg.ensure(tabId, 'answering a dialog');
    const text = str(c.params['text']);
    try {
      await this.d.dbg.send(tabId, 'Page.handleJavaScriptDialog', {
        accept: c.params['accept'] !== false,
        ...(text !== undefined ? { promptText: text } : {}),
      });
    } catch (e) {
      if (/No dialog/i.test(messageOf(e))) {
        this.d.dialogs.closed(tabId);
        throw new CommandError('not_found', 'no dialog is open in this tab');
      }
      throw e;
    }
    this.d.dialogs.closed(tabId);
    return {};
  }
}

async function blobBase64(blob: Blob): Promise<string> {
  const bytes = new Uint8Array(await blob.arrayBuffer());
  let s = '';
  for (let i = 0; i < bytes.length; i += 0x8000) {
    s += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
  }
  return btoa(s);
}
