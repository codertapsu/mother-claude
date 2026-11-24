import { TestBed } from '@angular/core/testing';
import { Router, provideRouter } from '@angular/router';
import { RouterTestingHarness } from '@angular/router/testing';

import { FakeBackend, flush, useFakeServer } from '../../core/mock/fake-server.testing';
import { ownedCaps } from '../../core/mock/fixtures';
import { setActiveMock } from '../../core/mock/mock-api';
import { SessionDetail } from '../../core/models';
import { SessionContext } from './session-context';
import { SessionDetailComponent } from './session-detail.component';

describe('SessionDetailComponent routing', () => {
  let harness: RouterTestingHarness;
  let backend: FakeBackend;

  beforeEach(async () => {
    backend = new FakeBackend();
    useFakeServer(backend, [
      provideRouter([{ path: 'session/:id', component: SessionDetailComponent }]),
    ]);
    harness = await RouterTestingHarness.create();
  });

  afterEach(() => setActiveMock(null));

  const ctx = () => harness.routeDebugElement!.injector.get(SessionContext);

  it('opens another session on Chat, whatever tab was open', async () => {
    await harness.navigateByUrl('/session/a?tab=info');
    expect(ctx().tab()).toBe('info');
    await harness.navigateByUrl('/session/b');
    expect(ctx().id()).toBe('b');
    expect(ctx().tab()).toBe('chat');
  });

  it('still honours an explicit tab for the new session', async () => {
    await harness.navigateByUrl('/session/a?tab=info');
    await harness.navigateByUrl('/session/b?tab=agents');
    expect(ctx().tab()).toBe('agents');
  });

  it('brings a notified request forward on Chat and consumes the param', async () => {
    await harness.navigateByUrl('/session/a?tab=info');
    await harness.navigateByUrl('/session/b?pending=req-9');
    expect(ctx().tab()).toBe('chat');
    expect(ctx().focusPending()).toBe('req-9');
    await flush();
    await harness.fixture.whenStable();
    expect(TestBed.inject(Router).url).toBe('/session/b');
  });

  const text = () => (harness.routeNativeElement?.textContent ?? '').replace(/\s+/g, ' ');

  it('says the Mac is unreachable instead of "not found" when it is', async () => {
    backend.routes.set('/sessions/gone-offline', () =>
      Promise.reject(new TypeError('Load failed')),
    );
    await harness.navigateByUrl('/session/gone-offline');
    await flush();
    harness.detectChanges();
    expect(text()).toContain("Can't load this session");
    expect(text()).not.toContain('Session not found');
  });

  it('puts the model, mode, effort and ultracode above the message box on Chat, and keeps them on Info', async () => {
    const detail = {
      id: 's1',
      name: 'auth',
      projectName: 'api',
      cwd: '/Users/me/dev/api',
      surface: 'mother-claude',
      owned: true,
      live: true,
      status: 'idle',
      activity: { kind: 'idle', label: 'Idle' },
      usage: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 },
      messageCount: 0,
      agents: { running: 0, total: 0, workflowsRunning: 0, backgroundRunning: 0 },
      queued: 0,
      pending: [],
      model: { id: 'claude-opus-4-7', label: 'Opus 4.7' },
      permissionMode: 'default',
      ultracode: 'off',
      ultracodeSource: 'applied',
      control: ownedCaps(),
    } as unknown as SessionDetail;
    backend.routes.set('/sessions/s1', () => ({ status: 200, body: detail }));
    const page = () => harness.routeNativeElement as HTMLElement;
    const settings = ['Model', 'Permission mode', 'Effort'];

    await harness.navigateByUrl('/session/s1');
    await flush();
    harness.detectChanges();
    expect(ctx().tab()).toBe('chat');
    for (const name of settings) {
      expect(page().querySelector(`.sd-dock select[aria-label="${name}"]`)).not.toBeNull();
      // Not twice: the row at the top of the transcript leaves them out.
      expect(page().querySelector(`.sd-scroll select[aria-label="${name}"]`)).toBeNull();
    }
    expect(page().querySelector('.sd-dock [role="switch"][aria-label="Ultracode"]')).not.toBeNull();
    expect(page().querySelector('.sd-scroll mc-channel-badge')).not.toBeNull();

    await harness.navigateByUrl('/session/s1?tab=info');
    await flush();
    harness.detectChanges();
    expect(ctx().tab()).toBe('info');
    expect(page().querySelector('.sd-dock')).toBeNull();
    for (const name of settings) {
      expect(page().querySelector(`.sd-scroll select[aria-label="${name}"]`)).not.toBeNull();
    }
    expect(
      page().querySelector('.sd-scroll [role="switch"][aria-label="Ultracode"]'),
    ).not.toBeNull();
  });

  it('says "not found" for a 404', async () => {
    backend.routes.set('/sessions/missing', () => ({
      status: 404,
      body: { error: { code: 'not_found', message: 'No such session' } },
    }));
    await harness.navigateByUrl('/session/missing');
    await flush();
    harness.detectChanges();
    expect(text()).toContain('Session not found');
    expect(text()).toContain('This session no longer exists.');
  });
});
