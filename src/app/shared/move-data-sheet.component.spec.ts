import { ComponentFixture, TestBed } from '@angular/core/testing';
import { provideRouter } from '@angular/router';

import { AuthService } from '../core/auth.service';
import { DataJobsService } from '../core/data-jobs.service';
import {
  FakeBackend,
  asDesktop,
  flush,
  me,
  serveAccounts,
  useFakeServer,
} from '../core/mock/fake-server.testing';
import { MOCK_ACCOUNT_PRO, MOCK_ACCOUNT_WORK, buildAccounts } from '../core/mock/fixtures';
import { setActiveMock } from '../core/mock/mock-api';
import { AccountRef, JobView, SessionSummary } from '../core/models';
import { accountLocation, setLocation } from '../core/move-data';
import { MoveDataSheetComponent, MoveStart } from './move-data-sheet.component';

const DEFAULT_REF: AccountRef = {
  id: 'default',
  name: 'Default',
  color: '#8b5cf6',
  kind: 'directory',
};
const WORK_REF: AccountRef = {
  id: MOCK_ACCOUNT_WORK,
  name: 'Work',
  color: '#0ea5e9',
  kind: 'directory',
};
const DRIVE = 'stor_5d2c8e1f0a7b';

const session = (id: string, over: Partial<SessionSummary> = {}): SessionSummary =>
  ({
    id,
    name: id,
    title: `Title of ${id}`,
    projectName: 'api',
    cwd: '/Users/me/dev/api',
    live: false,
    status: 'ended',
    lastActivityAt: 1_000,
    account: DEFAULT_REF,
    ...over,
  }) as SessionSummary;

const backupView = {
  settings: { folder: '/Users/me/iCloud/Mother Claude', everyHours: 6, pullOnLaunch: false },
  passphraseSet: true,
  passphraseChangeUnfinished: false,
  host: 'mbp',
  sets: [],
  suggestions: [],
  jobs: [],
  storage: [
    {
      id: 'backup',
      label: 'iCloud Drive',
      path: '/Users/me/iCloud/Mother Claude',
      kind: 'icloud',
      primary: true,
      available: true,
      sets: [
        { name: 'Default', account: 'default', exists: true, sessions: 12 },
        { name: 'Work', account: MOCK_ACCOUNT_WORK, exists: false },
      ],
    },
    {
      id: DRIVE,
      label: 'Google Drive',
      path: '/Users/me/Drive/Mother Claude',
      kind: 'gdrive',
      primary: false,
      available: true,
      sets: [{ name: 'Shared', exists: true, sessions: 2, archived: 1 }],
    },
  ],
};

const job = (over: Partial<JobView> = {}): JobView => ({
  id: 'job1',
  kind: 'copy',
  state: 'running',
  startedAt: Date.now(),
  filesDone: 0,
  filesTotal: 3,
  bytesDone: 0,
  bytesTotal: 3072,
  sessions: ['s1'],
  from: 'default',
  to: MOCK_ACCOUNT_WORK,
  ...over,
});

