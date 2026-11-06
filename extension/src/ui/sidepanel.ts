// The side panel: connection, pairing, the sessions working here with their
// Stop button, approvals waiting for an answer, and the active tab's site mode.

import type { PendingItem, SiteMode } from '../shared/protocol.ts';
import { hostPattern } from '../shared/site.ts';
import type { TabInfo, UiLease, UiStatus } from '../shared/ui.ts';
import {
  COLOR_HEX,
  CONNECTION_TEXT,
  ago,
  askPermissions,
  byId,
  h,
  keepFocus,
  pairingForm,
  request,
  watchStatus,
} from './common.ts';

/** How long a Stop holds (mirrors the service worker's STOP_HOLD_MS). */
const STOP_HOLD_MS = 60_000;

let status: UiStatus | null = null;
let tab: TabInfo | null = null;
let windowId: number | undefined;
let siteMessage = '';
const answerErrors = new Map<string, string>();

function renderConnection(s: UiStatus): void {
  const box = byId('connection');
  const dot = box.querySelector('.dot');
  const text = box.querySelector('.text');
  const c = s.connection;
  dot?.classList.toggle('on', c.state === 'connected');
  dot?.classList.toggle(
    'off',
    c.state === 'disconnected' || c.state === 'auth_failed' || c.state === 'revoked',
  );
  let line = CONNECTION_TEXT[c.state] ?? c.state;
  if (c.state === 'connected' && c.appVersion) line += ` ${c.appVersion}`;
  if (c.state === 'disconnected' && c.retryAt) {
    line += ` — retrying in ${Math.max(0, Math.round((c.retryAt - Date.now()) / 1000))} s`;
  }
  if (text) text.textContent = line;
  const err = byId('connection-error');
  err.textContent = c.state === 'connected' ? '' : c.error;
}

function renderPairing(s: UiStatus): void {
  const show =
    !s.paired || s.connection.state === 'revoked' || s.connection.state === 'auth_failed';
  const section = byId('pairing');
  section.hidden = !show;
  const slot = byId('pairing-form');
  if (show && !slot.firstChild)
    slot.append(pairingForm(s.browserName, () => slot.replaceChildren()));
  if (!show) slot.replaceChildren();
}

function leaseCard(l: UiLease): HTMLElement {
  const stopped = l.stoppedUntil > Date.now();
  const activity = stopped
    ? `Stopped — commands refused for ${Math.ceil((l.stoppedUntil - Date.now()) / 1000)} s`
    : l.activity
      ? `${l.activity[0]?.toUpperCase() ?? ''}${l.activity.slice(1)}…`
      : l.last && l.lastAt
        ? `Idle — last ${l.last}, ${ago(l.lastAt)}`
        : 'Idle';
  const button = stopped
    ? h(
        'button',
        {
          type: 'button',
          'data-key': `lease-${l.lease}`,
          'aria-label': `Resume ${l.title}`,
          // Resume takes the Stop button's place: a double click on Stop must not undo it.
          disabled: Date.now() - (l.stoppedUntil - STOP_HOLD_MS) < 1000,
          onclick: () => void act({ op: 'resume', lease: l.lease }),
        },
        'Resume',
      )
    : h(
        'button',
        {
          type: 'button',
          class: 'stop',
          'data-key': `lease-${l.lease}`,
          'aria-label': `Stop ${l.title}`,
          onclick: () => void act({ op: 'stop', lease: l.lease }),
        },
        'Stop',
      );
  return h(
    'div',
    { class: 'card', 'data-lease': l.lease },
    h(
      'div',
      { class: 'row' },
      h('span', {
        class: 'swatch',
        style: `background:${COLOR_HEX[l.color] ?? '#888'}`,
        'aria-hidden': 'true',
      }),
      h(
        'div',
        { class: 'grow' },
        h('div', { class: 'title' }, l.title),
        h('div', { class: 'muted activity' }, activity),
      ),
      button,
    ),
    h('div', { class: 'muted' }, `${l.tabs} tab${l.tabs === 1 ? '' : 's'} in its group`),
  );
}

async function act(req: Parameters<typeof request>[0]): Promise<void> {
  const res = await request(req);
  if (!res.ok) byId('connection-error').textContent = res.message;
}

function renderLeases(s: UiStatus): void {
  const section = byId('sessions');
  section.hidden = !s.paired;
  const list = byId('session-list');
  if (!s.leases.length) {
    list.replaceChildren(
      h('p', { class: 'muted' }, 'No Claude Code session is working in this browser right now.'),
    );
    return;
  }
  list.replaceChildren(...s.leases.map(leaseCard));
}

