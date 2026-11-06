// The service worker. Every chrome.* listener is registered synchronously at
// the top level (that is what lets an event wake a stopped MV3 worker); the
// work behind them waits for the stored state to load.

import { messageOf } from '../shared/errors.ts';
import type {
  IncomingFrame,
  LeaseCloseFrame,
  LeaseOpenFrame,
  StateFrame,
  WelcomeFrame,
} from '../shared/protocol.ts';
import { isSiteMode } from '../shared/protocol.ts';
import { isSensitive, modeFor, normalizeOriginKey, originOf } from '../shared/site.ts';
import {
  UI_PORT,
  isUiMessage,
  type TabInfo,
  type UiRequest,
  type UiResponse,
  type UiStatus,
} from '../shared/ui.ts';
import { Commands } from './commands.ts';
import { Connection, DesktopError } from './connection.ts';
import { DebuggerManager } from './debugger.ts';
import { Dialogs, dialogKind, type DialogInfo } from './dialogs.ts';
import { EXTENSION_VERSION, defaultBrowserName, grantedList, hasHostAccess } from './env.ts';
import { Leases, isPlaceholderUrl } from './leases.ts';
import { PageBridge } from './page.ts';
import { Router } from './router.ts';
import { Store, normalizeHost, validPort, type LeaseRecord } from './state.ts';

const store = new Store();
const ports = new Set<chrome.runtime.Port>();
let uiTimer: ReturnType<typeof setTimeout> | null = null;
/** Events that matter even when the desktop was away when they happened. */
const queuedEvents: Record<string, unknown>[] = [];

function sendEvent(event: Record<string, unknown>, queueIfOffline = false): void {
  const frame = { type: 'event', ...event };
  if (!conn.send(frame) && queueIfOffline) queuedEvents.push(frame);
}

const dialogs = new Dialogs((tabId: number, d: DialogInfo) => {
  const rec = leases.leaseOfTab(tabId);
  if (!rec && store.session.shared?.tabId !== tabId) return;
  sendEvent({
    event: 'dialog.opened',
    tabId,
    ...(rec ? { lease: rec.lease } : {}),
    kind: d.kind,
    message: d.message,
  });
  uiChanged();
});
const dbg = new DebuggerManager(dialogs);
dbg.onCancelled = uiChanged;
const page = new PageBridge(store);
const leases: Leases = new Leases(store, {
  revoked: (rec: LeaseRecord) => {
    router.abortLease(rec.lease, 'the user pressed Stop in the browser');
    sendEvent({ event: 'lease.revoked', lease: rec.lease, by: 'user' }, true);
  },
  changed: uiChanged,
});
const commands = new Commands({ store, leases, page, dbg, dialogs });
const conn: Connection = new Connection(store, {
  frame: (f) => void onFrame(f),
  welcome: onWelcome,
  changed: uiChanged,
  closed: () => router.abortAll(),
});
const router: Router = new Router({
  store,
  leases,
  commands,
  dialogs,
  dbg,
  send: (frame) => {
    if (conn.trySend(frame) === 'too_large') {
      // The desktop is still owed an answer.
      conn.send({
        type: 'result',
        id: frame.id,
        ok: false,
        error: {
          code: 'frame_too_large',
          message: 'the result is larger than a frame may be (8 MiB)',
        },
      });
    }
  },
  changed: uiChanged,
  cancelPage: (tabId) => page.cancel(tabId),
});

/** "Chrome on Mac": offered when pairing; never written at startup (that would race the options page). */
let defaultName = '';

const ready: Promise<void> = (async () => {
  await store.load();
  defaultName = await defaultBrowserName();
  await leases.verify();
  dbg.listen();
  conn.ensure();
  uiChanged();
})().catch((e: unknown) => {
  console.error('Mother Claude: start failed', e);
});

// ------------------------------------------------------------- the desktop

