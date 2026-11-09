// The side panel: connection, pairing, approvals waiting for an answer, the
// active tab and its site mode, connecting a Claude Code session that is
// already running to this tab (and telling it what to do), and the sessions
// working in this browser with their Stop button.

import type { PendingItem, SiteMode } from '../shared/protocol.ts';
import { hostPattern } from '../shared/site.ts';
import type { TabInfo, UiLease, UiSession, UiStatus } from '../shared/ui.ts';
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
/** How often the session list refreshes while the panel is open. */
const SESSIONS_EVERY_MS = 5_000;
/** A search box appears above this many sessions. */
const FILTER_ABOVE = 6;

interface Result {
  text: string;
  kind: 'ok' | 'error';
}

let status: UiStatus | null = null;
let tab: TabInfo | null = null;
let windowId: number | undefined;
let siteMessage = '';
const answerErrors = new Map<string, string>();

// Connecting a session.
let sessions: UiSession[] | null = null;
let sessionsError = '';
let picked: string | null = null;
/** The whole list shows (else only the chosen session, with "Choose another"). */
let choosing = true;
let filter = '';
let connectBusy = false;
let connectResult: Result | null = null;

// Telling a session that works here.
const tellOpen = new Set<string>();
const tellDrafts = new Map<string, string>();
const tellResults = new Map<string, Result>();
const tellBusy = new Set<string>();
let leaseSignature = '';

// ─────────────────────────────────────────────────────────── connection

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
  let line = c.state === 'connected' ? 'Connected' : (CONNECTION_TEXT[c.state] ?? c.state);
  if (c.state === 'disconnected' && c.retryAt) {
    line = `Not reachable — retry in ${Math.max(0, Math.round((c.retryAt - Date.now()) / 1000))} s`;
  }
  if (text) text.textContent = line;
  box.title = c.state === 'connected' && c.appVersion ? `Mother Claude ${c.appVersion}` : line;
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

// ─────────────────────────────────────────────────── sessions working here

function deliveredText(d: unknown): string {
  if (d === 'queued') return 'It gets your message at its next step.';
  if (d === 'relayed') return 'Your message was delivered.';
  return 'It has your message.';
}

async function sendTell(l: UiLease): Promise<void> {
  const message = (tellDrafts.get(l.lease) ?? '').trim();
  if (!message) {
    tellResults.set(l.lease, { kind: 'error', text: 'Write what it should do.' });
    renderLeasesNow();
    return;
  }
  tellBusy.add(l.lease);
  tellResults.delete(l.lease);
  renderLeasesNow();
  const res = await request({ op: 'tell', lease: l.lease, message });
  tellBusy.delete(l.lease);
  if (res.ok) {
    tellDrafts.delete(l.lease);
    tellResults.set(l.lease, { kind: 'ok', text: deliveredText(res['delivery']) });
  } else {
    tellResults.set(l.lease, { kind: 'error', text: res.message });
  }
  renderLeasesNow();
}

function tellComposer(l: UiLease): HTMLElement {
  const id = `tell-text-${l.lease}`;
  const box = h('textarea', {
    id,
    'data-key': id,
    rows: 3,
    placeholder: 'What should it do now?',
  });
  box.value = tellDrafts.get(l.lease) ?? '';
  box.addEventListener('input', () => tellDrafts.set(l.lease, box.value));
  const busy = tellBusy.has(l.lease);
  const result = tellResults.get(l.lease);
  return h(
    'div',
    {},
    h('label', { for: id }, 'Tell it what to do'),
    box,
    h(
      'div',
      { class: 'actions' },
      h(
        'button',
        {
          type: 'button',
          class: 'primary',
          'data-key': `send-${l.lease}`,
          disabled: busy,
          onclick: () => void sendTell(l),
        },
        busy ? 'Sending…' : 'Send',
      ),
    ),
    result
      ? h(
          'p',
          { class: result.kind === 'error' ? 'error' : 'success', role: 'status' },
          result.text,
        )
      : null,
  );
}