function approvalCard(p: PendingItem): HTMLElement {
  const err = answerErrors.get(p.id);
  const answer = (allow: boolean) => (): void => {
    void request({ op: 'answer', pending: p.id, allow }).then((res) => {
      if (res.ok) return;
      answerErrors.set(p.id, res.message);
      if (status) keepFocus(() => status && renderPending(status));
    });
  };
  const created =
    typeof p.createdAt === 'number'
      ? ago(p.createdAt < 1e12 ? p.createdAt * 1000 : p.createdAt)
      : '';
  return h(
    'div',
    { class: 'card', 'data-pending': p.id },
    h(
      'div',
      { class: 'row' },
      h('div', { class: 'grow title' }, p.summary ?? 'A browser action waits for your answer'),
      p.dangerous ? h('span', { class: 'badge danger' }, 'dangerous') : null,
    ),
    p.detail
      ? h(
          'p',
          { class: 'muted', style: 'white-space:pre-wrap;margin:6px 0' },
          p.detail.slice(0, 600),
        )
      : null,
    h('div', { class: 'muted' }, [p.session, created].filter(Boolean).join(' · ')),
    h(
      'div',
      { class: 'actions' },
      h(
        'button',
        { type: 'button', class: 'primary', 'data-key': `allow-${p.id}`, onclick: answer(true) },
        'Allow',
      ),
      h('button', { type: 'button', 'data-key': `deny-${p.id}`, onclick: answer(false) }, 'Deny'),
    ),
    err ? h('p', { class: 'error' }, err) : null,
  );
}

function renderPending(s: UiStatus): void {
  const section = byId('approvals');
  section.hidden = !s.pending.length;
  byId('approval-list').replaceChildren(...s.pending.map(approvalCard));
}

const MODE_LABELS: Record<SiteMode, string> = { off: 'Off', read: 'Read', act: 'Act' };
const MODE_HELP: Record<SiteMode, string> = {
  off: 'Sessions cannot see or touch this site.',
  read: 'Sessions may read this site, but not click or type.',
  act: 'Sessions may read and act on this site (risky actions still ask you).',
};

async function setMode(t: TabInfo, mode: SiteMode | null): Promise<void> {
  siteMessage = '';
  const pattern = hostPattern(t.url);
  // The permission prompt must come straight from the click.
  if (mode !== null && mode !== 'off' && !t.hostGranted && pattern) {
    const ok = await askPermissions({ origins: [pattern] });
    if (!ok)
      siteMessage = 'Without access to this site, sessions cannot read it even in Read or Act.';
  }
  if (!t.origin) return;
  const res = await request({ op: 'setSiteMode', origin: t.origin, mode });
  if (!res.ok) siteMessage = res.message;
  await refreshTab();
}