async function onFrame(frame: IncomingFrame): Promise<void> {
  await ready;
  switch (frame.type) {
    case 'state':
      await applyState(frame);
      return;
    case 'lease.open':
      await openLease(frame);
      return;
    case 'lease.close':
      await closeLease(frame);
      return;
    case 'command':
      router.handle(frame);
      return;
    default:
      return;
  }
}

function onWelcome(w: WelcomeFrame): void {
  const d = store.session.desktop;
  if (w.siteModes) d.siteModes = w.siteModes;
  if (w.settings) d.settings = w.settings;
  d.appVersion = w.appVersion ?? '';
  store.persist();
  while (queuedEvents.length) {
    const e = queuedEvents.shift();
    if (e) conn.send(e);
  }
  uiChanged();
}

async function applyState(s: StateFrame): Promise<void> {
  const d = store.session.desktop;
  if (s.siteModes) d.siteModes = s.siteModes;
  if (s.settings) d.settings = s.settings;
  if (s.pending) d.pending = s.pending;
  if (s.leases) {
    d.leases = s.leases;
    await leases.reconcile(s.leases);
  }
  store.persist();
  uiChanged();
}

async function openLease(f: LeaseOpenFrame): Promise<void> {
  try {
    const groupId = await leases.open(f);
    conn.send({ type: 'lease.opened', id: f.id, lease: f.lease, groupId });
  } catch (e) {
    conn.send({
      type: 'error',
      id: f.id,
      code: 'internal',
      message: `cannot open the tab group: ${messageOf(e)}`,
    });
  }
}

async function closeLease(f: LeaseCloseFrame): Promise<void> {
  router.abortLease(f.lease, 'the lease was closed');
  const tabs = await leases.close(f.lease, f.closeTabs);
  await Promise.all(tabs.map((id) => dbg.detach(id)));
  conn.send({ type: 'ok', id: f.id });
}

// ------------------------------------------------------------------- the UI

async function leaseTabCount(rec: LeaseRecord): Promise<number> {
  return (await leases.pageTabs(rec)).length;
}

async function status(): Promise<UiStatus> {
  const d = store.session.desktop;
  const list = await Promise.all(
    leases.all().map(async (rec) => {
      const now = router.activity.get(rec.lease);
      const last = router.lastActivity.get(rec.lease);
      return {
        lease: rec.lease,
        title: rec.session.title || 'Claude Code',
        color: rec.color,
        tabs: await leaseTabCount(rec),
        activity: now?.text ?? null,
        activitySince: now?.since ?? null,
        last: last?.text ?? null,
        lastAt: last?.since ?? null,
        stoppedUntil: rec.stoppedUntil > Date.now() ? rec.stoppedUntil : 0,
      };
    }),
  );
  return {
    connection: {
      state: conn.state,
      error: conn.lastError,
      appVersion: conn.appVersion || d.appVersion || '',
      retryAt: conn.retryAt,
      host: store.config.host,
      port: store.config.port,
      pingMs: conn.pingMs,
    },
    paired: !!store.auth,
    browserName: store.config.browserName || defaultName,
    browserId: store.auth?.browserId ?? '',
    leases: list,
    pending: d.pending,
    siteModes: d.siteModes,
    settings: d.settings,
    granted: await grantedList(),
    debuggerAttached: dbg.attachedCount,
    extensionVersion: EXTENSION_VERSION,
    extensionId: chrome.runtime.id,
  };
}

function uiChanged(): void {
  if (uiTimer) return;
  uiTimer = setTimeout(() => {
    uiTimer = null;
    void (async () => {
      await ready;
      const s = await status();
      for (const port of ports) {
        try {
          port.postMessage({ type: 'status', status: s });
        } catch {
          ports.delete(port);
        }
      }
      updateBadge(s);
    })();
  }, 40);
}

