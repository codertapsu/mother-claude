// Shared by the side panel, the options page and the lease placeholder: talk
// to the service worker, build DOM safely (text only, never HTML strings from
// the desktop or a page).

import { UI_PORT, type UiRequest, type UiResponse, type UiStatus } from '../shared/ui.ts';

export async function request(req: UiRequest): Promise<UiResponse> {
  try {
    const res = (await chrome.runtime.sendMessage({ type: 'mc.ui', req })) as
      UiResponse | undefined;
    return res ?? { ok: false, code: 'no_answer', message: 'The extension did not answer.' };
  } catch (e) {
    return { ok: false, code: 'error', message: e instanceof Error ? e.message : String(e) };
  }
}

/** Live status from the service worker; reconnects when the worker restarts. */
export function watchStatus(onStatus: (s: UiStatus) => void): void {
  let delay = 200;
  const connect = (): void => {
    let port: chrome.runtime.Port;
    try {
      port = chrome.runtime.connect({ name: UI_PORT });
    } catch {
      setTimeout(connect, (delay = Math.min(delay * 2, 5000)));
      return;
    }
    port.onMessage.addListener((msg: { type?: string; status?: UiStatus }) => {
      delay = 200;
      if (msg.type === 'status' && msg.status) onStatus(msg.status);
    });
    port.onDisconnect.addListener(() => setTimeout(connect, delay));
  };
  connect();
}

type Child = Node | string | null | undefined | false;

/** A small element builder: attributes, then children (strings become text). */
export function h<K extends keyof HTMLElementTagNameMap>(
  tag: K,
  attrs: Record<string, string | boolean | number | undefined | ((e: Event) => void)> = {},
  ...children: Child[]
): HTMLElementTagNameMap[K] {
  const el = document.createElement(tag);
  for (const [k, v] of Object.entries(attrs)) {
    if (v === undefined || v === false) continue;
    if (typeof v === 'function') el.addEventListener(k.replace(/^on/, ''), v);
    else if (v === true) el.setAttribute(k, '');
    else if (k === 'class') el.className = String(v);
    else el.setAttribute(k, String(v));
  }
  for (const c of children) {
    if (c === null || c === undefined || c === false) continue;
    el.append(typeof c === 'string' ? document.createTextNode(c) : c);
  }
  return el;
}

/**
 * Re-render without losing keyboard focus: an element with the same
 * `data-key` as the focused one gets the focus back.
 */
export function keepFocus(render: () => void): void {
  const active = document.activeElement;
  const key = active?.getAttribute('data-key');
  // Where the cursor was in a text field, to put it back.
  const sel = textField(active) ? [active.selectionStart, active.selectionEnd] : null;
  render();
  if (!key) return;
  const again = document.querySelector<HTMLElement>(`[data-key="${CSS.escape(key)}"]`);
  if (again && document.activeElement !== again) {
    again.focus();
    if (sel && textField(again)) again.setSelectionRange(sel[0] ?? null, sel[1] ?? null);
  }
}

function textField(el: Element | null): el is HTMLInputElement | HTMLTextAreaElement {
  if (el instanceof HTMLTextAreaElement) return true;
  return el instanceof HTMLInputElement && ['text', 'search', 'url', 'email', ''].includes(el.type);
}

export function byId<T extends HTMLElement = HTMLElement>(id: string): T {
  const el = document.getElementById(id);
  if (!el) throw new Error(`#${id} missing`);
  return el as T;
}

export function ago(ms: number): string {
  const s = Math.max(0, Math.round((Date.now() - ms) / 1000));
  if (s < 5) return 'just now';
  if (s < 60) return `${s} s ago`;
  const m = Math.round(s / 60);
  if (m < 60) return `${m} min ago`;
  return `${Math.round(m / 60)} h ago`;
}

export const COLOR_HEX: Record<string, string> = {
  grey: '#5f6368',
  blue: '#1a73e8',
  red: '#d93025',
  yellow: '#f9ab00',
  green: '#188038',
  pink: '#d01884',
  purple: '#9334e6',
  cyan: '#007b83',
  orange: '#fa903e',
};

export const CONNECTION_TEXT: Record<string, string> = {
  unpaired: 'Not paired with Mother Claude',
  connecting: 'Connecting to Mother Claude…',
  connected: 'Connected to Mother Claude',
  disconnected: 'Mother Claude is not reachable',
  revoked: 'This browser was removed in Mother Claude',
  auth_failed: 'Mother Claude did not accept this browser',
};

/**
 * What Chrome did with a permission request: `ok` when it granted (or
 * removed) it; otherwise `error` is Chrome's reason when it refused the
 * request itself, and empty when the user said no in its prompt.
 */
export interface PermissionOutcome {
  ok: boolean;
  error: string;
}

async function permissionCall(call: () => Promise<boolean>): Promise<PermissionOutcome> {
  let outcome: PermissionOutcome;
  try {
    outcome = { ok: await call(), error: '' };
  } catch (e) {
    outcome = { ok: false, error: e instanceof Error ? e.message : String(e) };
  }
  await request({ op: 'permissionsChanged' });
  return outcome;
}

/** Ask for optional permissions (needs the click that called it) and tell the worker. */
export function askPermissions(p: chrome.permissions.Permissions): Promise<PermissionOutcome> {
  // Called before anything awaits, so the request still has the click.
  return permissionCall(() => chrome.permissions.request(p));
}

export function dropPermissions(p: chrome.permissions.Permissions): Promise<PermissionOutcome> {
  return permissionCall(() => chrome.permissions.remove(p));
}

/** Pairing form shared by the side panel and the options page. */
export function pairingForm(defaultName: string, onDone: () => void): HTMLFormElement {
  const codeId = `code-${Math.random().toString(36).slice(2, 8)}`;
  const nameId = `name-${Math.random().toString(36).slice(2, 8)}`;
  const code = h('input', {
    id: codeId,
    name: 'code',
    autocomplete: 'off',
    autocapitalize: 'characters',
    spellcheck: 'false',
    placeholder: 'ABCD-EFGH',
    maxlength: 16,
    required: true,
    'aria-describedby': `${codeId}-help`,
  });
  const name = h('input', { id: nameId, name: 'name', autocomplete: 'off', required: true });
  name.value = defaultName;
  const error = h('p', { class: 'error', role: 'alert' });
  const button = h('button', { type: 'submit', class: 'primary' }, 'Pair');
  const form = h(
    'form',
    { class: 'pair', novalidate: true },
    h('label', { for: codeId }, 'Pairing code'),
    code,
    h(
      'p',
      { id: `${codeId}-help`, class: 'help' },
      'In Mother Claude on this computer: Settings › Browser › Pair a browser.',
    ),
    h('label', { for: nameId }, 'Name of this browser'),
    name,
    button,
    error,
  );
  form.addEventListener('submit', (e) => {
    e.preventDefault();
    error.textContent = '';
    button.disabled = true;
    button.textContent = 'Pairing…';
    void request({ op: 'pair', code: code.value, name: name.value }).then((res) => {
      button.disabled = false;
      button.textContent = 'Pair';
      if (res.ok) {
        code.value = '';
        onDone();
      } else {
        error.textContent = res.message;
        code.focus();
      }
    });
  });
  return form;
}
