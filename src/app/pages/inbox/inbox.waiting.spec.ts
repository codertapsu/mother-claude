import { Component } from '@angular/core';
import { ComponentFixture, TestBed } from '@angular/core/testing';
import { provideRouter } from '@angular/router';

import { CopiesService } from '../../core/copies.service';
import { MachinesService } from '../../core/machines.service';
import {
  PEER_ID,
  conflicted,
  failingSetup,
  job,
  keptCopy,
  machineAsk,
  machinesView,
  pairRequest,
  peer,
  permission,
} from '../../core/mock/attention.testing';
import {
  FakeBackend,
  asDesktop,
  flush,
  serveAccounts,
  useFakeServer,
} from '../../core/mock/fake-server.testing';
import { MOCK_ACCOUNT_WORK, accountFixture, buildAccounts } from '../../core/mock/fixtures';
import { setActiveMock } from '../../core/mock/mock-api';
import { JobView, PendingPreview } from '../../core/models';
import { PairRequestsService } from '../../core/pair-requests.service';
import { PermissionsService } from '../../core/permissions.service';
import { RealtimeService } from '../../core/realtime.service';
import { SyncService } from '../../core/sync.service';
import { UpdaterService } from '../../core/updater.service';
import { InboxComponent } from './inbox.component';

@Component({ template: '' })
class BlankComponent {}

const request = (id: string, answerable = true): PendingPreview => ({
  id,
  kind: 'permission',
  title: `Allow ${id}?`,
  toolName: 'Bash',
  dangerous: false,
  answerable,
  createdAt: Date.now() - 60_000,
});