function updateBadge(s: UiStatus): void {
  const active = s.leases.filter((l) => !l.stoppedUntil).length;
  let text = '';
  let color = '#7c5cc9';
  if (s.pending.length) {
    text = String(s.pending.length);
    color = '#d9480f';
  } else if (s.paired && s.connection.state !== 'connected') {
    text = '!';
    color = '#868e96';
  } else if (active) {
    text = String(active);
  }
  const state =
    s.connection.state === 'connected'
      ? `connected${active ? `, ${active} session${active === 1 ? '' : 's'} working` : ''}`
      : s.paired
        ? 'not connected to the app'
        : 'not paired';
  void chrome.action.setBadgeText({ text }).catch(() => undefined);
  void chrome.action.setBadgeBackgroundColor({ color }).catch(() => undefined);
  void chrome.action.setTitle({ title: `Mother Claude — ${state}` }).catch(() => undefined);
}

async function tabInfo(tabId: number): Promise<TabInfo> {
  const tab = await chrome.tabs.get(tabId);
  const url = tab.url ?? tab.pendingUrl ?? '';
  const origin = originOf(url);
  const d = store.session.desktop;
  const rec = tab.groupId >= 0 ? leases.byGroup(tab.groupId) : undefined;
  const dialog = dialogs.get(tabId);
  return {
    tabId,
    url,
    title: tab.title ?? '',
    origin,
    mode: modeFor(url, d.siteModes, d.settings),
    hostGranted: await hasHostAccess(url),
    lease: rec?.lease ?? null,
    shared: store.session.shared?.tabId === tabId,
    activeGrant: !!origin && store.session.activeGrants[String(tabId)] === origin,
    dialog: dialog ? { kind: dialog.kind, message: dialog.message } : null,
    ownEntry:
      !!origin &&
      Object.keys(d.siteModes).some((k) => k !== '*' && normalizeOriginKey(k) === origin),
    sensitive: isSensitive(origin, d.settings),
  };
}

const PAIR_MESSAGES: Record<string, string> = {
  bad_code:
    'That code is not right. Check the 8 characters shown in Mother Claude (Settings › Browser › Pair a browser).',
  expired_code: 'That code has expired. Show a new one in Mother Claude and try again.',
  rate_limited: 'Too many tries. Wait a minute, then try again.',
  timeout: 'Mother Claude did not answer. Is it running?',
};

function fail(code: string, message: string): UiResponse {
  return { ok: false, code, message };
}

async function desktopRequest(frame: Record<string, unknown>): Promise<UiResponse> {
  if (!conn.isAuthenticated) return fail('disconnected', 'Not connected to Mother Claude.');
  try {
    await conn.request(frame);
    return { ok: true };
  } catch (e) {
    const code = e instanceof DesktopError ? e.code : 'error';
    if (code === 'needs_desktop') {
      return fail(
        code,
        'This site is on Mother Claude’s list of sensitive sites. Raise it to act on the desktop, in Settings › Browser.',
      );
    }
    if (code === 'already_answered') return fail(code, 'Already answered elsewhere.');
    if (code === 'not_found') return fail(code, 'That request is no longer waiting.');
    if (code === 'refused') return fail(code, 'Mother Claude did not take that answer from here.');
    if (code === 'invalid') return fail(code, 'Mother Claude did not understand that site.');
    return fail(code, messageOf(e));
  }
}

