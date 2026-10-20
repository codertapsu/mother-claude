import { ComponentFixture, TestBed } from '@angular/core/testing';
import { provideRouter } from '@angular/router';

import {
  FakeBackend,
  flush,
  serveAccounts,
  useFakeServer,
} from '../../core/mock/fake-server.testing';
import {
  MOCK_ACCOUNT_PRO,
  MOCK_ACCOUNT_WORK,
  buildAccounts,
  ownedCaps,
} from '../../core/mock/fixtures';
import { setActiveMock } from '../../core/mock/mock-api';
import { AccountRef, PendingPreview, SessionSummary } from '../../core/models';
import { RealtimeService } from '../../core/realtime.service';
import { InboxComponent } from './inbox.component';

const DEFAULT_REF: AccountRef = {
  id: 'default',
  name: 'Default',
  color: '#8b5cf6',
  kind: 'directory',
};
const PRO_REF: AccountRef = {
  id: MOCK_ACCOUNT_PRO,
  name: 'Marcus (Pro)',
  color: '#f59e0b',
  kind: 'token',
};

const ask = (id: string, createdAt: number): PendingPreview => ({
  id,
  kind: 'permission',
  title: `Allow ${id}?`,
  toolName: 'Bash',
  dangerous: false,
  answerable: true,
  createdAt,
});

const session = (id: string, pending: PendingPreview[], account?: AccountRef): SessionSummary =>
  ({
    id,
    name: id,
    title: `Session ${id}`,
    projectName: 'api',
    cwd: '/Users/me/dev/api',
    surface: 'mother-claude',
    owned: true,
    live: true,
    status: 'waiting',
    activity: { kind: 'waiting', label: 'Waiting' },
    usage: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 },
    messageCount: 1,
    agents: { running: 0, total: 0, workflowsRunning: 0, backgroundRunning: 0 },
    queued: 0,
    pending,
    control: ownedCaps(),
    ...(account ? { account } : {}),
  }) as SessionSummary;

describe('InboxComponent accounts', () => {
  let backend: FakeBackend;
  let fixture: ComponentFixture<InboxComponent>;

  beforeEach(() => {
    backend = new FakeBackend();
    useFakeServer(backend, [provideRouter([])]);
    backend.routes.set('/inbox', () => ({ status: 200, body: { items: [] } }));
  });

  afterEach(() => setActiveMock(null));

  const store = (list: SessionSummary[]) =>
    (
      TestBed.inject(RealtimeService) as unknown as { replaceStore(l: SessionSummary[]): void }
    ).replaceStore(list);
  const render = async () => {
    fixture = TestBed.createComponent(InboxComponent);
    fixture.detectChanges();
    await flush();
    fixture.detectChanges();
    return fixture.nativeElement as HTMLElement;
  };
  const text = (el: Element | null | undefined) =>
    (el?.textContent ?? '').replace(/\s+/g, ' ').trim();
  const titles = (el: HTMLElement) =>
    [...el.querySelectorAll('mc-pending-card .title')].map((t) => text(t));

  it('with only the Default shows no filter and no chips', async () => {
    serveAccounts(backend, buildAccounts(Date.now(), false));
    store([session('s1', [ask('one', 1)]), session('s2', [ask('two', 2)])]);
    const el = await render();
    expect(el.querySelector('select[aria-label="Account"]')).toBeNull();
    expect(el.querySelector('mc-account-chip')).toBeNull();
    expect(titles(el)).toEqual(['Allow one?', 'Allow two?']);
  });

  it("names each request's account and shows one account's requests", async () => {
    serveAccounts(backend, buildAccounts(Date.now(), true));
    store([session('s1', [ask('one', 1)], DEFAULT_REF), session('s2', [ask('two', 2)], PRO_REF)]);
    const el = await render();
    expect([...el.querySelectorAll('mc-pending-card mc-account-chip')].map((c) => text(c))).toEqual(
      ['Account Default', 'Account Marcus (Pro)'],
    );
    const sel = el.querySelector('select[aria-label="Account"]') as HTMLSelectElement;
    expect([...sel.options].map((o) => o.text.trim())).toEqual([
      'All accounts',
      'Default',
      'Work',
      'Marcus (Pro)',
    ]);
    sel.value = MOCK_ACCOUNT_PRO;
    sel.dispatchEvent(new Event('change'));
    fixture.detectChanges();
    expect(titles(el)).toEqual(['Allow two?']);

    sel.value = MOCK_ACCOUNT_WORK;
    sel.dispatchEvent(new Event('change'));
    fixture.detectChanges();
    expect(titles(el)).toEqual([]);
    expect(text(el.querySelector('.empty-state strong'))).toBe('Nothing from Work needs you');
    expect(text(el.querySelector('.empty-state'))).toContain(
      '2 requests from other accounts wait.',
    );
    (el.querySelector('.empty-state .btn') as HTMLButtonElement).click();
    fixture.detectChanges();
    expect(titles(el)).toEqual(['Allow one?', 'Allow two?']);
  });
});
