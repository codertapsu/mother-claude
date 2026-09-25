import { TestBed } from '@angular/core/testing';
import { Router, provideRouter } from '@angular/router';
import { RouterTestingHarness } from '@angular/router/testing';

import { FakeBackend, flush, useFakeServer } from '../../core/mock/fake-server.testing';
import { setActiveMock } from '../../core/mock/mock-api';
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
