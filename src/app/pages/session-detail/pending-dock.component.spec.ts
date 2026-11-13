import { TestBed } from '@angular/core/testing';
import { provideRouter } from '@angular/router';

import { FakeBackend, flush, useFakeServer } from '../../core/mock/fake-server.testing';
import { ownedCaps } from '../../core/mock/fixtures';
import { setActiveMock } from '../../core/mock/mock-api';
import { SessionDetail } from '../../core/models';
import { PendingDockComponent } from './pending-dock.component';
import { SessionContext } from './session-context';

const session = (id: string, pending: SessionDetail['pending']) =>
  ({
    id,
    name: id,
    projectName: 'api',
    cwd: '/Users/me/dev/api',
    surface: 'cli',
    owned: true,
    live: true,
    status: 'waiting',
    activity: { kind: 'idle', label: 'Idle' },
    usage: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 },
    messageCount: 1,
    agents: { running: 0, total: 0, workflowsRunning: 0, backgroundRunning: 0 },
    queued: 0,
    pending,
    control: ownedCaps(),
  }) as unknown as SessionDetail;

describe('PendingDockComponent', () => {
  beforeEach(() => useFakeServer(new FakeBackend(), [provideRouter([]), SessionContext]));
  afterEach(() => setActiveMock(null));

  it("leaves the last session's requests behind when another one opens in the page", async () => {
    const ctx = TestBed.inject(SessionContext);
    ctx.id.set('a');
    ctx.detail.set(
      session('a', [
        { id: 'p1', kind: 'permission', title: 'Bash', createdAt: 1, answerable: true },
      ] as SessionDetail['pending']),
    );
    const fixture = TestBed.createComponent(PendingDockComponent);
    fixture.detectChanges();
    await flush();
    fixture.detectChanges();
    const el = fixture.nativeElement as HTMLElement;
    expect(el.querySelectorAll('mc-pending-card').length).toBe(1);

    // Beside the list, the same page opens the next session.
    ctx.id.set('b');
    ctx.detail.set(session('b', []));
    fixture.detectChanges();
    await flush();
    fixture.detectChanges();
    expect(el.querySelectorAll('mc-pending-card').length).toBe(0);
    expect(el.textContent).not.toContain('Answered elsewhere');
  });
});