function leaseCard(l: UiLease): HTMLElement {
  const stopped = l.stoppedUntil > Date.now();
  const activity = stopped
    ? `Stopped — its commands are refused for ${Math.ceil((l.stoppedUntil - Date.now()) / 1000)} s`
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
  const session = sessions?.find((s) => s.id === l.session);
  // Unknown (the list has not come yet): let the desktop decide.
  const canTell = session ? session.canTell : true;
  const open = tellOpen.has(l.lease);
  const toggle = h(
    'button',
    {
      type: 'button',
      'data-key': `tell-${l.lease}`,
      'aria-expanded': String(open),
      onclick: () => {
        if (tellOpen.has(l.lease)) tellOpen.delete(l.lease);
        else tellOpen.add(l.lease);
        renderLeasesNow();
      },
    },
    open ? 'Close' : 'Tell it what to do',
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
        h('div', { class: 'muted small' }, activity),
      ),
      button,
    ),
    h(
      'p',
      { class: 'muted small', style: 'margin:8px 0 0' },
      `${l.tabs} tab${l.tabs === 1 ? '' : 's'} in its group`,
    ),
    canTell
      ? h('div', { class: 'actions' }, toggle)
      : session?.whyNot
        ? h('p', { class: 'note' }, session.whyNot)
        : null,
    canTell && open ? tellComposer(l) : null,
  );
}

async function act(req: Parameters<typeof request>[0]): Promise<void> {
  const res = await request(req);
  if (!res.ok) byId('connection-error').textContent = res.message;
}

function renderLeasesNow(): void {
  const s = status;
  if (!s) return;
  leaseSignature = '';
  keepFocus(() => renderLeases(s));
}

function renderLeases(s: UiStatus): void {
  const section = byId('sessions');
  section.hidden = !s.paired || !s.leases.length;
  // Redraw only when something shown changed: a draft being typed stays put.
  const signature = JSON.stringify([
    s.leases,
    sessions?.map((x) => [x.id, x.canTell, x.whyNot]),
    [...tellOpen],
    [...tellBusy],
    [...tellResults],
  ]);
  if (signature === leaseSignature) return;
  leaseSignature = signature;
  byId('session-list').replaceChildren(...s.leases.map(leaseCard));
}

// ──────────────────────────────────────────────────────────── approvals

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
    { class: 'card attention', 'data-pending': p.id },
    h(
      'div',
      { class: 'row' },
      h(
        'div',
        { class: 'grow title', style: 'white-space:normal' },
        p.summary ?? 'A browser action waits for your answer',
      ),
      p.dangerous ? h('span', { class: 'badge danger' }, 'Risky') : null,
    ),
    p.detail
      ? h(
          'p',
          { class: 'small', style: 'white-space:pre-wrap;margin:8px 0 0' },
          p.detail.slice(0, 600),
        )
      : null,
    h(
      'p',
      { class: 'muted small', style: 'margin:8px 0 0' },
      [p.session, created].filter(Boolean).join(' · '),
    ),
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

// ───────────────────────────────────────────────────────────── this tab

const MODE_LABELS: Record<SiteMode, string> = { off: 'Off', read: 'Read', act: 'Act' };
const MODE_HELP: Record<SiteMode, string> = {
  off: 'Sessions cannot see or touch this site.',
  read: 'Sessions may read this site, but not click or type.',
  act: 'Sessions may read and act on this site. Risky steps still ask you.',
};

async function setMode(t: TabInfo, mode: SiteMode | null): Promise<void> {
  siteMessage = '';
  const pattern = hostPattern(t.url);
  // The permission prompt must come straight from the click.
  if (mode !== null && mode !== 'off' && !t.hostGranted && pattern) {
    const asked = await askPermissions({ origins: [pattern] });
    if (!asked.ok)
      siteMessage =
        asked.error || 'Without access to this site, sessions cannot read it even in Read or Act.';
  }
  if (!t.origin) return;
  const res = await request({ op: 'setSiteMode', origin: t.origin, mode });
  if (!res.ok) siteMessage = res.message;
  await refreshTab();
}

