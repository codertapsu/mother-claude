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
import { MOCK_ACCOUNT_WORK, buildAccounts } from '../../core/mock/fixtures';
import { setActiveMock } from '../../core/mock/mock-api';
import { BackupView, JobView } from '../../core/models';
import { BackupComponent } from './backup.component';

const H = 3_600_000;
const ICLOUD = '/Users/me/Library/Mobile Documents/com~apple~CloudDocs/Mother Claude';

const view = (over: Partial<BackupView> = {}): BackupView => ({
  settings: { folder: null, everyHours: 0, pullOnLaunch: false },
  passphraseSet: false,
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
        lastPush: Date.now() - 2 * H,
        sessions: 143,
        machines: [
          { host: 'marcus-mbp', lastPush: Date.now() - 2 * H },
          { host: 'marcus-air', lastPush: Date.now() - 5 * H },
        ],
      },
      {
        account: MOCK_ACCOUNT_WORK,
        name: 'Work',
        dir: `${ICLOUD}/Work`,
        exists: false,
        lastError: 'no backup passphrase is set',
        machines: [],
      },
    ],
  });

const backupJob = (id: string, state: JobView['state']): JobView => ({
  id,
  kind: 'backup',
  state,
  startedAt: Date.now(),
  filesDone: 0,
  filesTotal: 0,
  bytesDone: 0,
  bytesTotal: 0,
  sessions: [],
  ...(state === 'done'
    ? {
        result: {
          archived: ['a'],
          unchanged: 3,
          skippedLive: [],
          failed: [],
          config: null,
          bytes: 1,
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
    expect(text(el.querySelector('.sets'))).toContain('no backup passphrase is set');
    // Nothing that changes anything.
    expect(el.querySelectorAll('button').length).toBe(0);
    expect(el.querySelector('select')).toBeNull();
    expect(el.querySelector('input')).toBeNull();
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

  it('backs every account up now and follows each job', async () => {
    asDesktop();
    current = ready();
    const polls = new Map<string, number>();
    backend.routes.set('/backup/push', () => ({ status: 202, body: { jobs: ['jb1', 'jb2'] } }));
    for (const id of ['jb1', 'jb2']) {
      backend.routes.set(`/data/jobs/${id}`, () => {
        const n = (polls.get(id) ?? 0) + 1;
        polls.set(id, n);
        return { status: 200, body: backupJob(id, n < 2 ? 'running' : 'done') };
      });
    }
    const el = await render();
    button(el, 'Back up now')?.click();
    await settle();
    expect(backend.bodyOf('POST /backup/push')).toEqual({});
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
