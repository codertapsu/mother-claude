import { ComponentFixture, TestBed } from '@angular/core/testing';
import { provideRouter } from '@angular/router';

import { AuthService } from '../../core/auth.service';
import { FakeBackend, flush, me, useFakeServer } from '../../core/mock/fake-server.testing';
import { setActiveMock } from '../../core/mock/mock-api';
import { RealtimeService } from '../../core/realtime.service';
import { TeamView, TemplateEntry } from '../../core/teams';
import { StartTeamSheetComponent } from './start-team-sheet.component';
import { TeamsComponent } from './teams.component';

const ok = { errors: [], warnings: [] };

const templates: TemplateEntry[] = [
  {
    id: 'software-delivery',
    name: 'Software delivery',
    builtin: true,
    report: ok,
    roles: [
      { id: 'lead', title: 'Lead', lead: true, model: 'opus', autostart: true },
      { id: 'frontend', title: 'Frontend', model: 'sonnet', permissionMode: 'acceptEdits' },
    ],
  },
  {
    id: 'mine',
    name: 'Mine',
    builtin: false,
    report: { errors: [{ path: 'roles', message: 'x' }], warnings: [] },
    roles: [{ id: 'lead', title: 'Lead', lead: true, permissionMode: 'bypassPermissions' }],
  },
];

const team = (over: Partial<TeamView> = {}): TeamView => ({
  id: 'tm_0123456789ab',
  project: '/Users/me/dev/shop',
  label: 'shop',
  templateId: 'software-delivery',
  templateName: 'Software delivery',
  account: 'default',
  stopped: false,
  createdAt: 1,
  integration: {},
  idleStopMinutes: 30,
  reviews: true,
  roles: [
    {
      id: 'lead',
      title: 'Lead',
      lead: true,
      state: 'idle',
      sessionName: 'shop/lead',
      foreign: false,
      permissionMode: 'default',
      worktree: false,
      paths: [],
      readOnly: false,
      budgetApplied: false,
      autostart: true,
      classes: [],
      queuedTasks: 0,
      guardDenials: 0,
    },
  ],
  counts: { todo: 2, review: 1, done: 3 },
  ...over,
});

describe('TeamsComponent', () => {
  let backend: FakeBackend;
  let fixture: ComponentFixture<TeamsComponent>;

  beforeEach(() => {
    backend = new FakeBackend();
    useFakeServer(backend, [provideRouter([])]);
    backend.routes.set('/teams', () => ({ status: 200, body: { teams: [team()] } }));
    backend.routes.set('/team-templates', () => ({ status: 200, body: { templates } }));
    backend.routes.set('/teams/events', () => ({ status: 200, body: { events: [] } }));
    backend.routes.set('/projects', () => ({
      status: 200,
      body: { projects: [{ name: 'shop', path: '/Users/me/dev/shop' }] },
    }));
  });

  afterEach(() => setActiveMock(null));

  const render = async () => {
    fixture = TestBed.createComponent(TeamsComponent);
    fixture.detectChanges();
    await flush();
    fixture.detectChanges();
    return fixture.nativeElement as HTMLElement;
  };

  it('lists the teams with their roles and board, and the templates', async () => {
    const el = await render();
    const card = el.querySelector('a.team') as HTMLAnchorElement;
    expect(card.textContent).toContain('shop');
    expect(card.textContent).toContain('Lead · Idle');
    expect(card.textContent).toContain('3 open · 3 done');
    expect(card.textContent).toContain('1 to review');
    expect(card.getAttribute('href')).toBe('/teams/tm_0123456789ab');
    const rows = [...el.querySelectorAll('.template')].map((r) => r.textContent ?? '');
    expect(rows[0]).toContain('built in');
    expect(rows[0]).toContain('View');
    expect(rows[1]).toContain('1 problems');
    expect(rows[1]).toContain('Edit');
  });

  it('follows a team through `teams` frames', async () => {
    const el = await render();
    const rt = TestBed.inject(RealtimeService);
    await rt.start();
    backend.last.open();
    backend.last.frame({ t: 'hello', v: 2, me: me(), serverTime: Date.now() });
    backend.last.frame({ t: 'teams', team: team({ stopped: true }) });
    fixture.detectChanges();
    expect(el.querySelector('a.team')?.textContent).toContain('stopped');
    backend.last.frame({ t: 'teams', removed: 'tm_0123456789ab' });
    fixture.detectChanges();
    expect(el.querySelector('a.team')).toBeNull();
    rt.stop();
  });

  it('starts a team, and asks for the dangerous right a bypass role needs', async () => {
    TestBed.inject(AuthService).me.set(me({ allowDangerous: false }));
    backend.routes.set('/teams', (method) =>
      method === 'POST'
        ? { status: 201, body: team({ id: 'tm_aaaaaaaaaaaa' }) }
        : { status: 200, body: { teams: [] } },
    );
    await render();
    const sheet = TestBed.createComponent(StartTeamSheetComponent);
    sheet.componentRef.setInput('templates', templates);
    sheet.componentRef.setInput('open', true);
    sheet.detectChanges();
    await flush();
    sheet.detectChanges();
    const el = sheet.nativeElement as HTMLElement;
    /** Choose the option of a `<select name>` whose text starts with
     * `text` (Angular gives bound options values of its own). */
    const select = async (name: string, text: string) => {
      await flush();
      sheet.detectChanges();
      const s = el.querySelector(`select[name="${name}"]`) as HTMLSelectElement;
      const option = [...s.options].find((o) => (o.textContent ?? '').trim().startsWith(text));
      expect(option).withContext(`${name}: ${text}`).toBeDefined();
      s.value = option?.value ?? '';
      s.dispatchEvent(new Event('change'));
      sheet.detectChanges();
      await flush();
      sheet.detectChanges();
    };
    await select('project', 'shop');
    await select('template', 'Mine');
    expect(el.textContent).toContain('A role runs without permission prompts.');
    const submit = el.querySelector('button[type="submit"]') as HTMLButtonElement;
    expect(submit.disabled).toBeTrue();
    // Back to a safe template: it starts.
    await select('template', 'Software delivery');
    expect(el.textContent).not.toContain('A role runs without permission prompts.');
    await select('effort-frontend', 'high');
    expect(submit.disabled).toBeFalse();
    let started: TeamView | undefined;
    sheet.componentInstance.started.subscribe((t: TeamView) => (started = t));
    submit.click();
    await flush();
    expect(started?.id).toBe('tm_aaaaaaaaaaaa');
    expect(backend.bodyOf('POST /teams')).toEqual({
      project: '/Users/me/dev/shop',
      template: 'software-delivery',
      overrides: { frontend: { effort: 'high' } },
    });
  });
});
