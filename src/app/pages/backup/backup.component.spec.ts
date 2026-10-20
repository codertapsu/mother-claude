import { ComponentFixture, TestBed } from '@angular/core/testing';
import { provideRouter } from '@angular/router';

import { DataJobsService } from '../../core/data-jobs.service';
import {
  FakeBackend,
  asDesktop,
  flush,
  serveAccounts,
  useFakeServer,
} from '../../core/mock/fake-server.testing';
import { MOCK_ACCOUNT_PRO, MOCK_ACCOUNT_WORK, buildAccounts } from '../../core/mock/fixtures';
import { setActiveMock } from '../../core/mock/mock-api';
import { BackupView, JobView } from '../../core/models';
import { BackupComponent } from './backup.component';

const H = 3_600_000;
const ICLOUD = '/Users/me/Library/Mobile Documents/com~apple~CloudDocs/Mother Claude';

const view = (over: Partial<BackupView> = {}): BackupView => ({
  settings: { folder: null, everyHours: 0, pullOnLaunch: false },
  passphraseSet: false,
  passphraseChangeUnfinished: false,
  host: 'marcus-mbp',
  sets: [],
  suggestions: [{ label: 'iCloud Drive', path: ICLOUD }],
  jobs: [],
  ...over,
});

const ready = (): BackupView =>
  view({
    settings: { folder: ICLOUD, everyHours: 6, pullOnLaunch: false },
    passphraseSet: true,
    sets: [
      {
        account: 'default',
        name: 'Default',
        dir: `${ICLOUD}/Default`,
        exists: true,
        keyMissing: false,
        lastPush: Date.now() - 2 * H,
        sessions: 143,
        machines: [
          { id: 'm-this', host: 'marcus-mbp', lastPush: Date.now() - 2 * H },
          { id: 'm-air', host: 'marcus-air', lastPush: Date.now() - 5 * H },
        ],
      },
      {
        account: MOCK_ACCOUNT_WORK,
        name: 'Work',
        dir: `${ICLOUD}/Work`,
        exists: false,
        keyMissing: false,
        lastError: 'no backup passphrase is set',
        machines: [],
      },
      // A token account: its sessions are the Default's, so is its set.
      {
        account: MOCK_ACCOUNT_PRO,
        name: 'Marcus (Pro)',
        dir: `${ICLOUD}/Default`,
        exists: true,
        sharedWith: 'default',
        keyMissing: false,
        lastPush: Date.now() - 2 * H,
        sessions: 143,
        machines: [{ id: 'm-this', host: 'marcus-mbp', lastPush: Date.now() - 2 * H }],
      },
    ],
  });

const backupJob = (id: string, state: JobView['state'], account = 'default'): JobView => ({
  id,
  kind: 'backup',
  state,
  startedAt: Date.now(),
  filesDone: 0,
  filesTotal: 0,
  bytesDone: 0,
  bytesTotal: 0,
  sessions: [],
  account,
  ...(state === 'done'
    ? {
        result: {
          archived: ['a'],
          unchanged: 3,
          skippedLive: [],
          failed: [],
          conflicts: [],
          newerInSet: [],
          config: null,
          bytes: 1,
          warnings: [],
          removed: 0,
        },
      }
    : {}),
});