async function handleUi(req: UiRequest): Promise<UiResponse> {
  await ready;
  switch (req.op) {
    case 'status':
      return { ok: true, status: await status() };
    case 'pair': {
      // Case, spaces and dashes don't matter (the desktop also reads O as 0, I/L as 1).
      const code = req.code.toUpperCase().replace(/[\s-]+/g, '');
      if (!/^[A-Z0-9]{8}$/.test(code)) {
        return fail('bad_code', 'Enter the 8-character code Mother Claude shows, like ABCD-EFGH.');
      }
      const formatted = `${code.slice(0, 4)}-${code.slice(4)}`;
      const name = req.name.trim() || store.config.browserName || defaultName;
      if (name !== store.config.browserName) {
        store.config.browserName = name;
        await store.saveConfig();
      }
      try {
        await conn.pair(formatted, name);
        return { ok: true };
      } catch (e) {
        const code2 = e instanceof DesktopError ? e.code : 'error';
        return fail(code2, PAIR_MESSAGES[code2] ?? messageOf(e));
      }
    }
    case 'forget': {
      router.abortAll();
      for (const rec of leases.all()) {
        const tabs = await leases.close(rec.lease, false);
        await Promise.all(tabs.map((id) => dbg.detach(id)));
      }
      await conn.forget();
      return { ok: true };
    }
    case 'setConfig': {
      const host = normalizeHost(req.host);
      if (!host)
        return fail('bad_host', 'Mother Claude runs on this computer: use 127.0.0.1 or localhost.');
      if (!validPort(req.port)) return fail('bad_port', 'The port is a number from 1 to 65535.');
      const changed = host !== store.config.host || req.port !== store.config.port;
      store.config.host = host;
      store.config.port = req.port;
      if (typeof req.browserName === 'string' && req.browserName.trim())
        store.config.browserName = req.browserName.trim();
      await store.saveConfig();
      if (changed) conn.restart();
      uiChanged();
      return { ok: true };
    }
    case 'stop':
      if (!leases.revoke(req.lease))
        return fail('no_lease', 'That session no longer works in this browser.');
      return { ok: true };
    case 'resume':
      if (!(await leases.resume(req.lease)))
        return fail('no_lease', 'That session no longer works in this browser.');
      sendEvent({ event: 'lease.resumed', lease: req.lease, by: 'user' });
      return { ok: true };
    case 'setSiteMode': {
      const origin = originOf(req.origin);
      if (!origin) return fail('bad_origin', 'Not a web page.');
      if (req.mode !== null && !isSiteMode(req.mode)) return fail('bad_mode', 'Unknown mode.');
      // `mode: null` removes the site's own entry (a desktop addition to v1).
      return desktopRequest({ type: 'setSiteMode', origin, mode: req.mode });
    }
    case 'answer':
      return desktopRequest({ type: 'answer', pending: req.pending, allow: req.allow });
    case 'tabInfo':
      try {
        return { ok: true, tab: await tabInfo(req.tabId) };
      } catch {
        return fail('no_tab', 'No such tab.');
      }
    case 'share': {
      const info = await tabInfo(req.tabId).catch(() => null);
      if (!info?.origin) return fail('no_tab', 'This page cannot be shared.');
      if (!info.hostGranted && !info.activeGrant) {
        return fail('permission_needed', 'Let Mother Claude into this site first.');
      }
      store.session.shared = { tabId: req.tabId, origin: info.origin };
      store.persist();
      uiChanged();
      return { ok: true };
    }
    case 'unshare':
      store.session.shared = null;
      store.persist();
      uiChanged();
      return { ok: true };
    case 'addToLease': {
      const rec = leases.get(req.lease);
      if (!rec) return fail('no_lease', 'That session no longer works in this browser.');
      try {
        if (rec.groupId !== null)
          await chrome.tabs.group({ groupId: rec.groupId, tabIds: [req.tabId] });
        else return fail('no_group', 'The session has no tab group right now.');
        leases.noteTab(req.tabId, rec.groupId);
        uiChanged();
        return { ok: true };
      } catch (e) {
        return fail('error', messageOf(e));
      }
    }
    case 'reconnect':
      conn.restart();
      return { ok: true };
    case 'permissionsChanged':
      await permissionsChanged();
      return { ok: true };
    case 'leaseInfo': {
      const rec = leases.get(req.lease);
      return rec
        ? { ok: true, title: rec.session.title, color: rec.color, stopped: leases.isStopped(rec) }
        : fail('no_lease', 'This session no longer works in this browser.');
    }
    default:
      return fail('unknown', 'Unknown request.');
  }
}

async function permissionsChanged(): Promise<void> {
  hookOptionalApis();
  dbg.listen();
  sendEvent({ event: 'permission.changed', granted: await grantedList() });
  uiChanged();
}

// ------------------------------------------------------------ chrome events

