import { ComponentFixture, TestBed } from '@angular/core/testing';
import { provideRouter } from '@angular/router';

import { BrowserOverview } from '../../core/browser';
import { FakeBackend, asDesktop, flush, useFakeServer } from '../../core/mock/fake-server.testing';
import { setActiveMock } from '../../core/mock/mock-api';
import { RealtimeService } from '../../core/realtime.service';
import { BrowserComponent } from './browser.component';

const text = (n: Element | null | undefined) => (n?.textContent ?? '').replace(/\s+/g, ' ').trim();

const overview = (): BrowserOverview => ({
  browsers: [
    {
      id: 'brw_1',
      name: 'Chrome on MacBook',
      browser: { name: 'Chrome', version: '156.0' },
      extensionVersion: '0.1.0',
      origin: 'chrome-extension://abcdefghijklmnopabcdefghijklmnop',
      createdAt: Date.now() - 60_000,
      revoked: false,
      connected: true,
    },
  ],
  settings: {
    defaultMode: 'off',
    screenshots: true,
    actionsPerMinute: 30,
    pinnedBrowser: null,
    uploadFolders: [],
  },
  sites: [
    { origin: '*', mode: 'off' },
    { origin: 'https://github.com', mode: 'act' },
    { origin: 'https://www.paypal.com', mode: 'read', sensitive: 'payment' },
  ],
  sensitive: [{ domain: 'paypal.com', category: 'payment' }],
  grants: [],
  leases: [
    {
      id: 'l_1',
      session: 's1',
      title: 'Docs · website',
      browser: 'brw_1',
      color: 'blue',
      openedAt: Date.now() - 60_000,
      lastUsed: Date.now() - 5_000,
      connected: true,
    },
  ],
  stops: [],
  approvals: [
    {
      id: 'req-b1',
      browser: 'brw_1',
      lease: 'l_1',
      session: 's1',
      summary: 'Run a script on github.com',
      detail: 'It runs a script in the page.',
      createdAt: Date.now() - 1_000,
    },
  ],
  pairing: { liveCodes: 0 },
  endpoint: 'ws://127.0.0.1:6725/ws/browser',
});

describe('BrowserComponent', () => {
  let backend: FakeBackend;
  let fixture: ComponentFixture<BrowserComponent>;
  let state: BrowserOverview;

  beforeEach(() => {
    backend = new FakeBackend();
    state = overview();
    backend.routes.set('/browser', () => ({ status: 200, body: state }));
    backend.routes.set('/browser/activity', () => ({
      status: 200,
      body: {
        entries: [
          {
            at: Date.now() - 2_000,
            session: 's1',
            tool: 'browser_click',
            action: 'click',
            page: 'https://github.com/acme/app',
            outcome: 'ok',
            approved: true,
          },
        ],
      },
    }));
    backend.routes.set('/browser/sites', (_m, body) => {
      const b = body as { origin: string; mode: string | null };
      state = {
        ...state,
        sites: state.sites.map((s) =>
          s.origin === b.origin ? { ...s, mode: (b.mode ?? 'off') as 'off' } : s,
        ),
      };
      return { status: 200, body: state };
    });
    backend.routes.set('/browser/pair', () => ({
      status: 201,
      body: {
        code: 'K7QD8F3B',
        display: 'K7QD-8F3B',
        expiresAt: Date.now() + 300_000,
        ttlSec: 300,
      },
    }));
    useFakeServer(backend, [provideRouter([])]);
  });

  afterEach(() => {
    fixture?.destroy();
    setActiveMock(null);
  });

  const render = async () => {
    fixture = TestBed.createComponent(BrowserComponent);
    fixture.detectChanges();
    await flush();
    fixture.detectChanges();
    return fixture.nativeElement as HTMLElement;
  };

  it('shows browsers, sessions, site modes, activity and waiting approvals', async () => {
    const el = await render();
    expect(text(el.querySelector('.browsers'))).toContain('Chrome on MacBook');
    expect(text(el.querySelector('.browsers'))).toContain('connected');
    expect(el.querySelector('.dot.on')).not.toBeNull();
    expect(text(el.querySelector('.lease'))).toContain('Docs · website');
    const sites = [...el.querySelectorAll('.site')].map(text);
    expect(sites[0]).toContain('Every other site');
    expect(sites.some((s) => s.includes('https://github.com'))).toBe(true);
    expect(sites.some((s) => s.includes('payment'))).toBe(true);
    expect(text(el.querySelector('.activity'))).toContain('Clicked · github.com/acme/app');
    expect(text(el.querySelector('.activity'))).toContain('approved by you');
    // The approval renders as the shared pending card.
    const card = el.querySelector('.approvals mc-pending-card');
    expect(card).not.toBeNull();
    expect(text(card)).toContain('Run a script on github.com');
    // The Inbox lists them too, with everything else waiting.
    const also = el.querySelector<HTMLAnchorElement>('.approvals .also a');
    expect(text(also)).toBe('Also in the Inbox, with everything else waiting ›');
    expect(also?.getAttribute('href')).toBe('/inbox');
  });

  it('on a phone without the dangerous grant: no pairing, act locked where the desktop decides', async () => {
    const el = await render();
    expect(el.querySelector('.pair .btn.primary')).toBeNull();
    expect(text(el.querySelector('.pair'))).toContain(
      'Pair a browser on the Mother Claude desktop app',
    );
    const paypal = [...el.querySelectorAll('.site')].find((s) => text(s).includes('paypal'))!;
    const act = [...paypal.querySelectorAll<HTMLButtonElement>('.segmented button')].find(
      (b) => text(b) === 'Act',
    )!;
    expect(act.disabled).toBe(true);
    const github = [...el.querySelectorAll('.site')].find((s) => text(s).includes('github'))!;
    const read = [...github.querySelectorAll<HTMLButtonElement>('.segmented button')].find(
      (b) => text(b) === 'Read',
    )!;
    expect(read.disabled).toBe(false);
    read.click();
    await flush();
    fixture.detectChanges();
    expect(backend.bodyOf('PUT /browser/sites')).toEqual({
      origin: 'https://github.com',
      mode: 'read',
    });
    expect(read.getAttribute('aria-pressed')).toBe('true');
  });

  it('on the desktop: pairs a browser and shows the code', async () => {
    asDesktop();
    const el = await render();
    const pair = el.querySelector<HTMLButtonElement>('.pair .btn.primary')!;
    expect(text(pair)).toContain('Pair a browser');
    pair.click();
    await flush();
    fixture.detectChanges();
    expect(backend.calls).toContain('POST /browser/pair');
    expect(text(el.querySelector('.pair .code'))).toContain('K7QD-8F3B');
    expect(text(el.querySelector('.pair'))).toContain('ws://127.0.0.1:6725/ws/browser');
  });

  it('refetches when a browser frame arrives', async () => {
    await render();
    const before = backend.calls.filter((c) => c === 'GET /browser').length;
    const realtime = TestBed.inject(RealtimeService) as unknown as {
      dispatch(raw: unknown): void;
    };
    realtime.dispatch(JSON.stringify({ t: 'browser', what: 'sites' }));
    await new Promise((r) => setTimeout(r, 200));
    await flush();
    expect(backend.calls.filter((c) => c === 'GET /browser').length).toBe(before + 1);
  });
});
