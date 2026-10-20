import { Component } from '@angular/core';
import { ComponentFixture, TestBed } from '@angular/core/testing';
import { Router, provideRouter } from '@angular/router';

import { FakeBackend, flush, serveAccounts, useFakeServer } from '../core/mock/fake-server.testing';
import {
  MOCK_ACCOUNT_PRO,
  MOCK_ACCOUNT_WORK,
  buildAccounts,
  ownedCaps,
} from '../core/mock/fixtures';
import { setActiveMock } from '../core/mock/mock-api';
import { SessionSummary } from '../core/models';
import { ContinueOnSheetComponent } from './continue-on-sheet.component';

@Component({ template: '' })
class BlankComponent {}

const DEFAULT_REF = {
  id: 'default',
  name: 'Default',
  color: '#8b5cf6',
  kind: 'directory' as const,
};

const session = (over: Partial<SessionSummary> = {}): SessionSummary => ({
  id: 's1',
  name: 'auth',
  projectName: 'api',
  cwd: '/Users/me/dev/api',
  surface: 'vscode',
  owned: false,
  live: true,
  status: 'idle',
  activity: { kind: 'idle', label: 'Idle' },
  usage: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 },
  messageCount: 3,
  agents: { running: 0, total: 0, workflowsRunning: 0, backgroundRunning: 0 },
  queued: 0,
  pending: [],
  control: { ...ownedCaps(), channel: 'hooks', continueOwned: true },
  account: DEFAULT_REF,
  ...over,
});

describe('ContinueOnSheetComponent', () => {
  let backend: FakeBackend;
  let fixture: ComponentFixture<ContinueOnSheetComponent>;

  beforeEach(() => {
    backend = new FakeBackend();
    useFakeServer(backend, [provideRouter([{ path: 'session/:id', component: BlankComponent }])]);
    serveAccounts(backend, buildAccounts(Date.now(), true));
  });

  afterEach(() => setActiveMock(null));

  const open = async (s: SessionSummary, suggested: string | null = MOCK_ACCOUNT_PRO) => {
    fixture = TestBed.createComponent(ContinueOnSheetComponent);
    fixture.componentRef.setInput('sessions', [s]);
    fixture.componentRef.setInput('suggested', suggested);
    fixture.componentRef.setInput('open', true);
    fixture.detectChanges();
    await flush();
    fixture.detectChanges();
    return fixture.nativeElement as HTMLElement;
  };
  const text = (el: Element) => (el.textContent ?? '').replace(/\s+/g, ' ').trim();
  const options = (el: HTMLElement) => [
    ...el.querySelectorAll<HTMLButtonElement>('[role="radiogroup"][aria-label="Account"] .opt'),
  ];
  const goButton = (el: HTMLElement) =>
    [...el.querySelectorAll<HTMLButtonElement>('.actions .btn.primary')][0];

  it('offers the other accounts, saying why one cannot take the conversation', async () => {
    const el = await open(session());
    const opts = options(el);
    expect(opts.map((o) => text(o.querySelector('.oname')!))).toEqual(['Work', 'Marcus (Pro)']);
    // Work keeps its own directory: not before Move/Copy.
    expect(opts[0].disabled).toBeTrue();
    expect(text(opts[0])).toContain('Keeps its own directory');
    // The suggestion is preselected.
    expect(opts[1].getAttribute('aria-checked')).toBe('true');
    expect(text(goButton(el))).toBe('Continue on Marcus (Pro)');
  });

  it('forks the conversation under the chosen account and opens the copy', async () => {
    backend.routes.set('/sessions/s1/continue', () => ({
      status: 201,
      body: { id: 's2', forked: true },
    }));
    const closed = jasmine.createSpy('closed');
    const el = await open(session());
    fixture.componentInstance.closed.subscribe(closed);
    goButton(el).click();
    await flush();
    expect(backend.bodyOf('POST /sessions/s1/continue')).toEqual({
      account: MOCK_ACCOUNT_PRO,
      fork: true,
    });
    expect(TestBed.inject(Router).url).toBe('/session/s2');
    expect(closed).toHaveBeenCalled();
  });

  it("shows the server's words when the account needs a copy first", async () => {
    backend.routes.set('/sessions/s1/continue', () => ({
      status: 409,
      body: {
        error: {
          code: 'account_needs_copy',
          message: 'That account keeps its own directory: copy it there first.',
        },
      },
    }));
    const el = await open(session());
    goButton(el).click();
    await flush();
    fixture.detectChanges();
    expect(text(el.querySelector('.notice.error')!)).toBe(
      'That account keeps its own directory: copy it there first.',
    );
  });

  it('says plainly that a token is missing', async () => {
    backend.routes.set('/sessions/s1/continue', () => ({
      status: 409,
      body: { error: { code: 'account_token_missing', message: 'the token is not stored' } },
    }));
    const el = await open(session());
    goButton(el).click();
    await flush();
    fixture.detectChanges();
    expect(text(el.querySelector('.notice.error')!)).toContain(
      'Marcus (Pro) has no token on this Mac yet',
    );
  });

  it('stops a session Mother Claude runs, then continues it on the account', async () => {
    let continues = 0;
    backend.routes.set('/sessions/s1/continue', () =>
      ++continues === 1
        ? {
            status: 409,
            body: { error: { code: 'already_owned', message: 'already driven by Mother Claude' } },
          }
        : { status: 201, body: { id: 's3', forked: true } },
    );
    backend.routes.set('/sessions/s1/stop', () => ({ status: 200, body: { ok: true } }));
    const el = await open(session({ owned: true, control: ownedCaps() }));
    goButton(el).click();
    await flush();
    fixture.detectChanges();
    expect(text(el.querySelector('.notice.warn')!)).toContain('it stops here first');
    expect(backend.calls).not.toContain('POST /sessions/s1/stop');

    goButton(el).click();
    await new Promise((r) => setTimeout(r, 400));
    await flush();
    expect(backend.calls.filter((c) => c === 'POST /sessions/s1/stop').length).toBe(1);
    expect(backend.bodyOf('POST /sessions/s1/continue')).toEqual({
      account: MOCK_ACCOUNT_PRO,
      fork: true,
    });
    expect(TestBed.inject(Router).url).toBe('/session/s3');
  });

  it('offers nothing it cannot run: a token account without its token', async () => {
    const view = buildAccounts(Date.now(), true);
    view.accounts = view.accounts.map((a) =>
      a.id === MOCK_ACCOUNT_PRO ? { ...a, tokenStored: false, attention: 'tokenMissing' } : a,
    );
    serveAccounts(backend, view);
    const el = await open(session(), MOCK_ACCOUNT_PRO);
    const opts = options(el);
    expect(opts.every((o) => o.disabled)).toBeTrue();
    expect(text(opts[1])).toContain('No token on this Mac yet.');
    expect(goButton(el).disabled).toBeTrue();
    expect(text(goButton(el))).toBe('Pick an account');
  });

  it('from a separate directory, every other account needs a copy first', async () => {
    const work = {
      id: MOCK_ACCOUNT_WORK,
      name: 'Work',
      color: '#0ea5e9',
      kind: 'directory' as const,
    };
    const el = await open(session({ account: work }), null);
    const opts = options(el);
    expect(opts.map((o) => text(o.querySelector('.oname')!))).toEqual(['Default', 'Marcus (Pro)']);
    expect(opts.every((o) => o.disabled)).toBeTrue();
  });
});