function tabEvent(
  tabId: number,
  rec: LeaseRecord | undefined,
  tab: chrome.tabs.Tab,
  status: string,
): void {
  if (!rec && store.session.shared?.tabId !== tabId) return;
  const url = tab.url ?? tab.pendingUrl ?? '';
  if (isPlaceholderUrl(url)) return;
  sendEvent({
    event: 'tab.updated',
    tabId,
    ...(rec ? { lease: rec.lease } : {}),
    url,
    title: tab.title ?? '',
    status,
  });
}

chrome.tabs.onUpdated.addListener((tabId, info, tab) => {
  void ready.then(async () => {
    if (info.groupId !== undefined) {
      leases.noteTab(tabId, info.groupId);
      uiChanged();
    }
    const rec =
      leases.leaseOfTab(tabId) ?? (tab.groupId >= 0 ? leases.byGroup(tab.groupId) : undefined);
    if (rec) leases.noteTab(tabId, tab.groupId);
    if (info.url !== undefined || info.status === 'complete') {
      tabEvent(tabId, rec, tab, info.status ?? tab.status ?? 'complete');
    }
    if (info.url !== undefined) {
      dialogs.closed(tabId);
      const shared = store.session.shared;
      if (shared?.tabId === tabId && originOf(info.url) !== shared.origin) {
        store.session.shared = null;
        store.persist();
      }
      const grant = store.session.activeGrants[String(tabId)];
      if (grant && originOf(info.url) !== grant) {
        delete store.session.activeGrants[String(tabId)];
        store.persist();
      }
    }
    // Lease pages get the page hook early, so a dialog never catches us blind.
    if (
      rec &&
      info.status === 'complete' &&
      !isPlaceholderUrl(tab.url) &&
      (await hasHostAccess(tab.url))
    ) {
      await page.inject(tabId).catch(() => undefined);
    }
    if (info.status === 'complete' || info.url !== undefined || info.title !== undefined)
      uiChanged();
  });
});

chrome.tabs.onRemoved.addListener((tabId) => {
  void ready.then(async () => {
    const rec = leases.leaseOfClosedTab(tabId);
    if (rec && rec.placeholderTabId !== tabId)
      sendEvent({ event: 'tab.closed', tabId, lease: rec.lease });
    leases.forgetTab(tabId);
    store.forgetTab(tabId);
    dialogs.closed(tabId);
    await dbg.detach(tabId);
    uiChanged();
  });
});

chrome.tabs.onCreated.addListener((tab) => {
  void ready.then(() => {
    if (tab.id !== undefined && tab.groupId >= 0) leases.noteTab(tab.id, tab.groupId);
  });
});

chrome.tabs.onActivated.addListener(({ tabId }) => {
  void ready.then(() => {
    leases.onTabActivated(tabId);
    uiChanged();
  });
});

chrome.tabGroups.onRemoved.addListener((group) => {
  void ready.then(() => leases.onGroupRemoved(group.id));
});

chrome.permissions.onAdded.addListener(() => void ready.then(permissionsChanged));
chrome.permissions.onRemoved.addListener(() => void ready.then(permissionsChanged));

chrome.storage.onChanged.addListener((changes, area) => {
  if (area !== 'local' || !changes['config']) return;
  void ready.then(async () => {
    const before = `${store.config.host}:${store.config.port}`;
    await store.load();
    if (`${store.config.host}:${store.config.port}` !== before) conn.restart();
    uiChanged();
  });
});

chrome.runtime.onConnect.addListener((port) => {
  if (port.name !== UI_PORT) return;
  ports.add(port);
  port.onDisconnect.addListener(() => ports.delete(port));
  uiChanged();
});

