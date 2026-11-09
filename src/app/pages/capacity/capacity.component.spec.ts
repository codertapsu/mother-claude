import { TestBed } from '@angular/core/testing';
import { provideRouter } from '@angular/router';

import { CapacityView, PolicyView, RoutingReport } from '../../core/capacity';
import { FakeBackend, asDesktop, flush, useFakeServer } from '../../core/mock/fake-server.testing';
import { setActiveMock } from '../../core/mock/mock-api';
import { MockCapacity, defaultPolicy } from '../../core/mock/mock-capacity';
import { CapacityBudgetsComponent } from './capacity-budgets.component';
import { CapacityComponent } from './capacity.component';
import { RoutingPolicyComponent } from './routing-policy.component';
import { RoutingReportComponent } from './routing-report.component';

const view = (desktop = false): CapacityView => new MockCapacity({ desktop }).view();

const policyView = (canEdit: boolean): PolicyView => ({
  policy: defaultPolicy(),
  defaults: defaultPolicy(),
  isDefault: true,
  classes: [
    { class: 'plan', label: 'planning' },
    { class: 'implement', label: 'implementation' },
    { class: 'other', label: 'other' },
  ],
  models: ['haiku', 'sonnet', 'opus', 'fable'],
  efforts: ['low', 'medium', 'high', 'xhigh', 'max'],
  learned: [{ project: '/Users/me/app', class: 'implement', level: 1, clean: 0 }],
  escalations: [],
  canEdit,
});

describe('CapacityComponent', () => {
  let backend: FakeBackend;

  beforeEach(() => {
    backend = new FakeBackend();
    useFakeServer(backend, [provideRouter([])]);
    localStorage.removeItem('mc_capacity_tab');
  });

  afterEach(() => setActiveMock(null));

  it('shows the signals, who waits and why', async () => {
    backend.routes.set('/capacity', () => ({ status: 200, body: view() }));
    const fixture = TestBed.createComponent(CapacityComponent);
    await flush();
    fixture.detectChanges();
    const text = (fixture.nativeElement as HTMLElement).textContent ?? '';
    expect(text).toContain('of 3 workers');
    expect(text).toContain('9.6 GB');
    expect(text).toContain('reviewer');
    expect(text).toContain('the 5-hour usage window is at 91 %');
    expect(text).toContain("website's daily budget is spent");
    expect(text).toContain('Only the lead starts');
    expect(text).toContain('escalated 1 step from Sonnet · medium');
    fixture.destroy();
  });

  it('says so when the server has no capacity data', async () => {
    backend.routes.set('/capacity', () => ({
      status: 404,
      body: { error: { code: 'not_found' } },
    }));
    const fixture = TestBed.createComponent(CapacityComponent);
    await flush();
    fixture.detectChanges();
    expect((fixture.nativeElement as HTMLElement).textContent).toContain(
      'This Mother Claude has no capacity data yet',
    );
    fixture.destroy();
  });
});

describe('CapacityBudgetsComponent', () => {
  let backend: FakeBackend;

  beforeEach(() => {
    backend = new FakeBackend();
    useFakeServer(backend, [provideRouter([])]);
  });

  afterEach(() => setActiveMock(null));

  it('lets a phone set a project budget and reads the answer', async () => {
    const after = view();
    after.settings.budgets.projects['/Users/me/app'] = 12.5;
    backend.routes.set('/capacity/settings', () => ({ status: 200, body: after }));
    const fixture = TestBed.createComponent(CapacityBudgetsComponent);
    fixture.componentRef.setInput('view', view());
    let emitted: CapacityView | undefined;
    fixture.componentInstance.changed.subscribe((v) => (emitted = v));
    fixture.detectChanges();
    const c = fixture.componentInstance as unknown as {
      edit(): void;
      addProject(): void;
      setProject(i: number, f: 'key' | 'usd', v: string): void;
      session: { set(v: string): void };
    };
    c.edit();
    c.addProject();
    c.setProject(1, 'key', '/Users/me/app');
    c.setProject(1, 'usd', '12.50');
    c.session.set('');
    await fixture.componentInstance.save();
    const sent = backend.bodyOf('PUT /capacity/settings') as {
      budgets: { projects: Record<string, number>; sessionUsd: number | null };
    };
    expect(sent.budgets.projects['/Users/me/app']).toBe(12.5);
    expect(sent.budgets.projects['/Users/marcus/dev/mother-claude']).toBe(20);
    expect(sent.budgets.sessionUsd).toBeNull();
    expect(emitted?.settings.budgets.projects['/Users/me/app']).toBe(12.5);
  });

  it('refuses an amount that is not one, before asking the server', async () => {
    const fixture = TestBed.createComponent(CapacityBudgetsComponent);
    fixture.componentRef.setInput('view', view());
    fixture.detectChanges();
    const c = fixture.componentInstance as unknown as {
      edit(): void;
      task: { set(v: string): void };
      error(): string;
    };
    c.edit();
    c.task.set('lots');
    await fixture.componentInstance.save();
    expect(backend.calls).not.toContain('PUT /capacity/settings');
    expect(c.error()).toBe('lots is not an amount in dollars.');
  });

  it('shows what each budget has spent', () => {
    const fixture = TestBed.createComponent(CapacityBudgetsComponent);
    fixture.componentRef.setInput('view', view());
    fixture.detectChanges();
    const text = (fixture.nativeElement as HTMLElement).textContent ?? '';
    expect(text).toContain('mother-claude today');
    expect(text).toContain('$16.40 of $20.00');
    expect(text).toContain('Opus work runs on Sonnet');
  });
});

