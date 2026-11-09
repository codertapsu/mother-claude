// Options: where the app is, pairing, and every optional permission with a
// button to ask for it or give it back.

import { hostPattern } from '../shared/site.ts';
import type { UiStatus } from '../shared/ui.ts';
import {
  CONNECTION_TEXT,
  askPermissions,
  byId,
  dropPermissions,
  h,
  keepFocus,
  pairingForm,
  request,
  watchStatus,
} from './common.ts';

const PERMISSIONS: { name: string; label: string; why: string }[] = [
  {
    name: 'debugger',
    label: 'Debugger',
    why: 'Trusted clicks and keys for sites that ignore scripted ones, screenshots of tabs in the background, the console, network requests, cookies, JavaScript dialogs, and eval. Chrome shows a bar while it is in use.',
  },
  {
    name: 'webNavigation',
    label: 'Navigation events',
    why: 'Notices page loads earlier and reports why a page failed to load.',
  },
  {
    name: 'downloads',
    label: 'Downloads',
    why: 'Tells the session when a page it works on starts a download (the file name only).',
  },
];

let formFilled = false;

/**
 * Grant or remove one permission. The debugger is not optional to Chrome (it
 * gave it at install), so its Grant is the extension's own switch; the
 * others are Chrome's prompt, which must come straight from the click.
 */
async function setPermission(name: string, grant: boolean): Promise<void> {
  const out = byId('permission-error');
  if (name === 'debugger') {
    out.textContent = '';
    const res = await request({ op: 'setDebugger', granted: grant });
    if (!res.ok) out.textContent = res.message;
    return;
  }
  const p = { permissions: [name] as chrome.runtime.ManifestPermission[] };
  const done = grant ? await askPermissions(p) : await dropPermissions(p);
  out.textContent = done.error ? `Chrome did not change ${name}: ${done.error}` : '';
}

function renderConnection(s: UiStatus): void {
  const box = byId('connection');
  box.querySelector('.dot')?.classList.toggle('on', s.connection.state === 'connected');
  const text = box.querySelector('.text');
  if (text) {
    text.textContent =
      (CONNECTION_TEXT[s.connection.state] ?? s.connection.state) +
      (s.connection.appVersion && s.connection.state === 'connected'
        ? ` ${s.connection.appVersion}`
        : '');
  }
  byId('connection-error').textContent =
    s.connection.state === 'connected' ? '' : s.connection.error;
  if (!formFilled) {
    byId<HTMLInputElement>('host').value = s.connection.host;
    byId<HTMLInputElement>('port').value = String(s.connection.port);
    formFilled = true;
  }
}

function renderPairing(s: UiStatus): void {
  const slot = byId('pairing');
  if (s.paired && s.connection.state !== 'revoked') {
    slot.replaceChildren(
      h(
        'p',
        {},
        `Paired as “${s.browserName}”`,
        s.browserId ? h('span', { class: 'muted' }, ` (${s.browserId})`) : null,
        '.',
      ),
      h(
        'div',
        { class: 'actions' },
        h(
          'button',
          {
            type: 'button',
            'data-key': 'forget',
            onclick: () => {
              if (
                confirm(
                  'Forget this pairing? Sessions stop working in this browser until you pair again.',
                )
              ) {
                void request({ op: 'forget' });
              }
            },
          },
          'Forget this pairing',
        ),
        h(
          'button',
          {
            type: 'button',
            'data-key': 'reconnect',
            onclick: () => void request({ op: 'reconnect' }),
          },
          'Reconnect now',
        ),
      ),
    );
    return;
  }
  if (!slot.querySelector('form')) {
    slot.replaceChildren(pairingForm(s.browserName, () => undefined));
  }
}

function renderPermissions(s: UiStatus): void {
  const granted = new Set(s.granted);
  byId('permission-rows').replaceChildren(
    ...PERMISSIONS.map((p) => {
      const has = granted.has(p.name);
      return h(
        'tr',
        {},
        h('th', { scope: 'row' }, p.label),
        h('td', { class: 'muted' }, p.why),
        h(
          'td',
          {},
          h('div', {}, has ? 'Granted' : 'Not granted'),
          h(
            'button',
            {
              type: 'button',
              'data-key': `perm-${p.name}`,
              onclick: () => void setPermission(p.name, !has),
            },
            has ? 'Remove' : 'Grant',
          ),
        ),
      );
    }),
  );
  const origins = s.granted.filter((g) => g.includes('://') || g === '<all_urls>');
  byId('sites').replaceChildren(
    ...(origins.length
      ? origins.map((o) =>
          h(
            'li',
            {},
            o === '<all_urls>' ? 'All sites' : o,
            ' ',
            h(
              'button',
              {
                type: 'button',
                'data-key': `site-${o}`,
                onclick: () => void allowSites([o], false),
              },
              'Remove',
            ),
          ),
        )
      : [h('li', { class: 'muted' }, 'No site yet.')]),
  );
  byId('about').textContent = `Extension ${s.extensionVersion} · id ${s.extensionId}`;
}

/** Let the extension into sites, or out of them; Chrome's reason when it refuses. */
async function allowSites(origins: string[], allow: boolean): Promise<void> {
  const done = allow ? await askPermissions({ origins }) : await dropPermissions({ origins });
  byId('site-error').textContent = done.error;
}

function render(s: UiStatus): void {
  keepFocus(() => {
    renderConnection(s);
    renderPairing(s);
    renderPermissions(s);
  });
}

function start(): void {
  watchStatus(render);
  byId<HTMLFormElement>('desktop-form').addEventListener('submit', (e) => {
    e.preventDefault();
    const host = byId<HTMLInputElement>('host').value;
    const port = Number(byId<HTMLInputElement>('port').value);
    void request({ op: 'setConfig', host, port }).then((res) => {
      byId('desktop-error').textContent = res.ok ? '' : res.message;
    });
  });
  byId<HTMLFormElement>('site-form').addEventListener('submit', (e) => {
    e.preventDefault();
    const raw = byId<HTMLInputElement>('site').value.trim();
    const url = /^[a-z]+:\/\//i.test(raw) ? raw : `https://${raw}`;
    const pattern = hostPattern(url);
    if (!pattern) {
      byId('site-error').textContent = 'Enter a web address, like https://github.com.';
      return;
    }
    byId('site-error').textContent = '';
    void allowSites([pattern], true);
  });
  byId('all-sites').addEventListener('click', () => void allowSites(['<all_urls>'], true));
}

start();
