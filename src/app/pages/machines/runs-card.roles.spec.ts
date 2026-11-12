import { ComponentFixture, TestBed } from '@angular/core/testing';
import { provideRouter } from '@angular/router';

import { FakeBackend, asDesktop, flush, useFakeServer } from '../../core/mock/fake-server.testing';
import { setActiveMock } from '../../core/mock/mock-api';
import { HostedRole } from '../../core/tasks';
import { RunsCardComponent } from './runs-card.component';
import { PEER_ID } from './machines.testing';

const SESSION = 'c0ee0000-1111-4222-8333-444455556666';

function hosted(over: Partial<HostedRole> = {}): HostedRole {
  return {
    binding: 'rb_0123456789abcdef',
    lead: PEER_ID,
    leadName: 'MacBook',
    team: 'tm_0123456789ab',
    teamLabel: 'shop',
    role: 'windows',
    roleTitle: 'Windows',
    project: '/Users/you/dev/shop',
    projectName: 'shop',
    checkout: 'D:\\dev\\shop',
    folder: 'C:\\data\\machines\\work\\roles\\rb_0123456789abcdef\\shop',
    session: SESSION,
    state: 'running',
    error: null,
    permissionMode: 'acceptEdits',
    model: 'sonnet',
    accountName: 'Default',
    alive: true,
    busy: false,
    denials: 0,
    results: 2,
    keptNote: null,
    startedAt: Date.now() - 600_000,
    stoppedAt: null,
    heardAt: Date.now() - 2_000,
    ...over,
  };
}

describe('RunsCardComponent: team roles run for another computer', () => {
  let backend: FakeBackend;
  let fixture: ComponentFixture<RunsCardComponent>;

  beforeEach(() => {
    backend = new FakeBackend();
    useFakeServer(backend, [provideRouter([])]);
  });

  afterEach(() => {
    fixture?.destroy();
    setActiveMock(null);
  });

  const text = (el: Element | null | undefined) =>
    (el?.textContent ?? '').replace(/\s+/g, ' ').trim();
  const settle = async () => {
    for (let i = 0; i < 3; i++) {
      await flush();
      fixture.detectChanges();
    }
  };
  const render = async (roles: HostedRole[] | null) => {
    backend.routes.set('/machines/runs', () => ({ status: 200, body: { runs: [] } }));
    backend.routes.set('/machines/run-grants', () => ({ status: 200, body: { grants: [] } }));
    backend.routes.set('/machines/tasks', () => ({ status: 200, body: { tasks: [] } }));
    if (roles) {
      backend.routes.set('/machines/roles', () => ({ status: 200, body: { roles } }));
    }
    fixture = TestBed.createComponent(RunsCardComponent);
    fixture.detectChanges();
    await settle();
    return fixture.nativeElement as HTMLElement;
  };

  it('shows nothing when this computer runs no role for another', async () => {
    asDesktop({ hostOs: 'windows' });
    const el = await render(null);
    expect(el.querySelector('mc-machines-roles h3')).toBeNull();
    expect(text(el)).not.toContain('Team roles');
  });

  it('lists each role: its team, whose lead, its state, its session; Stop while it runs', async () => {
    asDesktop({ hostOs: 'windows' });
    let stopped = false;
    backend.routes.set('/machines/roles/rb_0123456789abcdef/stop', () => {
      stopped = true;
      return { status: 200, body: {} };
    });
    const el = await render([
      hosted(),
      hosted({
        binding: 'rb_fedcba9876543210',
        role: 'ios',
        roleTitle: 'iOS',
        alive: false,
        state: 'stopped',
        session: null,
        keptNote: 'Its worktree holds work the lead has not merged yet.',
      }),
    ]);
    const card = el.querySelector('mc-machines-roles')!;
    expect(text(card)).toContain('Team roles');
    const rows = [...card.querySelectorAll('li')];
    expect(rows.length).toBe(2);
    expect(text(rows[0])).toContain('Windows of shop');
    expect(text(rows[0])).toContain('for MacBook');
    expect(text(rows[0])).toContain('Idle');
    expect(text(rows[0])).toContain('2 results');
    expect(rows[0].querySelector('a')?.getAttribute('href')).toContain(SESSION);
    expect(text(rows[1])).toContain('Stopped');
    expect(text(rows[1])).toContain('not merged');
    expect([...rows[1].querySelectorAll('button')].map((b) => text(b))).not.toContain('Stop');
    const stop = [...rows[0].querySelectorAll('button')].find((b) => text(b) === 'Stop')!;
    stop.click();
    await settle();
    expect(stopped).toBe(true);
    expect(backend.calls).toContain('POST /machines/roles/rb_0123456789abcdef/stop');
  });
});
