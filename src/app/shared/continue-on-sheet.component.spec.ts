import { Component } from '@angular/core';
import { ComponentFixture, TestBed } from '@angular/core/testing';
import { Router, provideRouter } from '@angular/router';

import {
  FakeBackend,
  asDesktop,
  flush,
  serveAccounts,
  useFakeServer,
} from '../core/mock/fake-server.testing';
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

const consentRequired = {
  status: 409,
  body: {
    error: {
      code: 'copy_consent_required',
      message:
        'That account keeps its own directory: the conversation has to be copied there first.',
      from: 'default',
      to: MOCK_ACCOUNT_WORK,
    },
  },
};

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
  const settle = async () => {
    await flush();
    fixture.detectChanges();
    await flush();
    fixture.detectChanges();
  };
  const text = (el: Element | null | undefined) =>
    (el?.textContent ?? '').replace(/\s+/g, ' ').trim();
  const options = (el: HTMLElement) => [
    ...el.querySelectorAll<HTMLButtonElement>('[role="radiogroup"][aria-label="Account"] .opt'),
  ];
  const goButton = (el: HTMLElement) =>
    [...el.querySelectorAll<HTMLButtonElement>('.actions .btn.primary')][0];
  const pick = async (el: HTMLElement, name: string) => {
    options(el)
      .find((o) => text(o.querySelector('.oname')) === name)
      ?.click();
    fixture.detectChanges();
  };

  it('offers every other account, saying when the conversation is copied first', async () => {
    const el = await open(session());
    const opts = options(el);
    expect(opts.map((o) => text(o.querySelector('.oname')))).toEqual(['Work', 'Marcus (Pro)']);
    // Work keeps its own directory: it can be picked, the copy is said.
    expect(opts[0].disabled).toBeFalse();
    expect(text(opts[0])).toContain('Keeps its own directory');
    expect(text(opts[0])).toContain('copied there first (asked once)');
    // The token account shares the directory: a plain fork.
    expect(text(opts[1])).not.toContain('directory');
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

  it('asks the desktop once before the first copy, then copies and continues', async () => {
    asDesktop();
    let continues = 0;
    backend.routes.set('/sessions/s1/continue', () =>
      ++continues === 1 ? consentRequired : { status: 201, body: { id: 's4', forked: true } },
    );
    backend.routes.set('/accounts/copy-consent', (_m, body) => {
      const view = buildAccounts(Date.now(), true);
      view.copyConsent = [{ from: 'default', to: MOCK_ACCOUNT_WORK }];
      expect(body).toEqual({ from: 'default', to: MOCK_ACCOUNT_WORK, granted: true });
      return { status: 200, body: view };
    });
    const el = await open(session());
    expect(text(options(el)[0])).toContain('copied there first (asked once)');
    await pick(el, 'Work');
    goButton(el).click();
    await settle();
    const ask = el.querySelector('[aria-label="Allow copies"]');
    expect(text(ask)).toContain(
      'Copy conversations from Default to Work when you continue them there? Asked once; you can revoke it on the Accounts page.',
    );
    expect(backend.calls).not.toContain('PUT /accounts/copy-consent');

    (ask?.querySelector('.btn.primary') as HTMLButtonElement).click();
    await settle();
    expect(backend.calls).toContain('PUT /accounts/copy-consent');
    expect(backend.calls.filter((c) => c === 'POST /sessions/s1/continue').length).toBe(2);
    expect(backend.bodyOf('POST /sessions/s1/continue')).toEqual({
      account: MOCK_ACCOUNT_WORK,
      fork: true,
    });
    expect(TestBed.inject(Router).url).toBe('/session/s4');
  });

  it('"Not now" leaves everything as it was', async () => {
    asDesktop();
    backend.routes.set('/sessions/s1/continue', () => consentRequired);
    const el = await open(session());
    await pick(el, 'Work');
    goButton(el).click();
    await settle();
    const ask = el.querySelector('[aria-label="Allow copies"]');
    (ask?.querySelector('.btn.ghost') as HTMLButtonElement).click();
    fixture.detectChanges();
    expect(el.querySelector('[aria-label="Allow copies"]')).toBeNull();
    expect(backend.calls).not.toContain('PUT /accounts/copy-consent');
    expect(text(goButton(el))).toBe('Continue on Work');
  });

  it('asks a phone too before the first copy (docs/ACCOUNTS.md §13.9)', async () => {
    let continues = 0;
    backend.routes.set('/sessions/s1/continue', () =>
      ++continues === 1 ? consentRequired : { status: 201, body: { id: 's5', forked: true } },
    );
    backend.routes.set('/accounts/copy-consent', () => ({
      status: 200,
      body: buildAccounts(Date.now(), true),
    }));
    const el = await open(session());
    await pick(el, 'Work');
    goButton(el).click();
    await settle();
    const ask = el.querySelector('[aria-label="Allow copies"]');
    expect(ask).not.toBeNull();
    (ask?.querySelector('.btn.primary') as HTMLButtonElement).click();
    await settle();
    expect(backend.calls).toContain('PUT /accounts/copy-consent');
    expect(TestBed.inject(Router).url).toBe('/session/s5');
  });

  it('says plainly that a token is missing', async () => {
    backend.routes.set('/sessions/s1/continue', () => ({
      status: 409,
      body: { error: { code: 'account_token_missing', message: 'the token is not stored' } },
    }));
    const el = await open(session());
    goButton(el).click();
    await settle();
    expect(text(el.querySelector('.notice.error'))).toContain(
      'Marcus (Pro) has no token on this Mac yet',
    );
  });

  it("puts a copy's refusal in words", async () => {
    asDesktop();
    backend.routes.set('/sessions/s1/continue', () => ({
      status: 409,
      body: {
        error: {
          code: 'config_locked',
          message:
            'Claude Code is writing /Users/me/.claude-work/.claude.json right now; try again in a few seconds',
        },
      },
    }));
    const el = await open(session());
    await pick(el, 'Work');
    goButton(el).click();
    await settle();
    expect(text(el.querySelector('.notice.error'))).toBe(
      'Claude Code is writing /Users/me/.claude-work/.claude.json right now; try again in a few seconds.',
    );
  });

  it('continues a session Mother Claude runs on another account without stopping it', async () => {
    backend.routes.set('/sessions/s1/continue', () => ({
      status: 201,
      body: { id: 's3', forked: true },
    }));
    const el = await open(session({ owned: true, control: ownedCaps() }));
    expect(el.querySelector('.notice.warn')).toBeNull();
    goButton(el).click();
    await settle();
    expect(backend.calls).not.toContain('POST /sessions/s1/stop');
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
    expect(opts[1].disabled).toBeTrue();
    expect(text(opts[1])).toContain('No token on this Mac yet.');
    // The suggestion can't run: the first account that can is picked.
    expect(opts[0].disabled).toBeFalse();
    expect(text(goButton(el))).toBe('Continue on Work');
  });

  it('from a separate directory, every other account takes a copy', async () => {
    const work = {
      id: MOCK_ACCOUNT_WORK,
      name: 'Work',
      color: '#0ea5e9',
      kind: 'directory' as const,
    };
    const el = await open(session({ account: work }), null);
    const opts = options(el);
    expect(opts.map((o) => text(o.querySelector('.oname')))).toEqual(['Default', 'Marcus (Pro)']);
    expect(opts.every((o) => !o.disabled)).toBeTrue();
    expect(text(opts[0])).toContain('Keeps its own directory');
    expect(text(opts[1])).toContain("Uses the Default's directory");
  });
});
