import { ComponentFixture, TestBed } from '@angular/core/testing';
import { provideRouter } from '@angular/router';

import {
  FakeBackend,
  asDesktop,
  asPhoneOf,
  flush,
  useFakeServer,
} from '../../core/mock/fake-server.testing';
import { buildFixtures } from '../../core/mock/fixtures';
import { setActiveMock } from '../../core/mock/mock-api';
import { DetectedTunnel, RemoteInfo } from '../../core/models';
import { RemoteComponent } from './remote.component';

const text = (n: Element | null | undefined) => (n?.textContent ?? '').replace(/\s+/g, ' ').trim();

describe('RemoteComponent and API servers', () => {
  let backend: FakeBackend;
  let fixture: ComponentFixture<RemoteComponent>;

  beforeEach(() => {
    backend = new FakeBackend();
    useFakeServer(backend, [provideRouter([])]);
    asDesktop();
    backend.routes.set('/devices', () => ({ status: 200, body: { devices: [], pending: [] } }));
    backend.routes.set('/control/foreign', () => ({
      status: 200,
      body: { installed: false, cliAvailable: true, sessionsWithHooks: 0, notes: [] },
    }));
  });

  afterEach(() => {
    fixture?.destroy();
    setActiveMock(null);
  });

  const mainTunnel: DetectedTunnel = {
    provider: 'ngrok',
    url: 'https://main.ngrok.app',
    upstream: 'http://127.0.0.1:5612',
    upstreamPort: 5612,
    kind: 'http',
    target: 'bridge',
  };
  // A server besides the main one is named by `server`.
  const workTunnel = {
    ...mainTunnel,
    url: 'https://acme.ngrok.app',
    upstream: 'http://127.0.0.1:5613',
    upstreamPort: 5613,
    server: 'work',
  } as DetectedTunnel;

  const render = async (remote: RemoteInfo) => {
    backend.routes.set('/remote', () => ({ status: 200, body: remote }));
    fixture = TestBed.createComponent(RemoteComponent);
    fixture.detectChanges();
    await flush();
    fixture.detectChanges();
    return fixture.nativeElement as HTMLElement;
  };
  const base = (): RemoteInfo => buildFixtures(Date.now()).remote;

  it('names the API server a detected tunnel points at, and says it is exposed', async () => {
    const r = base();
    r.internet.detected = [...r.internet.detected, workTunnel];
    r.bridge = {
      port: 5612,
      running: true,
      exposed: false,
      servers: [
        { id: 'main', name: 'Main', port: 5612, running: true, exposed: false },
        { id: 'work', name: 'Work', port: 5613, running: true, exposed: true },
      ],
    } as RemoteInfo['bridge'];
    const el = await render(r);
    const rows = [...el.querySelectorAll('.tunnel')];
    expect(rows.length).toBe(2);
    const link = rows[1].querySelector<HTMLAnchorElement>('a.api-server')!;
    expect(text(link)).toBe('API server Work');
    expect(link.getAttribute('href')).toContain('server=work');
    // The tunnel to the dashboard's own port names no server.
    expect(rows[0].querySelector('a.api-server')).toBeNull();
    expect(text(el.querySelector('.internet .notice.error'))).toBe(
      'A tunnel points at the API server Work (port 5613): its tokenless access is switched off until that tunnel stops. Stop it unless you meant to publish that API.',
    );
  });

  it('lists the servers when no tunnel points at one', async () => {
    const r = base();
    r.bridge = {
      port: 5612,
      running: true,
      servers: [
        { id: 'main', name: 'Main', port: 5612, running: true, exposed: false },
        { id: 'work', name: 'Work', port: 5613, running: false, exposed: false },
      ],
    } as RemoteInfo['bridge'];
    const el = await render(r);
    expect(text(el.querySelector('.bridge'))).toBe(
      'API servers (Main: running, port 5612 · Work: stopped, port 5613) stay on this computer — the tunnel port never serves them. API settings',
    );
  });

  it('with only the main server reads as it did', async () => {
    const r = base();
    r.internet.detected = [...r.internet.detected, mainTunnel];
    r.bridge = { port: 5612, running: true, exposed: true };
    const el = await render(r);
    expect(el.querySelector('a.api-server')).toBeNull();
    expect(text(el.querySelector('.internet .notice.error'))).toBe(
      'A tunnel points at the HTTP API port 5612: its tokenless access is switched off until that tunnel stops. Stop it unless you meant to publish the API.',
    );
  });

  it('shows a phone itself, then the devices paired, read only — no dead end', async () => {
    asPhoneOf('macos');
    backend.routes.set('/devices', () => ({
      status: 200,
      body: {
        self: 'dev_2',
        devices: [
          {
            id: 'dev_1',
            name: 'iPad',
            platform: 'iPadOS',
            createdAt: 1,
            lastSeenAt: Date.now() - 3_600_000,
            allowDangerous: false,
            allowDangerousInternet: false,
            online: false,
            current: false,
          },
          {
            id: 'dev_2',
            name: 'iPhone',
            platform: 'iOS',
            createdAt: 1,
            allowDangerous: true,
            allowDangerousInternet: false,
            online: true,
            current: true,
          },
        ],
      },
    }));
    const el = await render(base());
    expect(text(el.querySelector('mc-page-head h1'))).toBe('Phone & devices');
    expect(el.querySelector('.this-device')).not.toBeNull();
    const rows = [...el.querySelectorAll('.device-ro')];
    // This device first, marked; what each may do in words.
    expect(rows.map((r) => text(r.querySelector('strong')))).toEqual(['iPhone', 'iPad']);
    expect(text(rows[0])).toContain('This device');
    expect(text(rows[0])).toContain('Dangerous actions on the local network');
    expect(text(rows[1])).toContain('seen');
    // Nothing to pair, rename or revoke from here, and no desktop reads.
    expect(el.querySelector('.card.pair')).toBeNull();
    expect([...el.querySelectorAll('button')].filter((b) => !b.closest('mc-page-head'))).toEqual(
      [],
    );
    expect(backend.calls).not.toContain('GET /remote');
  });
});