/** "Allow on …": Chrome's prompt, straight from the click. */
async function allowSites(origins: string[]): Promise<void> {
  const asked = await askPermissions({ origins });
  siteMessage = asked.error;
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
      h(
        'div',
        { class: 'card muted' },
        'The browser does not let extensions into this page. Open a website to work with it.',
      ),
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
        { class: 'note warn' },
        'A sensitive site (banking, payment or sign-in): sessions can act here only if you allow it in Mother Claude on the desktop.',
      )
    : null;
  const reset = t.ownEntry
    ? h(
        'button',
        {
          type: 'button',
          class: 'link',
          'data-key': 'mode-default',
          disabled: s.connection.state !== 'connected',
          onclick: () => void setMode(t, null),
        },
        'Use the default for this site',
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
                  onclick: () => void allowSites([pattern]),
                },
                `Allow on ${new URL(t.url).hostname}`,
              )
            : null,
          h(
            'button',
            {
              type: 'button',
              onclick: () => void allowSites(['<all_urls>']),
            },
            'Allow on all sites',
          ),
        ),
      );
  const membership = lease
    ? h('p', { class: 'note' }, `In the tab group of “${lease.title}”.`)
    : null;
  card.replaceChildren(
    h(
      'div',
      { class: 'card' },
      h('div', { class: 'site' }, new URL(t.url).hostname || t.origin),
      t.title ? h('p', { class: 'muted small', style: 'margin:2px 0 12px' }, t.title) : null,
      modes,
      h('p', { class: 'help', style: 'margin-bottom:0' }, MODE_HELP[t.mode]),
      reset ? h('div', { class: 'actions', style: 'margin-top:6px' }, reset) : null,
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
      membership,
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
  renderConnect();
}

// ─────────────────────────────────────────────── connect a session here

let pickerBox: HTMLElement | null = null;
let filterBox: HTMLInputElement | null = null;
let askBox: HTMLTextAreaElement | null = null;
let connectButton: HTMLButtonElement | null = null;
let connectNote: HTMLElement | null = null;
let connectOut: HTMLElement | null = null;

const STATUS_TEXT: Record<string, string> = {
  working: 'Working',
  waiting: 'Waiting for you',
  idle: 'Idle',
  background: 'Background work',
  starting: 'Starting',
};

/** Built once: the message box keeps what is typed while the list refreshes. */
function buildConnectCard(): void {
  filterBox = h('input', {
    type: 'search',
    placeholder: 'Find a session',
    'aria-label': 'Find a session',
    'data-key': 'filter',
  });
  filterBox.hidden = true;
  filterBox.addEventListener('input', () => {
    filter = filterBox?.value ?? '';
    renderPicker();
  });
  pickerBox = h('div', { class: 'picker', role: 'radiogroup', 'aria-labelledby': 'connect-title' });
  askBox = h('textarea', {
    id: 'ask',
    'data-key': 'ask',
    rows: 4,
    placeholder: 'For example: read this page and list every plan with its price.',
  });
  askBox.addEventListener('input', updateConnectButton);
  connectButton = h(
    'button',
    {
      type: 'button',
      class: 'primary wide',
      'data-key': 'connect',
      onclick: () => void doConnect(),
    },
    'Connect',
  );
  connectNote = h('p', { class: 'note' });
  connectOut = h('p', { role: 'status', 'aria-live': 'polite' });
  byId('connect-card').replaceChildren(
    h(
      'p',
      { class: 'help', style: 'margin-top:0' },
      'Choose a Claude Code session that is already running. It keeps its conversation and context, and gets this tab in its own tab group.',
    ),
    filterBox,
    pickerBox,
    h('label', { for: 'ask' }, 'What should it do here?'),
    askBox,
    h('div', { class: 'actions' }, connectButton),
    connectNote,
    connectOut,
  );
}

function pickedSession(): UiSession | undefined {
  return sessions?.find((s) => s.id === picked);
}

function sessionMatches(s: UiSession, q: string): boolean {
  const words = q.toLowerCase().split(/\s+/).filter(Boolean);
  const hay = `${s.name} ${s.project} ${s.cwd} ${s.surface}`.toLowerCase();
  return words.every((w) => hay.includes(w));
}

function sessionOption(s: UiSession): HTMLElement {
  const inputId = `pick-${s.id}`;
  const chosen = s.id === picked;
  const input = h('input', {
    type: 'radio',
    name: 'session',
    id: inputId,
    value: s.id,
    'data-key': inputId,
    checked: chosen,
  });
  input.addEventListener('change', () => {
    picked = s.id;
    choosing = false;
    connectResult = null;
    renderPicker();
  });
  const meta = [s.project, s.surface].filter(Boolean).join(' · ');
  return h(
    'label',
    { class: 'pick', for: inputId },
    input,
    h('span', { class: 'name' }, s.name),
    h('span', { class: 'meta', title: s.cwd }, meta),
    h(
      'span',
      { class: 'badges' },
      h('span', { class: `badge state ${s.status}` }, STATUS_TEXT[s.status] ?? s.status),
      s.lease ? h('span', { class: 'badge accent' }, 'In this browser') : null,
      !s.hasTools ? h('span', { class: 'badge warn' }, 'No browser tools') : null,
      !s.canTell ? h('span', { class: 'badge' }, 'Can’t message') : null,
      s.lastActivityAt ? h('span', { class: 'badge' }, ago(s.lastActivityAt)) : null,
    ),
    chosen && !s.hasTools
      ? h(
          'span',
          { class: 'note warn' },
          'Type /reload-plugins in this session first: it started before Mother Claude gave sessions the browser tools.',
        )
      : null,
    chosen && !s.canTell && s.whyNot ? h('span', { class: 'note' }, s.whyNot) : null,
  );
}

function renderPicker(): void {
  if (!pickerBox || !filterBox) return;
  const chosen = pickedSession();
  if (sessions === null) {
    filterBox.hidden = true;
    pickerBox.replaceChildren(h('p', { class: 'empty' }, sessionsError || 'Looking for sessions…'));
  } else if (!sessions.length) {
    filterBox.hidden = true;
    pickerBox.replaceChildren(
      h(
        'p',
        { class: 'empty' },
        'No Claude Code session is running on this computer. Start one in VS Code, a terminal or Mother Claude, then press Refresh.',
      ),
    );
  } else if (chosen && !choosing) {
    filterBox.hidden = true;
    const others = sessions.length - 1;
    const again = h(
      'button',
      {
        type: 'button',
        class: 'link',
        'data-key': 'choose-again',
        onclick: () => {
          choosing = true;
          renderPicker();
        },
      },
      `Choose another session (${others} more)`,
    );
    pickerBox.replaceChildren(sessionOption(chosen), ...(others > 0 ? [h('div', {}, again)] : []));
  } else {
    filterBox.hidden = sessions.length <= FILTER_ABOVE;
    const shown = sessions.filter((s) => s.id === picked || sessionMatches(s, filter));
    pickerBox.replaceChildren(...shown.map(sessionOption));
    if (!shown.length) pickerBox.append(h('p', { class: 'empty' }, 'No session matches.'));
  }
  updateConnectButton();
}

function updateConnectButton(): void {
  if (!connectButton || !connectNote || !askBox || !connectOut) return;
  const s = pickedSession();
  const t = tab;
  const message = askBox.value.trim();
  const online = status?.connection.state === 'connected';
  let why = '';
  if (!online) why = 'Connect to Mother Claude first.';
  else if (!t?.origin) why = 'Open a website in this tab.';
  else if (t.mode === 'off') why = 'This site is Off: choose Read or Act above first.';
  else if (!t.hostGranted && !t.activeGrant)
    why = 'Let Mother Claude into this site first (above).';
  else if (!s) why = 'Choose a session.';
  const inGroup = !!(s?.lease && t?.lease && s.lease === t.lease);
  const willTell = !!(s && message && s.canTell);
  connectButton.textContent = connectBusy
    ? 'Working…'
    : inGroup
      ? 'Send'
      : willTell
        ? 'Connect and send'
        : 'Connect';
  connectButton.disabled = connectBusy || !!why || (inGroup && !willTell);
  connectNote.textContent =
    why ||
    (inGroup && !message ? 'This tab already works for it: write what it should do.' : '') ||
    (s && message && !s.canTell
      ? 'Mother Claude cannot send this session messages: connect, then ask it yourself to use the browser.'
      : '');
  connectNote.hidden = !connectNote.textContent;
  connectOut.className = connectResult?.kind === 'error' ? 'error' : 'success';
  connectOut.textContent = connectResult?.text ?? '';
  connectOut.hidden = !connectResult;
}

async function doConnect(): Promise<void> {
  const s = pickedSession();
  const t = tab;
  if (!s || !t || !askBox) return;
  const message = askBox.value.trim();
  connectBusy = true;
  connectResult = null;
  updateConnectButton();
  if (s.lease && t.lease === s.lease) {
    const res = await request({ op: 'tell', lease: s.lease, message });
    connectBusy = false;
    if (res.ok) {
      connectResult = { kind: 'ok', text: deliveredText(res['delivery']) };
      askBox.value = '';
    } else {
      connectResult = { kind: 'error', text: res.message };
    }
  } else {
    const res = await request({
      op: 'connect',
      session: s.id,
      tabId: t.tabId,
      ...(message && s.canTell ? { message } : {}),
    });
    connectBusy = false;
    if (!res.ok) {
      connectResult = { kind: 'error', text: res.message };
    } else {
      const done = `Connected: this tab is in the tab group of “${s.name}”.`;
      if (res['told'] === true) {
        connectResult = { kind: 'ok', text: `${done} ${deliveredText(res['delivery'])}` };
        askBox.value = '';
      } else if (typeof res['tellError'] === 'string') {
        connectResult = {
          kind: 'error',
          text: `${done} But your message did not go: ${res['tellError']}`,
        };
      } else {
        connectResult = { kind: 'ok', text: done };
      }
    }
  }
  updateConnectButton();
  await loadSessions();
  await refreshTab();
}

function renderConnect(): void {
  const s = status;
  const show = !!s?.paired && s.connection.state === 'connected' && !!tab?.origin;
  byId('connect').hidden = !show;
  if (!show) return;
  // A tab already in a session's group: that session.
  const here = s.leases.find((l) => l.lease === tab?.lease);
  if (here && sessions?.some((x) => x.id === here.session) && picked !== here.session) {
    picked = here.session;
    choosing = false;
  } else if (!picked && sessions?.length === 1) {
    picked = sessions[0]?.id ?? null;
    choosing = false;
  }
  keepFocus(renderPicker);
}

async function loadSessions(): Promise<void> {
  const s = status;
  if (!s?.paired || s.connection.state !== 'connected') {
    sessions = null;
    sessionsError = '';
    return;
  }
  const res = await request({ op: 'sessions' });
  if (res.ok) {
    sessions = (res['sessions'] as UiSession[] | undefined) ?? [];
    sessionsError = '';
    if (picked && !sessions.some((x) => x.id === picked)) picked = null;
  } else {
    sessions = null;
    sessionsError = res.message;
  }
  renderConnect();
  if (status) keepFocus(() => status && renderLeases(status));
}

// ─────────────────────────────────────────────────────────────── start

function render(s: UiStatus): void {
  const wasOnline = status?.connection.state === 'connected';
  status = s;
  byId('version').textContent = `version ${s.extensionVersion}`;
  keepFocus(() => {
    renderConnection(s);
    renderPairing(s);
    renderLeases(s);
    renderPending(s);
  });
  byId('debugger-note').hidden = s.debuggerAttached === 0;
  if (!wasOnline && s.connection.state === 'connected') void loadSessions();
  void refreshTab();
}

async function start(): Promise<void> {
  try {
    windowId = (await chrome.windows.getCurrent()).id;
  } catch {
    windowId = undefined;
  }
  buildConnectCard();
  byId('refresh-sessions').addEventListener('click', () => void loadSessions());
  watchStatus(render);
  chrome.tabs.onActivated.addListener(() => {
    connectResult = null;
    void refreshTab();
  });
  chrome.tabs.onUpdated.addListener((_id, info) => {
    if (info.url !== undefined || info.status === 'complete') void refreshTab();
  });
  // Countdowns and "x s ago".
  setInterval(() => {
    const s = status;
    if (!s) return;
    keepFocus(() => {
      renderConnection(s);
      if (s.leases.some((l) => l.stoppedUntil > Date.now())) renderLeasesNow();
    });
  }, 1000);
  setInterval(() => {
    if (document.visibilityState === 'visible') void loadSessions();
  }, SESSIONS_EVERY_MS);
}

void start();