describe('BackupComponent', () => {
  let backend: FakeBackend;
  let fixture: ComponentFixture<BackupComponent>;
  let current: BackupView;

  beforeEach(() => {
    backend = new FakeBackend();
    useFakeServer(backend, [provideRouter([])]);
    serveAccounts(backend, buildAccounts(Date.now(), true));
    TestBed.inject(DataJobsService).pollMs = 1;
    current = view();
    backend.routes.set('/backup', () => ({ status: 200, body: current }));
  });

  afterEach(() => setActiveMock(null));

  const render = async () => {
    fixture = TestBed.createComponent(BackupComponent);
    fixture.detectChanges();
    await settle();
    return fixture.nativeElement as HTMLElement;
  };
  const settle = async (times = 3) => {
    for (let i = 0; i < times; i++) {
      await flush();
      fixture.detectChanges();
    }
  };
  const text = (el: Element | null | undefined) =>
    (el?.textContent ?? '').replace(/\s+/g, ' ').trim();
  const button = (el: HTMLElement, label: string) =>
    [...el.querySelectorAll<HTMLButtonElement>('button')].find((b) => text(b).startsWith(label));

  it('shows a phone how backups stand, with nothing to change', async () => {
    current = ready();
    const el = await render();
    expect(text(el.querySelector('.phone-note'))).toContain('set up and run on the Mac');
    expect(text(el)).toContain("Set, in this Mac's Keychain");
    expect(text(el)).toContain('Every 6 hours');
    expect(text(el.querySelector('.sets'))).toContain('Backed up from this Mac');
    expect(text(el.querySelector('.sets'))).toContain('143 sessions');
    // Other Macs that pushed, not this one.
    expect(text(el.querySelector('.sets'))).toContain('Also from marcus-air');
    expect(text(el.querySelector('.sets'))).not.toContain('Also from marcus-mbp');
    expect(text(el.querySelector('.sets'))).toContain('No backup passphrase is set.');
    // A token account's sessions are in the Default's set.
    const pro = [...el.querySelectorAll('.setrow')].find(
      (r) => text(r.querySelector('.sname')) === 'Marcus (Pro)',
    );
    expect(text(pro?.querySelector('.smain > span'))).toBe(
      "Its sessions are in Default's backup: a token account uses the Default account's directory.",
    );
    expect(text(pro)).not.toContain('143 sessions');
    // Nothing that changes anything.
    expect(el.querySelectorAll('button').length).toBe(0);
    expect(el.querySelector('select')).toBeNull();
    expect(el.querySelector('input')).toBeNull();
  });

  it("gives a token account's row no buttons: its set is the Default's", async () => {
    asDesktop();
    current = ready();
    const el = await render();
    const labels = [...el.querySelectorAll('.sacts .btn')].map((b) => b.getAttribute('aria-label'));
    expect(labels).toContain('Back up Default');
    expect(labels).toContain('Back up Work');
    expect(labels.some((l) => (l ?? '').includes('Marcus (Pro)'))).toBeFalse();
  });

  it("says when a set's key is not in the folder here, and checks again", async () => {
    asDesktop();
    current = ready();
    current.sets[1] = { ...current.sets[1], keyMissing: true, lastError: undefined };
    const el = await render();
    const work = [...el.querySelectorAll('.setrow')].find(
      (r) => text(r.querySelector('.sname')) === 'Work',
    )!;
    expect(text(work.querySelector('.nokey'))).toContain(
      "Its key (identity.age) isn't in the folder on this Mac",
    );
    expect(text(work.querySelector('.nokey'))).toContain('choose Keep Downloaded');
    // Nothing to back up to it or read from it: only a fresh look.
    expect(text(work.querySelector('.sacts'))).toBe('Check again');
    expect(text(work)).not.toContain('No backup yet');
    const before = backend.calls.filter((c) => c === 'GET /backup').length;
    (work.querySelector('.sacts .btn') as HTMLButtonElement).click();
    await settle();
    expect(backend.calls.filter((c) => c === 'GET /backup').length).toBe(before + 1);
  });

  it('asks to finish a passphrase change that did not finish', async () => {
    asDesktop();
    current = { ...ready(), passphraseChangeUnfinished: true };
    const el = await render();
    expect(text(el.querySelector('.unfinished'))).toContain(
      'A passphrase change did not finish, so some backups may open only with the new passphrase.',
    );
    button(el, 'Set the passphrase again')?.click();
    await settle();
    const dialog = el.querySelector('mc-passphrase-sheet dialog') as HTMLDialogElement;
    expect(dialog.open).toBeTrue();
    expect(text(dialog.querySelector('.title'))).toBe('Finish the passphrase change');
  });

  it('tells a phone to finish the passphrase change on the Mac', async () => {
    current = { ...ready(), passphraseChangeUnfinished: true };
    const el = await render();
    expect(text(el.querySelector('.unfinished'))).toContain(
      'Set the passphrase again on the Mac to finish it.',
    );
    expect(el.querySelector('.unfinished button')).toBeNull();
  });

  it('tells machines apart by id, and says when another Mac has this name', async () => {
    current = ready();
    current.sets[0] = {
      ...current.sets[0],
      machines: [
        { id: 'm-this', host: 'marcus-mbp', lastPush: Date.now() - 2 * H },
        { id: 'm-copy', host: 'marcus-mbp', lastPush: Date.now() - 3 * H },
        { id: 'm-air', host: 'marcus-air', lastPush: Date.now() - 5 * H },
      ],
    };
    const el = await render();
    const def = text(el.querySelector('.setrow'));
    expect(def).toContain('Also from marcus-air');
    expect(def).toContain('Also from another Mac named marcus-mbp');
  });

  it('lets the desktop pick a suggested folder in one click', async () => {
    asDesktop();
    backend.routes.set('/backup/settings', (_m, body) => {
      current = view({ settings: { ...current.settings, ...(body as object) } });
      return { status: 200, body: current };
    });
    const el = await render();
    expect(text(el.querySelector('.sets'))).toContain('Choose a folder first');
    expect(button(el, 'Back up now')?.disabled).toBeTrue();
    // The empty list says it; no second line about it.
    expect(el.querySelector('.why')).toBeNull();
    button(el, 'iCloud Drive')?.click();
    await settle();
    expect(backend.bodyOf('PUT /backup/settings')).toEqual({ folder: ICLOUD });
    expect(text(el.querySelector('.notice.ok'))).toBe('Backups go to this folder now.');
    expect((el.querySelector('input[name="folder"]') as HTMLInputElement).value).toBe(ICLOUD);
    // Still no passphrase.
    expect(text(el.querySelector('.why'))).toBe('Set a passphrase first.');
  });

  it('changes the schedule and Pull on launch from the desktop', async () => {
    asDesktop();
    current = ready();
    backend.routes.set('/backup/settings', (_m, body) => {
      current = { ...current, settings: { ...current.settings, ...(body as object) } };
      return { status: 200, body: current };
    });
    const el = await render();
    const sel = el.querySelector('#backup-every') as HTMLSelectElement;
    expect(sel.value).toBe('6');
    sel.value = '24';
    sel.dispatchEvent(new Event('change'));
    await settle();
    expect(backend.bodyOf('PUT /backup/settings')).toEqual({ everyHours: 24 });
    const pull = el.querySelector('.switch input') as HTMLInputElement;
    pull.checked = true;
    pull.dispatchEvent(new Event('change'));
    await settle();
    expect(backend.bodyOf('PUT /backup/settings')).toEqual({ pullOnLaunch: true });
  });

  it('backs every set up now and follows each job, which names its account', async () => {
    asDesktop();
    current = ready();
    const polls = new Map<string, number>();
    backend.routes.set('/backup/push', () => ({ status: 202, body: { jobs: ['jb1', 'jb2'] } }));
    for (const [id, account] of [
      ['jb1', 'default'],
      ['jb2', MOCK_ACCOUNT_WORK],
    ]) {
      backend.routes.set(`/data/jobs/${id}`, () => {
        const n = (polls.get(id) ?? 0) + 1;
        polls.set(id, n);
        return { status: 200, body: backupJob(id, n < 2 ? 'running' : 'done', account) };
      });
    }
    const el = await render();
    button(el, 'Back up now')?.click();
    await settle();
    expect(backend.bodyOf('POST /backup/push')).toEqual({});
    // One set per directory: the token account's is the Default's.
    expect(text(el.querySelector('.notice.ok'))).toBe('Backing up Default and Work…');
    await new Promise((r) => setTimeout(r, 30));
    await settle();
    const rows = [...el.querySelectorAll('mc-data-job')];
    expect(rows.map((r) => text(r.querySelector('.jtitle'))).sort()).toEqual([
      'Backup of Default',
      'Backup of Work',
    ]);
    expect(rows.every((r) => r.getAttribute('data-state') === 'done')).toBeTrue();
    expect(text(rows[0])).toContain('1 session backed up · 3 unchanged');
    // Once they ended the view is read again (last backups, machines).
    expect(backend.calls.filter((c) => c === 'GET /backup').length).toBeGreaterThan(1);
  });

  it('follows a set being backed up already once, and says so', async () => {
    asDesktop();
    current = ready();
    backend.routes.set('/backup/push', () => ({ status: 202, body: { jobs: ['jb1'] } }));
    backend.routes.set('/data/jobs/jb1', () => ({
      status: 200,
      body: backupJob('jb1', 'running'),
    }));
    const el = await render();
    (el.querySelector('[aria-label="Back up Default"]') as HTMLButtonElement).click();
    await settle();
    expect(text(el.querySelector('.notice.ok'))).toBe('Backing up Default…');
    (el.querySelector('[aria-label="Back up Default"]') as HTMLButtonElement).click();
    await settle();
    // The server answers the running job's id: the same row, followed once.
    expect(text(el.querySelector('.notice.ok'))).toBe('Default is being backed up already.');
    expect(el.querySelectorAll('mc-data-job').length).toBe(1);
  });

  it("says why nothing was backed up, in the app's words", async () => {
    asDesktop();
    current = ready();
    backend.routes.set('/backup/push', () => ({
      status: 409,
      body: { error: { code: 'no_backup_passphrase', message: 'no backup passphrase is set' } },
    }));
    const el = await render();
    button(el, 'Back up now')?.click();
    await settle();
    expect(text(el.querySelector('.notice.error'))).toBe('Set a passphrase first.');
  });

  it('unpacks the newest settings bundle and says where', async () => {
    asDesktop();
    current = ready();
    const folder = '/Users/me/Library/Application Support/x/restored-config/Default';
    backend.routes.set('/backup/restore', () => ({ status: 200, body: { folder } }));
    const el = await render();
    button(el, 'Restore settings')?.click();
    await settle();
    expect(backend.bodyOf('POST /backup/restore')).toEqual({ account: 'default', config: true });
    const cfg = text(el.querySelector('.cfg'));
    expect(cfg).toContain('your live settings are not changed');
    expect(cfg).toContain(folder);
  });
});