describe('RoutingPolicyComponent', () => {
  let backend: FakeBackend;

  beforeEach(() => {
    backend = new FakeBackend();
    useFakeServer(backend, [provideRouter([])]);
  });

  afterEach(() => setActiveMock(null));

  it('is read-only on a phone', async () => {
    backend.routes.set('/routing/policy', () => ({ status: 200, body: policyView(false) }));
    const fixture = TestBed.createComponent(RoutingPolicyComponent);
    await flush();
    fixture.detectChanges();
    const text = (fixture.nativeElement as HTMLElement).textContent ?? '';
    expect(text).toContain('Opus · high');
    expect(text).toContain("the role's default");
    expect(text).toContain('The policy is edited on the desktop app.');
    expect(text).not.toContain('Edit the policy');
    expect(text).toContain('implement tasks in app');
  });

  it('validates an edit on the desktop and saves it', async () => {
    asDesktop();
    backend.routes.set('/routing/policy', (method, body) =>
      method === 'PUT'
        ? { status: 200, body: { ...policyView(true), policy: body, isDefault: false } }
        : { status: 200, body: policyView(true) },
    );
    const fixture = TestBed.createComponent(RoutingPolicyComponent);
    await flush();
    fixture.detectChanges();
    const c = fixture.componentInstance as unknown as {
      edit(): void;
      setRung(i: number, f: 'model' | 'effort', v: string): void;
      setClass(cls: string, f: 'model' | 'effort', v: string): void;
      invalid(): boolean;
    };
    c.edit();
    c.setRung(1, 'model', 'fable');
    fixture.detectChanges();
    expect(c.invalid()).toBe(true);
    expect((fixture.nativeElement as HTMLElement).textContent).toContain(
      'Each rung must be above the one before it',
    );
    await fixture.componentInstance.save();
    expect(backend.calls).not.toContain('PUT /routing/policy');
    c.setRung(1, 'model', 'haiku');
    c.setClass('plan', 'model', 'sonnet');
    expect(c.invalid()).toBe(false);
    await fixture.componentInstance.save();
    const sent = backend.bodyOf('PUT /routing/policy') as { classes: Record<string, unknown> };
    expect(sent.classes['plan']).toEqual({ model: 'sonnet', effort: 'high' });
  });
});

describe('RoutingReportComponent', () => {
  let backend: FakeBackend;

  beforeEach(() => {
    backend = new FakeBackend();
    useFakeServer(backend, [provideRouter([])]);
  });

  afterEach(() => setActiveMock(null));

  it('reads the report for the chosen period', async () => {
    const mock = new MockCapacity({ desktop: false });
    backend.routes.set('/routing/report', (_m, _b, url) => {
      const res =
        mock.route(
          '/routing/report',
          'GET',
          undefined,
          url?.searchParams ?? new URLSearchParams(),
        ) ?? new Response('{}');
      return res.json().then((body) => ({ status: 200, body: body as RoutingReport }));
    });
    const fixture = TestBed.createComponent(RoutingReportComponent);
    await flush();
    fixture.detectChanges();
    const text = (fixture.nativeElement as HTMLElement).textContent ?? '';
    expect(text).toContain('By class, model and effort');
    expect(text).toContain('implement');
    expect(text).toContain('Sonnet · medium');
    expect(backend.urls).toContain('/routing/report?days=30');
    const seven = [...(fixture.nativeElement as HTMLElement).querySelectorAll('button')].find(
      (b) => b.textContent?.trim() === '7 days',
    );
    seven?.click();
    await flush();
    expect(backend.urls).toContain('/routing/report?days=7');
  });
});
