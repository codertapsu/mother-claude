import { TestBed } from '@angular/core/testing';
import { Router, provideRouter } from '@angular/router';
import { RouterTestingHarness } from '@angular/router/testing';

import { FakeBackend, flush, useFakeServer } from '../../core/mock/fake-server.testing';
import { setActiveMock } from '../../core/mock/mock-api';
import { SessionContext } from './session-context';
import { SessionDetailComponent } from './session-detail.component';

describe('SessionDetailComponent routing', () => {
  let harness: RouterTestingHarness;

  beforeEach(async () => {
    useFakeServer(new FakeBackend(), [
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
});