chrome.runtime.onMessage.addListener((msg: unknown, sender, sendResponse) => {
  if (isUiMessage(msg)) {
    if (sender.id !== chrome.runtime.id || sender.tab?.url?.startsWith('http')) return false;
    handleUi(msg.req).then(sendResponse, (e: unknown) => sendResponse(fail('error', messageOf(e))));
    return true;
  }
  const m = msg as { type?: unknown; open?: unknown; kind?: unknown; message?: unknown };
  if (m?.type === 'mc.dialog' && sender.tab?.id !== undefined) {
    const tabId = sender.tab.id;
    void ready.then(() => {
      if (m.open === true) dialogs.opened(tabId, dialogKind(m.kind), String(m.message ?? ''));
      else dialogs.closed(tabId);
      uiChanged();
    });
  }
  return false;
});

chrome.action.onClicked.addListener((tab) => {
  // Opening the panel must happen inside the click: no await before it.
  if (tab.id !== undefined && chrome.sidePanel?.open) {
    chrome.sidePanel.open({ tabId: tab.id }).catch(() => undefined);
  } else {
    // Firefox: the same page is a sidebar.
    const firefox = (globalThis as { browser?: { sidebarAction?: { open?: () => Promise<void> } } })
      .browser;
    firefox?.sidebarAction?.open?.().catch(() => undefined);
  }
  void ready.then(() => {
    const origin = originOf(tab.url);
    if (tab.id !== undefined && origin) {
      store.session.activeGrants[String(tab.id)] = origin;
      store.persist();
      uiChanged();
    }
  });
});

chrome.runtime.onInstalled.addListener((details) => {
  if (details.reason === 'install') {
    void chrome.runtime.openOptionsPage().catch(() => undefined);
  }
});

chrome.runtime.onStartup.addListener(() => {
  void ready;
});

chrome.alarms.onAlarm.addListener((alarm) => {
  if (alarm.name === 'mc-keepalive') void ready.then(() => conn.wake());
});
void chrome.alarms.create('mc-keepalive', { periodInMinutes: 0.5 });

// ---------------------------------------- optional APIs (present once granted)

let downloadsHooked = false;
let navigationHooked = false;
const pendingDownloads = new Map<
  number,
  { tabId: number; lease: string; url: string; timer: ReturnType<typeof setTimeout> }
>();

function basename(path: string): string {
  return path.split(/[\\/]/).pop() ?? path;
}

function emitDownload(id: number, filename: string): void {
  const p = pendingDownloads.get(id);
  if (!p) return;
  clearTimeout(p.timer);
  pendingDownloads.delete(id);
  sendEvent({
    event: 'download.started',
    tabId: p.tabId,
    lease: p.lease,
    url: p.url,
    filename: basename(filename),
  });
}

function hookOptionalApis(): void {
  if (!downloadsHooked && typeof chrome.downloads !== 'undefined' && chrome.downloads.onCreated) {
    downloadsHooked = true;
    chrome.downloads.onCreated.addListener((item) => {
      void ready.then(async () => {
        for (const rec of leases.all()) {
          for (const t of await leases.pageTabs(rec)) {
            if (t.id === undefined || !t.url) continue;
            const same =
              item.referrer === t.url ||
              (!!item.referrer && originOf(item.referrer) === originOf(t.url) && t.active);
            if (!same) continue;
            const url = item.finalUrl || item.url;
            const timer = setTimeout(
              () => emitDownload(item.id, basename(new URL(url).pathname) || 'download'),
              2000,
            );
            pendingDownloads.set(item.id, { tabId: t.id, lease: rec.lease, url, timer });
            if (item.filename) emitDownload(item.id, item.filename);
            return;
          }
        }
      });
    });
    chrome.downloads.onChanged.addListener((delta) => {
      if (delta.filename?.current) emitDownload(delta.id, delta.filename.current);
    });
  }
  if (
    !navigationHooked &&
    typeof chrome.webNavigation !== 'undefined' &&
    chrome.webNavigation.onCommitted
  ) {
    navigationHooked = true;
    chrome.webNavigation.onCommitted.addListener((d) => {
      if (d.frameId !== 0) return;
      void ready.then(async () => {
        if (leases.leaseOfTab(d.tabId) && (await hasHostAccess(d.url)))
          await page.installHook(d.tabId).catch(() => undefined);
      });
    });
  }
}
hookOptionalApis();