describe('InboxComponent: everything that waits for you', () => {
  let backend: FakeBackend;
  let fixture: ComponentFixture<InboxComponent>;

  beforeEach(() => {
    backend = new FakeBackend();
    useFakeServer(backend, [provideRouter([{ path: '**', component: BlankComponent }])]);
    backend.routes.set('/inbox', () => ({ status: 200, body: { items: [] } }));
    backend.routes.set('/data/jobs', () => ({ status: 200, body: { jobs: [] } }));
    backend.routes.set('/devices', () => ({ status: 200, body: { devices: [], pending: [] } }));
  });

  afterEach(() => {
    fixture?.destroy();
    setActiveMock(null);
    localStorage.removeItem('mc_inbox_dismissed');
  });

  const text = (e: Element | null | undefined) =>
    (e?.textContent ?? '').replace(/\s+/g, ' ').trim();
  const settle = async () => {
    for (let i = 0; i < 4; i++) {
      await flush();
      fixture.detectChanges();
    }
  };
  const render = async () => {
    fixture = TestBed.createComponent(InboxComponent);
    fixture.detectChanges();
    await settle();
    return fixture.nativeElement as HTMLElement;
  };
  const section = (el: HTMLElement, id: string) =>
    el.querySelector<HTMLElement>(`section[aria-labelledby="${id}"]`);
  const items = (el: Element | null, kind: string) => [
    ...(el?.querySelectorAll<HTMLElement>(`mc-attention-card[data-kind="${kind}"]`) ?? []),
  ];
  const title = (card: Element | undefined) => text(card?.querySelector('.title'));
  const why = (card: Element | undefined) => text(card?.querySelector('.why'));
  const button = (el: Element | undefined, label: string) =>
    [...(el?.querySelectorAll<HTMLElement>('button, a') ?? [])].find((b) => text(b) === label);
  const store = (pending: PendingPreview[]) =>
    (
      TestBed.inject(RealtimeService) as unknown as { replaceStore(l: unknown[]): void }
    ).replaceStore([
      {
        id: 's1',
        name: 's1',
        title: 'Fix tray icon',
        live: true,
        owned: true,
        status: 'waiting',
        activity: { kind: 'waiting', label: 'Waiting' },
        pending,
      },
    ]);

  it('lists a device that wants to pair, answered with its number on the desktop', async () => {
    asDesktop();
    backend.routes.set('/devices', () => ({
      status: 200,
      body: { devices: [], pending: [pairRequest()] },
    }));
    backend.routes.set('/devices/pending/pp_1/deny', () => ({ status: 200, body: {} }));
    TestBed.inject(PairRequestsService).start();
    const el = await render();
    const needs = section(el, 'inbox-needs');
    expect(text(needs?.querySelector('h2'))).toBe('Needs you 1');
    const [card] = items(needs, 'pair');
    expect(title(card)).toBe("Marcus's iPhone wants to pair");
    expect(text(card.querySelector('.meta'))).toBe('iOS · via local network');
    expect(why(card)).toBe("The device shows a 4-digit number: you'll type it to approve.");

    button(card, 'Approve…')!.click();
    fixture.detectChanges();
    expect((el.querySelector('mc-pair-approval-dialog dialog') as HTMLDialogElement).open).toBe(
      true,
    );
    button(card, 'Deny')!.click();
    await settle();
    expect(backend.calls).toContain('POST /devices/pending/pp_1/deny');
    expect(items(el, 'pair')).toEqual([]);
  });

  it('never asks a phone about pairing, which only the desktop approves', async () => {
    backend.routes.set('/devices', () => ({
      status: 200,
      body: { devices: [], pending: [pairRequest()] },
    }));
    const el = await render();
    expect(items(el, 'pair')).toEqual([]);
    expect(backend.calls).not.toContain('GET /devices');
  });

  it('shows what another computer asked: answered on the desktop, shown to a phone', async () => {
    TestBed.inject(MachinesService).apply(
      machinesView({ enabled: true, peers: [peer()], asks: [machineAsk()] }),
    );
    let el = await render();
    const asks = section(el, 'inbox-needs')?.querySelector('mc-machine-asks');
    expect(text(asks)).toContain('Sign in to Work');
    expect(text(asks)).toContain('From DESKTOP-PC');
    expect(button(asks!, 'Sign in…')).toBeUndefined();

    fixture.destroy();
    asDesktop();
    el = await render();
    expect(button(el.querySelector('mc-machine-asks')!, 'Sign in…')).toBeDefined();
    expect(text(section(el, 'inbox-needs')?.querySelector('h2'))).toBe('Needs you 1');
  });

  it("puts a failing check to fix: this computer's runs its fix, another's is asked there", async () => {
    asDesktop();
    const { view, setup } = failingSetup();
    TestBed.inject(MachinesService).apply(view);
    TestBed.inject(MachinesService).setup.set(setup);
    backend.routes.set('/machines/setup/fix', () => ({ status: 200, body: setup }));
    backend.routes.set('/machines/setup/ask', () => ({ status: 200, body: { asked: true } }));
    const el = await render();
    const fix = section(el, 'inbox-fix');
    expect(text(fix?.querySelector('h2'))).toBe('To fix 2');
    const [here, there] = items(fix, 'check');
    expect(title(here)).toBe('Stays awake while used');
    expect(why(here)).toBe('It sleeps after 10 minutes.');
    expect(text(here.querySelector('.mark'))).toBe('✕');
    button(here, 'Keep awake while used')!.click();
    await settle();
    expect(backend.bodyOf('POST /machines/setup/fix')).toEqual({ check: 'keepAwake' });

    expect(title(there)).toBe('The firewall');
    expect(text(there.querySelector('.mchip'))).toBe('DESKTOP-PC');
    button(there, 'Ask DESKTOP-PC to fix')!.click();
    await settle();
    expect(backend.bodyOf('POST /machines/setup/ask')).toEqual({
      machine: PEER_ID,
      check: 'firewall',
    });
  });

  it('shows a phone the checks, fixed on the desktop', async () => {
    const { view, setup } = failingSetup();
    TestBed.inject(MachinesService).apply(view);
    TestBed.inject(MachinesService).setup.set(setup);
    const el = await render();
    const [here] = items(el, 'check');
    expect(button(here, 'Keep awake while used')).toBeUndefined();
    expect(text(here.querySelector('mc-desktop-only'))).toBe('Fix it on the desktop app.');
  });

  it('names a computer whose versions are apart, and asks it to update', async () => {
    asDesktop();
    backend.routes.set('/machines/setup/ask', () => ({ status: 200, body: { asked: true } }));
    TestBed.inject(MachinesService).apply(
      machinesView({
        enabled: true,
        peers: [
          peer({
            versions: { app: '0.10.9', protocol: 1, claude: '2.1.322' },
            skew: { app: 'older' },
          }),
        ],
      }),
    );
    const el = await render();
    const [card] = items(el, 'skew');
    expect(title(card)).toBe('DESKTOP-PC needs an update');
    expect(why(card)).toBe(
      'DESKTOP-PC runs an older Mother Claude (0.10.9; this computer: 0.11.0).',
    );
    button(card, 'Ask DESKTOP-PC to update')!.click();
    await settle();
    expect(backend.bodyOf('POST /machines/setup/ask')).toEqual({ machine: PEER_ID, check: 'app' });
  });

  it('says a project changed on both computers, and where to choose', async () => {
    asDesktop();
    TestBed.inject(SyncService).apply({ pairs: [conflicted()], watching: true });
    const el = await render();
    const [card] = items(el, 'sync');
    expect(title(card)).toBe('tray-tool changed on both computers');
    expect(text(card.querySelector('.mchip'))).toBe('DESKTOP-PC');
    expect(button(card, 'Choose…')?.getAttribute('href')).toBe('/computers/sync');
  });

  it('says a copy kept on another computer is paused, for a reason someone can end', async () => {
    TestBed.inject(CopiesService).apply({
      enabled: true,
      held: [],
      kept: [keptCopy('noGrant'), keptCopy('offline', { id: 'kept.b' })],
      capBytes: 0,
      used: {},
    });
    const el = await render();
    const cards = items(el, 'copy');
    expect(cards.length).toBe(1);
    expect(title(cards[0])).toBe('The copy of Release notes on DESKTOP-PC is paused');
    expect(why(cards[0])).toBe('The other computer has not given this one the Move permission.');
    expect(button(cards[0], 'Open Computers')?.getAttribute('href')).toBe('/computers#copies');
  });

  it('puts a failed backup to fix: tried again, or put away on this device', async () => {
    serveAccounts(backend, buildAccounts(Date.now(), true));
    const failedMove: JobView = job({
      id: 'job_move',
      kind: 'move',
      account: undefined,
      from: 'default',
      to: MOCK_ACCOUNT_WORK,
      error: 'a Claude Code process is using the directory',
      // After the backup's (an hour ago): failures are listed as they came.
      finishedAt: Date.now() - 30 * 60_000,
    });
    backend.routes.set('/data/jobs', () => ({
      status: 200,
      body: { jobs: [job(), failedMove] },
    }));
    backend.routes.set('/backup/push', () => ({ status: 202, body: { jobs: ['job_2'] } }));
    backend.routes.set('/data/jobs/job_2', () => ({
      status: 200,
      body: job({ id: 'job_2', state: 'done', startedAt: Date.now() }),
    }));
    const el = await render();
    const [backup, move] = items(el, 'job');
    expect(title(backup)).toBe('Backup of Work failed');
    expect(why(backup)).toBe('The backup folder was offline.');
    expect(title(move)).toBe('Move from Default to Work failed');
    expect(button(move, 'Details')?.getAttribute('href')).toBe('/accounts/changes?show=moves');

    button(backup, 'Try again')!.click();
    await settle();
    expect(backend.bodyOf('POST /backup/push')).toEqual({ account: MOCK_ACCOUNT_WORK });
    // It ran through: the failure is gone.
    expect(items(el, 'job').map(title)).toEqual(['Move from Default to Work failed']);

    button(items(el, 'job')[0], 'Dismiss')!.click();
    fixture.detectChanges();
    expect(items(el, 'job')).toEqual([]);
    expect(localStorage.getItem('mc_inbox_dismissed')).toContain('job:job_move');
  });

  it('says an account lost its sign-in, or signed in as someone else, and keeps it on the desktop', async () => {
    asDesktop();
    const view = buildAccounts(Date.now(), false);
    view.accounts.push(
      accountFixture({ id: 'acc_1', name: 'Work', attention: 'signedOut', signedIn: false }),
      accountFixture({ id: 'acc_2', name: 'Client X', identityWarning: { kind: 'changed' } }),
    );
    serveAccounts(backend, view);
    backend.routes.set('/accounts/acc_2/identity/accept', () => ({
      status: 200,
      body: view.accounts[2],
    }));
    const el = await render();
    const [out, someone] = items(el, 'signIn');
    expect(title(out)).toBe('Work is signed out');
    expect(button(out, 'Sign in…')?.getAttribute('href')).toBe('/accounts');
    expect(title(someone)).toBe('Client X signed in as someone else');
    expect(why(someone)).toBe('It signed in as a different Claude account than before.');
    button(someone, 'Keep it')!.click();
    await settle();
    expect(backend.calls).toContain('POST /accounts/acc_2/identity/accept');
  });

  it('puts an update and a missing permission to fix on the desktop, never on a phone', async () => {
    TestBed.inject(UpdaterService).status.set('available');
    TestBed.inject(UpdaterService).newVersion.set('0.12.0');
    TestBed.inject(PermissionsService).permissions.set([permission()]);
    let el = await render();
    expect(items(el, 'update')).toEqual([]);
    expect(items(el, 'permission')).toEqual([]);

    fixture.destroy();
    asDesktop();
    el = await render();
    const [perm] = items(el, 'permission');
    expect(title(perm)).toBe('Full Disk Access is missing');
    expect(button(perm, 'Open System Settings')).toBeDefined();
    const [update] = items(el, 'update');
    expect(title(update)).toBe('Mother Claude 0.12.0 is ready');
    expect(text(update.querySelector('.mark'))).toBe('●');
    expect(button(update, 'Install…')?.getAttribute('href')).toBe('/settings#updates');
  });

  it('puts what another computer asked that failed here to fix, until put away', async () => {
    TestBed.inject(MachinesService).apply(
      machinesView({
        enabled: true,
        peers: [peer()],
        asks: [
          machineAsk({
            kind: 'clone',
            title: 'side-project',
            state: 'failed',
            detail: 'The folder exists already.',
            answeredAt: Date.now() - 60_000,
          }),
        ],
      }),
    );
    const el = await render();
    const [card] = items(el, 'ask');
    expect(title(card)).toBe('Clone side-project');
    expect(why(card)).toBe('The folder exists already.');
    button(card, 'Dismiss')!.click();
    fixture.detectChanges();
    expect(items(el, 'ask')).toEqual([]);
  });

  it('keeps what can only be answered where it runs under Waiting where it runs', async () => {
    store([request('q1'), request('q2', false)]);
    const el = await render();
    expect(text(section(el, 'inbox-needs')?.querySelector('h2'))).toBe('Needs you 1');
    const desk = section(el, 'inbox-desk');
    expect(text(desk?.querySelector('h2'))).toBe('Waiting where it runs 1');
    expect(desk?.querySelectorAll('mc-pending-card').length).toBe(1);
  });

  it('filters by computer beside the account: this one, or another', async () => {
    asDesktop();
    TestBed.inject(MachinesService).apply(
      machinesView({ enabled: true, peers: [peer({ skew: { app: 'newer' } })] }),
    );
    TestBed.inject(SyncService).apply({ pairs: [conflicted()], watching: true });
    TestBed.inject(UpdaterService).status.set('available');
    store([request('q1')]);
    const el = await render();
    const sel = el.querySelector<HTMLSelectElement>('select[aria-label="Computer"]')!;
    expect([...sel.options].map((o) => o.text.trim())).toEqual([
      'All computers',
      'This computer',
      'DESKTOP-PC',
    ]);
    const kinds = () =>
      [...el.querySelectorAll('mc-attention-card')].map((c) => c.getAttribute('data-kind'));
    expect(kinds()).toEqual(['skew', 'sync', 'update']);

    sel.value = PEER_ID;
    sel.dispatchEvent(new Event('change'));
    fixture.detectChanges();
    expect(kinds()).toEqual(['skew', 'sync']);
    expect(el.querySelectorAll('mc-pending-card').length).toBe(0);

    sel.value = 'here';
    sel.dispatchEvent(new Event('change'));
    fixture.detectChanges();
    expect(kinds()).toEqual(['update']);
    expect(el.querySelectorAll('mc-pending-card').length).toBe(1);
  });

  it('says what a filter hides, and shows everything again', async () => {
    TestBed.inject(MachinesService).apply(machinesView({ enabled: true, peers: [peer()] }));
    TestBed.inject(SyncService).apply({ pairs: [conflicted()], watching: true });
    const el = await render();
    const sel = el.querySelector<HTMLSelectElement>('select[aria-label="Computer"]')!;
    sel.value = 'here';
    sel.dispatchEvent(new Event('change'));
    fixture.detectChanges();
    expect(text(el.querySelector('.empty-state strong'))).toBe(
      'Nothing on this computer needs you',
    );
    expect(text(el.querySelector('.empty-state'))).toContain(
      '1 more item waits outside this filter.',
    );
    button(el.querySelector('.empty-state')!, 'Show everything')!.click();
    fixture.detectChanges();
    expect(items(el, 'sync').length).toBe(1);
  });
});
