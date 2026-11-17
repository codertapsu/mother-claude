import { ComponentFixture, TestBed } from '@angular/core/testing';
import { provideRouter } from '@angular/router';

import { FakeBackend, asDesktop, flush, useFakeServer } from '../../core/mock/fake-server.testing';
import { setActiveMock } from '../../core/mock/mock-api';
import { BridgeStatus } from '../../core/models';
import { BridgeComponent } from './bridge.component';

const status = (over: Partial<BridgeStatus> = {}): BridgeStatus => ({
  enabled: true,
  running: true,
  port: 5612,
  url: 'http://127.0.0.1:5612',
  requireToken: false,
  tokenPolicy: 'local',
  configuredTokenPolicy: 'local',
  exposedByTunnel: false,
  hostBuilt: true,
  hostRunning: true,
  messagesApi: false,
  claudeAuthenticated: true,
  claudeAccount: 'me@example.com',
  activeOperations: 0,
  defaults: {},
  threads: [],
  ...over,
});

describe('BridgeComponent: the Claude API', () => {
  let backend: FakeBackend;
  let fixture: ComponentFixture<BridgeComponent>;

  beforeEach(() => {
    backend = new FakeBackend();
    useFakeServer(backend, [provideRouter([])]);
    backend.routes.set('/defaults', () => ({ status: 200, body: { models: [], efforts: [] } }));
    backend.routes.set('/sessions', () => ({ status: 200, body: { sessions: [] } }));
    backend.routes.set('/bridge/clients', () => ({ status: 200, body: { roots: [] } }));
  });

  afterEach(() => setActiveMock(null));

  const render = async (s: BridgeStatus) => {
    backend.routes.set('/bridge', () => ({ status: 200, body: s }));
    backend.routes.set('/bridge/start', (_m, body) => ({
      status: 200,
      body: status({
        defaults: (body as { defaults: BridgeStatus['defaults'] }).defaults,
        networkUrls: ['http://192.168.1.10:5612', 'http://171.254.176.94:5612'],
        ...((body as { key?: string }).key ? { keyHint: '…a1b2' } : {}),
      }),
    }));
    fixture = TestBed.createComponent(BridgeComponent);
    fixture.detectChanges();
    await flush();
    fixture.detectChanges();
    return fixture.nativeElement as HTMLElement;
  };
  const reach = (el: HTMLElement, value: string) =>
    el.querySelector(`fieldset.reach input[value="${value}"]`) as HTMLInputElement;
  const button = (el: HTMLElement, text: string) =>
    [...el.querySelectorAll('button')].find((b) => b.textContent?.trim() === text)!;
  const startButton = (el: HTMLElement) =>
    [...el.querySelectorAll('button')].find((b) =>
      b.textContent?.includes('Restart with these settings'),
    )!;

  it('shows the base URL and a request in the API’s own shape', async () => {
    const el = await render(status());
    const snippet = [...el.querySelectorAll('pre.snippet')].map((p) => p.textContent ?? '');
    expect(snippet.some((t) => t.includes('/v1/messages') && t.includes('anthropic-version'))).toBe(
      true,
    );
  });

  it('the desktop makes it reachable from anywhere with a key, on another port', async () => {
    asDesktop();
    const el = await render(status());
    reach(el, 'all').click();
    fixture.detectChanges();
    expect(el.textContent).toContain('through a tunnel such as ngrok');
    expect(el.textContent).toContain('without a key');
    button(el, 'Generate').click();
    fixture.detectChanges();
    await flush();
    fixture.detectChanges();
    const input = el.querySelector('input.key-input') as HTMLInputElement;
    expect(input.type).toBe('text');
    expect(input.value).toMatch(/^mck-[A-Za-z0-9]{40}$/);
    expect(el.textContent).toContain('calls it with the key');
    const port = el.querySelector('input.port') as HTMLInputElement;
    port.value = '5678';
    port.dispatchEvent(new Event('input'));
    fixture.detectChanges();
    await flush();
    startButton(el).click();
    await flush();
    fixture.detectChanges();
    const sent = backend.bodyOf('POST /bridge/start') as {
      defaults: Record<string, unknown>;
      key?: string;
      port?: number;
    };
    expect(sent.defaults['access']).toBe('all');
    expect(sent.key).toBe(input.value);
    expect(sent.port).toBe(5678);
    expect(el.textContent).toContain('http://171.254.176.94:5612');
    expect(el.textContent).toContain('…a1b2');
    // The key is never written into an attribute.
    const attrs = [...el.querySelectorAll('*')].flatMap((n) =>
      [...n.attributes].map((a) => a.value),
    );
    expect(attrs.some((v) => v.startsWith('mck-'))).toBe(false);
  });

  it('a blank key is no authentication; one the server has comes off only when asked', async () => {
    asDesktop();
    let el = await render(status());
    startButton(el).click();
    await flush();
    let sent = backend.bodyOf('POST /bridge/start') as Record<string, unknown>;
    expect('key' in sent).toBe(false);
    fixture.destroy();
    el = await render(status({ keyHint: '…zz99' }));
    expect(el.textContent).toContain('…zz99');
    button(el, 'Remove').click();
    fixture.detectChanges();
    expect(el.textContent).toContain('no authentication');
    startButton(el).click();
    await flush();
    sent = backend.bodyOf('POST /bridge/start') as Record<string, unknown>;
    expect(sent['key']).toBe('');
  });

  it('a phone cannot open it beyond this computer, but keeps it open', async () => {
    let el = await render(status());
    expect(reach(el, 'lan').disabled).toBe(true);
    expect(reach(el, 'all').disabled).toBe(true);
    expect(el.textContent).toContain('Only the desktop app can open it beyond this');
    fixture.destroy();
    el = await render(
      status({ defaults: { access: 'lan' }, networkUrls: ['http://192.168.1.10:5612'] }),
    );
    expect(reach(el, 'lan').disabled).toBe(false);
    expect(reach(el, 'lan').checked).toBe(true);
    expect(reach(el, 'all').disabled).toBe(true);
    expect(el.textContent).toContain('http://192.168.1.10:5612');
  });
});