describe('MoveDataSheetComponent', () => {
  let backend: FakeBackend;
  let fixture: ComponentFixture<MoveDataSheetComponent>;

  beforeEach(() => {
    backend = new FakeBackend();
    useFakeServer(backend, [provideRouter([])]);
    serveAccounts(backend, buildAccounts(Date.now(), true));
    backend.routes.set('/backup', () => ({ status: 200, body: backupView }));
    backend.routes.set('/sessions', () => ({
      status: 200,
      body: {
        sessions: [
          session('s1'),
          session('s2', { title: 'Live one', live: true, cwd: '/Users/me/dev/web' }),
          session('w1', { account: WORK_REF, title: 'Work thing' }),
        ],
      },
    }));
    asDesktop();
    TestBed.inject(DataJobsService).pollMs = 1;
  });

  afterEach(() => setActiveMock(null));

  const settle = async (times = 4) => {
    for (let i = 0; i < times; i++) {
      await flush();
      fixture.detectChanges();
    }
  };
  const open = async (start: MoveStart) => {
    fixture = TestBed.createComponent(MoveDataSheetComponent);
    fixture.componentRef.setInput('start', start);
    fixture.componentRef.setInput('open', true);
    fixture.detectChanges();
    await settle();
    return fixture.nativeElement as HTMLElement;
  };
  const text = (el: Element | null | undefined) =>
    (el?.textContent ?? '').replace(/\s+/g, ' ').trim();
  const button = (el: HTMLElement, label: string) =>
    [...el.querySelectorAll<HTMLButtonElement>('button')].find((b) => text(b).startsWith(label));
  const select = (el: HTMLElement, name: string) =>
    el.querySelector<HTMLSelectElement>(`select[name="${name}"]`)!;
  const choose = async (el: HTMLElement, name: string, value: string) => {
    const s = select(el, name);
    s.value = value;
    s.dispatchEvent(new Event('change'));
    await settle();
  };
  const options = (el: HTMLElement, name: string) =>
    [...select(el, name).querySelectorAll('optgroup')].map((g) => [
      g.label,
      [...g.querySelectorAll('option')].map((o) => text(o)),
    ]);
  const lastMove = () => backend.bodyOf('POST /data/move') as Record<string, unknown>;

  it('copies a conversation to another account, then follows the job and undoes it', async () => {
    let polls = 0;
    backend.routes.set('/data/move', (_m, body) =>
      (body as { dryRun?: boolean }).dryRun
        ? {
            status: 200,
            body: {
              plan: {
                from: 'default',
                to: MOCK_ACCOUNT_WORK,
                mode: 'copy',
                jobs: 1,
                sessions: [
                  {
                    id: 's1',
                    cwd: '/Users/me/dev/api',
                    title: 'Title of s1',
                    files: 3,
                    bytes: 3072,
                    action: 'copy',
                  },
                ],
                skipped: [],
                files: 3,
                bytes: 3072,
              },
            },
          }
        : { status: 202, body: { jobs: [job()], job: job() } },
    );
    backend.routes.set('/data/jobs/job1', () => ({
      status: 200,
      body:
        ++polls < 2
          ? job({ bytesDone: 1024 })
          : job({
              state: 'done',
              bytesDone: 3072,
              undoUntil: Date.now() + 7 * 86_400_000,
              result: { copied: ['s1'], skipped: [] },
            }),
    }));
    backend.routes.set('/data/jobs/job1/undo', () => ({
      status: 200,
      body: job({ state: 'undone', finishedAt: Date.now() }),
    }));
    const el = await open({
      from: accountLocation('default'),
      what: 'sessions',
      sessions: ['s1'],
      known: [session('s1')],
    });

    // The accounts, then each storage with its sets.
    expect(options(el, 'from')).toEqual([
      [
        'Accounts',
        ['Default — shared folder', 'Work — own folder', 'Marcus (Pro) — shared folder'],
      ],
      ['iCloud Drive (backup folder)', ['Default']],
      ['Google Drive', ['Shared']],
    ]);
    expect(options(el, 'to')[1]).toEqual([
      'iCloud Drive (backup folder)',
      ['Default', 'Work', 'New set…'],
    ]);
    // Another account's directory first.
    expect(select(el, 'to').value).toBe(`a:${MOCK_ACCOUNT_WORK}`);
    expect(text(el)).toContain('The shared folder');
    expect(text(el.querySelector('mc-move-checklist'))).toContain('1 of 2 selected');
    expect(text(el.querySelector('mc-move-checklist'))).toContain('Running');

    button(el, 'Check what it does')?.click();
    await settle();
    expect(lastMove()).toEqual({
      from: 'default',
      to: MOCK_ACCOUNT_WORK,
      items: { sessions: ['s1'] },
      mode: 'copy',
      trust: false,
      replaceExisting: false,
      dryRun: true,
    });
    expect(text(el.querySelector('.planline'))).toBe(
      'Copies 1 conversation (3 files, 3.0 KB) into Work.',
    );
    expect(text(el)).toContain('can be undone for seven days');

    button(el, 'Copy 1 conversation')?.click();
    await settle();
    expect(lastMove()['dryRun']).toBeFalse();
    await new Promise((r) => setTimeout(r, 20));
    await settle();
    const row = el.querySelector('mc-data-job');
    expect(row?.getAttribute('data-state')).toBe('done');
    expect(text(row)).toContain('Copy from Default to Work');
    expect(text(row)).toContain('Copied 1 conversation');

    button(el, 'Undo')?.click();
    fixture.detectChanges();
    button(el, 'Tap again to undo')?.click();
    await settle();
    expect(backend.calls).toContain('POST /data/jobs/job1/undo');
    expect(el.querySelector('mc-data-job')?.getAttribute('data-state')).toBe('undone');
  });

  it('assigns between accounts of the shared folder: nothing moves, copy is not offered', async () => {
    const el = await open({ from: accountLocation('default'), what: 'all' });
    await choose(el, 'to', `a:${MOCK_ACCOUNT_PRO}`);
    expect(button(el, 'Copy')?.disabled).toBeTrue();
    expect(button(el, 'Move')?.getAttribute('aria-pressed')).toBe('true');
    expect(text(el)).toContain("Nothing moves: it is recorded as Marcus (Pro)'s");
    expect(text(el)).toContain('a copy there would be the same conversation');
    // The folder is the same one: its trust means nothing.
    expect(text(el)).not.toContain('Also trust their folders');
    expect(text(el.querySelector('.allline'))).toContain(
      "Every conversation recorded as Default's",
    );
    backend.routes.set('/data/move', () => ({ status: 200, body: { plan: { mode: 'move' } } }));
    button(el, 'Check what it does')?.click();
    await settle();
    expect(lastMove()).toEqual(
      jasmine.objectContaining({ to: MOCK_ACCOUNT_PRO, items: { all: true }, mode: 'move' }),
    );
  });

  it('archives into a storage set, listing what is skipped and why', async () => {
    backend.routes.set('/data/move', () => ({
      status: 200,
      body: {
        plan: {
          from: 'default',
          to: { storage: DRIVE, set: 'Shared' },
          mode: 'move',
          sessions: [{ id: 's1', title: 'Title of s1', files: 3, bytes: 4096, action: 'archive' }],
          skipped: [
            { id: 's2', code: 'session_live', message: 'session s2 is running; stop it first' },
          ],
          files: 3,
          bytes: 4096,
        },
      },
    }));
    const el = await open({
      from: accountLocation('default'),
      what: 'sessions',
      sessions: ['s1', 's2'],
    });
    await choose(el, 'to', `s:${DRIVE}:Shared`);
    expect(text(el)).toContain(
      '~/Drive/Mother Claude/Shared · 2 conversations · 1 in storage only',
    );
    button(el, 'Move')?.click();
    fixture.detectChanges();
    expect(text(el)).toContain('Archived: it goes into Google Drive · Shared');
    expect(text(el)).not.toContain('Also trust their folders');
    button(el, 'Check what it does')?.click();
    await settle();
    expect(lastMove()).toEqual(
      jasmine.objectContaining({
        from: 'default',
        to: { storage: DRIVE, set: 'Shared' },
        items: { sessions: ['s1', 's2'] },
        mode: 'move',
      }),
    );
    expect(text(el.querySelector('.planline'))).toBe(
      'Archives 1 conversation (3 files, 4.0 KB) into Google Drive · Shared.',
    );
    const skipped = text(el.querySelector('.skips'));
    expect(skipped).toContain('Live one');
    expect(skipped).toContain('Session s2 is running; stop it first.');
    expect(button(el, 'Archive 1 conversation')).toBeDefined();
  });

  it('says a copy into a storage is a backup with nothing to undo', async () => {
    backend.routes.set('/data/move', () => ({
      status: 200,
      body: { plan: { mode: 'copy', sessions: [{ id: 's1', action: 'copy' }] } },
    }));
    const el = await open({
      from: accountLocation('default'),
      to: setLocation('backup', 'Default'),
      what: 'sessions',
      sessions: ['s1'],
    });
    button(el, 'Check what it does')?.click();
    await settle();
    expect(text(el)).toContain('A copy into a storage is a backup: there is nothing to undo.');
  });

  it("lists a set's conversations from its index, archived ones as in storage only", async () => {
    backend.routes.set('/backup/index', () => ({
      status: 200,
      body: {
        name: 'Shared',
        set: '/Users/me/Drive/Mother Claude/Shared',
        sessions: [
          {
            id: 'aaaa1111',
            action: 'new',
            title: 'Old notes',
            remoteBytes: 2048,
            archived: { at: 1, machine: 'm' },
          },
          { id: 'bbbb2222', action: 'same', title: 'Invoice layout', remoteBytes: 1024 },
        ],
      },
    }));
    const el = await open({ from: setLocation(DRIVE, 'Shared'), what: 'sessions' });
    expect(backend.urls).toContain(`/backup/index?storage=${DRIVE}&set=Shared`);
    // Out of a set: the Default, as no account is named after it.
    expect(select(el, 'to').value).toBe('a:default');
    const list = text(el.querySelector('mc-move-checklist'));
    expect(list).toContain('Old notes');
    expect(list).toContain('In storage only');
    expect(text(el)).toContain('Restored into Default; Google Drive · Shared keeps it.');
    const boxes = el.querySelectorAll<HTMLInputElement>('mc-move-checklist input[type="checkbox"]');
    boxes[0].click();
    await settle();
    backend.routes.set('/data/move', () => ({ status: 200, body: { plan: { mode: 'copy' } } }));
    button(el, 'Check what it does')?.click();
    await settle();
    expect(lastMove()).toEqual(
      jasmine.objectContaining({
        from: { storage: DRIVE, set: 'Shared' },
        to: 'default',
        items: { sessions: ['aaaa1111'] },
      }),
    );
  });

  it('takes conversations from where each one is when From is left out', async () => {
    backend.routes.set('/data/move', () => ({
      status: 200,
      body: {
        plan: {
          mode: 'copy',
          jobs: 2,
          sessions: [{ id: 's1' }, { id: 'w1', from: MOCK_ACCOUNT_WORK }],
        },
      },
    }));
    const el = await open({
      from: null,
      what: 'sessions',
      sessions: ['s1', 'w1'],
      known: [session('s1'), session('w1', { account: WORK_REF, title: 'Work thing' })],
    });
    expect(text(select(el, 'from').selectedOptions[0])).toBe('Where each one is');
    // Only conversations can go from where each one is.
    expect(button(el, 'Projects')?.disabled).toBeTrue();
    await choose(el, 'to', 'a:default');
    button(el, 'Check what it does')?.click();
    await settle();
    const body = lastMove();
    expect('from' in body).toBeFalse();
    expect(body['items']).toEqual({ sessions: ['s1', 'w1'] });
    expect(text(el)).toContain('It runs as 2 jobs');
    expect(text(el.querySelector('.plist'))).toContain('from Work');

    // Away from "where each one is" the picks go; back there they return.
    button(el, 'Back')?.click();
    await settle();
    await choose(el, 'from', 'a:default');
    expect(text(el.querySelector('mc-move-checklist'))).toContain('0 of');
    await choose(el, 'from', '');
    expect(text(el.querySelector('mc-move-checklist'))).toContain('2 of 2 selected');
  });

  it('copies customizations only between accounts in different folders', async () => {
    backend.routes.set('/data/move', () => ({
      status: 200,
      body: {
        plan: {
          mode: 'copy',
          customizations: [
            { item: 'skills', name: 'review', action: 'add' },
            {
              item: 'settings',
              name: 'settings.json',
              action: 'skip',
              reason: 'the target has its own',
            },
          ],
          warnings: ['settings.json runs commands'],
        },
      },
    }));
    const el = await open({ from: accountLocation('default'), what: 'customizations' });
    expect(button(el, 'Move')?.disabled).toBeTrue();
    expect(text(el)).toContain('Customizations are only ever copied.');
    const boxes = [...el.querySelectorAll<HTMLInputElement>('.custom input[type="checkbox"]')];
    expect(boxes.length).toBe(9);
    boxes[1].click(); // settings
    boxes[2].click(); // skills
    const replace = [...el.querySelectorAll<HTMLInputElement>('.check input')].pop()!;
    replace.click();
    await settle();
    button(el, 'Check what it does')?.click();
    await settle();
    expect(lastMove()).toEqual({
      from: 'default',
      to: MOCK_ACCOUNT_WORK,
      items: { customizations: ['settings', 'skills'] },
      mode: 'copy',
      trust: false,
      replaceExisting: true,
      dryRun: true,
    });
    expect(text(el)).toContain('Skills: review');
    expect(text(el)).toContain('Left as it is: the target has its own');
    expect(text(el.querySelector('.warns'))).toContain('settings.json runs commands');
    expect(button(el, 'Copy 1 customization')).toBeDefined();

    // Into a storage they can't go.
    button(el, 'Back')?.click();
    await settle();
    await choose(el, 'to', `s:${DRIVE}:Shared`);
    expect(text(el)).toContain(
      'Customizations are copied only between accounts in different folders.',
    );
    expect(button(el, 'Check what it does')?.disabled).toBeTrue();
  });

  it('keeps the dangerous parts from a device without the right, saying why', async () => {
    TestBed.inject(AuthService).me.set(me({ allowDangerous: false }));
    const el = await open({
      from: accountLocation('default'),
      what: 'projects',
      projects: ['/Users/me/dev/api'],
    });
    const trust = el.querySelector<HTMLInputElement>('.check input')!;
    expect(trust.disabled).toBeTrue();
    expect(text(el.querySelector('.check'))).toContain(
      'Needs dangerous actions for this device, granted on the Mac in Remote › Devices.',
    );
    button(el, 'Customizations')?.click();
    await settle();
    const boxes = [...el.querySelectorAll<HTMLInputElement>('.custom input[type="checkbox"]')];
    expect(boxes.filter((b) => b.disabled).length).toBe(2);
    expect(text(el.querySelectorAll('.custom li')[1])).toContain('Remote › Devices');
  });

  it('moves into a new set, named in the sheet', async () => {
    const el = await open({
      from: accountLocation(MOCK_ACCOUNT_WORK),
      to: setLocation(DRIVE, ''),
      what: 'all',
    });
    expect(select(el, 'to').value).toBe(`n:${DRIVE}`);
    expect(button(el, 'Check what it does')?.disabled).toBeTrue();
    const name = el.querySelector<HTMLInputElement>('input[name="newSet"]')!;
    name.value = 'shared';
    name.dispatchEvent(new Event('input'));
    await settle();
    expect(text(el)).toContain('There is a set with this name already');
    name.value = 'Archive 2026';
    name.dispatchEvent(new Event('input'));
    await settle();
    backend.routes.set('/data/move', () => ({ status: 200, body: { plan: { mode: 'copy' } } }));
    button(el, 'Check what it does')?.click();
    await settle();
    expect(lastMove()).toEqual(
      jasmine.objectContaining({
        from: MOCK_ACCOUNT_WORK,
        to: { storage: DRIVE, set: 'Archive 2026' },
        items: { all: true },
      }),
    );
  });

  it("puts the server's refusals in words and stays where it was", async () => {
    backend.routes.set('/data/move', () => ({
      status: 409,
      body: { error: { code: 'set_key_missing', message: 'the backup set in /x has no key' } },
    }));
    const el = await open({
      from: accountLocation('default'),
      to: setLocation('backup', 'Default'),
      what: 'all',
    });
    button(el, 'Check what it does')?.click();
    await settle();
    expect(text(el.querySelector('.notice.error'))).toContain(
      "The backup's key (identity.age) isn't in the folder",
    );
    expect(button(el, 'Check what it does')).toBeDefined();
  });
});