function renderTab(): void {
  const section = byId('this-tab');
  const t = tab;
  const s = status;
  if (!t || !s?.paired) {
    section.hidden = true;
    return;
  }
  section.hidden = false;
  const card = byId('tab-card');
  if (!t.origin) {
    card.replaceChildren(
      h('div', { class: 'card muted' }, 'The browser does not let extensions into this page.'),
    );
    return;
  }
  const lease = s.leases.find((l) => l.lease === t.lease);
  const modes = h(
    'div',
    { class: 'modes', role: 'group', 'aria-label': `What sessions may do on ${t.origin}` },
    ...(['off', 'read', 'act'] as SiteMode[]).map((m) =>
      h(
        'button',
        {
          type: 'button',
          'data-key': `mode-${m}`,
          'aria-pressed': String(t.mode === m),
          // Raising a sensitive site to act happens on the desktop only.
          disabled: s.connection.state !== 'connected' || (m === 'act' && t.sensitive),
          onclick: () => void setMode(t, m),
        },
        MODE_LABELS[m],
      ),
    ),
  );
  const sensitive = t.sensitive
    ? h(
        'p',
        { class: 'help' },
        'A sensitive site (banking, payment or sign-in): sessions can act here only if you allow it in Mother Claude on the desktop.',
      )
    : null;
  const reset = t.ownEntry
    ? h(
        'button',
        {
          type: 'button',
          'data-key': 'mode-default',
          disabled: s.connection.state !== 'connected',
          onclick: () => void setMode(t, null),
        },
        'Use the default',
      )
    : null;
  const pattern = hostPattern(t.url);
  const access = t.hostGranted
    ? null
    : h(
        'div',
        { class: 'notice' },
        `Mother Claude cannot read ${t.origin} yet.`,
        h(
          'div',
          { class: 'actions' },
          pattern
            ? h(
                'button',
                {
                  type: 'button',
                  onclick: () => void askPermissions({ origins: [pattern] }).then(refreshTab),
                },
                `Allow on ${new URL(t.url).hostname}`,
              )
            : null,
          h(
            'button',
            {
              type: 'button',
              onclick: () => void askPermissions({ origins: ['<all_urls>'] }).then(refreshTab),
            },
            'Allow on all sites',
          ),
        ),
      );
  const membership = lease
    ? h('p', { class: 'muted' }, `In the tab group of ${lease.title}.`)
    : s.leases.length
      ? addToLease(t, s)
      : null;
  // The toolbar click granted this tab (activeTab): sessions may use it, if the user says so.
  const sharing =
    lease || !(t.activeGrant || t.hostGranted)
      ? null
      : t.shared
        ? h(
            'div',
            { class: 'actions' },
            h('span', { class: 'muted' }, 'Shared with sessions until you leave this site.'),
            h(
              'button',
              {
                type: 'button',
                'data-key': 'unshare',
                onclick: () => void request({ op: 'unshare' }).then(refreshTab),
              },
              'Stop sharing',
            ),
          )
        : h(
            'div',
            { class: 'actions' },
            h(
              'button',
              {
                type: 'button',
                'data-key': 'share',
                onclick: () => void request({ op: 'share', tabId: t.tabId }).then(refreshTab),
              },
              'Share this tab with sessions',
            ),
          );
  card.replaceChildren(
    h(
      'div',
      { class: 'card' },
      h('div', { class: 'title' }, t.origin),
      h('p', { class: 'help' }, MODE_HELP[t.mode]),
      h('div', { class: 'actions' }, modes, reset),
      sensitive,
      siteMessage ? h('p', { class: 'error', role: 'alert' }, siteMessage) : null,
      t.dialog
        ? h(
            'p',
            { class: 'notice' },
            `A ${t.dialog.kind} dialog is open here: “${t.dialog.message.slice(0, 200)}”`,
          )
        : null,
      access,
      sharing,
      membership,
    ),
  );
}

function addToLease(t: TabInfo, s: UiStatus): HTMLElement {
  const selectId = 'add-to-lease';
  const select = h(
    'select',
    { id: selectId },
    ...s.leases.map((l) => h('option', { value: l.lease }, l.title)),
  );
  return h(
    'div',
    {},
    h('label', { for: selectId }, 'Let a session use this tab'),
    select,
    h(
      'div',
      { class: 'actions' },
      h(
        'button',
        {
          type: 'button',
          onclick: () =>
            void request({ op: 'addToLease', tabId: t.tabId, lease: select.value }).then(
              refreshTab,
            ),
        },
        'Add to its tab group',
      ),
    ),
  );
}

async function refreshTab(): Promise<void> {
  try {
    const [active] = await chrome.tabs.query({
      active: true,
      ...(windowId !== undefined ? { windowId } : { lastFocusedWindow: true }),
    });
    if (active?.id === undefined) {
      tab = null;
    } else {
      const res = await request({ op: 'tabInfo', tabId: active.id });
      tab = res.ok ? (res['tab'] as TabInfo) : null;
    }
  } catch {
    tab = null;
  }
  keepFocus(renderTab);
}

function render(s: UiStatus): void {
  status = s;
  byId('version').textContent = `version ${s.extensionVersion}`;
  keepFocus(() => {
    renderConnection(s);
    renderPairing(s);
    renderLeases(s);
    renderPending(s);
  });
  byId('debugger-note').hidden = s.debuggerAttached === 0;
  void refreshTab();
}

async function start(): Promise<void> {
  try {
    windowId = (await chrome.windows.getCurrent()).id;
  } catch {
    windowId = undefined;
  }
  watchStatus(render);
  chrome.tabs.onActivated.addListener(() => void refreshTab());
  chrome.tabs.onUpdated.addListener((_id, info) => {
    if (info.url !== undefined || info.status === 'complete') void refreshTab();
  });
  // Countdowns and "x s ago".
  setInterval(() => {
    const s = status;
    if (!s) return;
    keepFocus(() => {
      renderConnection(s);
      if (s.leases.length) renderLeases(s);
    });
  }, 1000);
}

void start();
