import { ComponentFixture, TestBed } from '@angular/core/testing';
import { provideRouter } from '@angular/router';

import { DataJobsService } from '../../core/data-jobs.service';
import {
  FakeBackend,
  asDesktop,
  asPhoneOf,
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

  // ───────────────────────────── Lanes: drag and drop

  const lane = (el: HTMLElement, name: string) =>
    [...el.querySelectorAll<HTMLElement>('.lane')].find(
      (l) => text(l.querySelector('.sname')) === name,
    )!;
  /** Drag `from` onto `to`, as the browser dispatches it. */
  const dnd = async (from: Element, to: Element) => {
    const dataTransfer = new DataTransfer();
    const fire = (target: Element, type: string) =>
      target.dispatchEvent(new DragEvent(type, { bubbles: true, cancelable: true, dataTransfer }));
    fire(from, 'dragstart');
    fixture.detectChanges();
    fire(to, 'dragover');
    fixture.detectChanges();
    fire(to, 'drop');
    fire(from, 'dragend');
    await settle();
  };
  const indexOf = (sessions: unknown[]) => ({
    status: 200,
    body: {
      account: 'default',
      name: 'Default',
      set: 'Default',
      machines: [],
      sessions,
      config: [],
    },
  });
  const session = (id: string, action: string, extra: Record<string, unknown> = {}) => ({
    id,
    action,
    title: `Conversation ${id}`,
    cwd: '/Users/marcus/dev/app',
    remoteBytes: 1000,
    host: 'marcus-air',
    pushedAt: Date.now() - H,
    newestMtime: Date.now() - H,
    ...extra,
  });

  it('backs an account up when it is dropped on the backups, and brings a backup back where it is dropped', async () => {
    asDesktop();
    current = ready();
    backend.routes.set('/backup/push', () => ({ status: 202, body: { jobs: ['jp'] } }));
    backend.routes.set('/data/jobs/jp', () => ({ status: 200, body: backupJob('jp', 'done') }));
    backend.routes.set('/backup/index', () => indexOf([session('s1', 'new')]));
    const el = await render();
    expect(text(el.querySelector('.dnd'))).toContain('Drag an account onto the backups');
    const def = lane(el, 'Default');
    expect(text(def.querySelector('.link'))).toBe('Backed up 2h ago');
    expect(def.querySelector('.link')?.getAttribute('data-state')).toBe('ok');

    // The account onto its backup: backed up at once.
    await dnd(def.querySelector('.tile.here')!, def.querySelector('.tile.vault')!);
    expect(backend.bodyOf('POST /backup/push')).toEqual({ account: 'default' });
    expect(text(el.querySelector('.notice.ok'))).toBe('Backing up Default…');

    // The backup onto its own account: the Pull dialog for it.
    await dnd(def.querySelector('.tile.vault')!, def.querySelector('.tile.here')!);
    expect((el.querySelector('mc-pull-sheet dialog') as HTMLDialogElement).open).toBe(true);
    expect(backend.calls).not.toContain('POST /backup/pull');
    (el.querySelector('mc-pull-sheet button[aria-label="Close"]') as HTMLButtonElement).click();
    await settle();

    // Onto another account: Restore into that one.
    await dnd(def.querySelector('.tile.vault')!, lane(el, 'Work').querySelector('.tile.here')!);
    expect((el.querySelector('mc-restore-sheet dialog') as HTMLDialogElement).open).toBe(true);
    expect(backend.urls).toContain(`/backup/index?account=default&into=${MOCK_ACCOUNT_WORK}`);

    // A token account's lane takes nothing: its backup is the Default's.
    const before = backend.calls.length;
    await dnd(
      def.querySelector('.tile.vault')!,
      lane(el, 'Marcus (Pro)').querySelector('.tile.here')!,
    );
    expect(backend.calls.length).toBe(before);
    expect(lane(el, 'Marcus (Pro)').querySelector('.tile.here')?.getAttribute('draggable')).toBe(
      'false',
    );
  });

  it('brings conversations dragged from a backup to their account, and restores them into another', async () => {
    asDesktop();
    current = ready();
    const sessions = [
      session('s1', 'new'),
      session('s2', 'update'),
      session('s3', 'same'),
      session('s4', 'new', { pruneWarning: 'Claude Code will delete this again.' }),
    ];
    backend.routes.set('/backup/index', () => indexOf(sessions));
    backend.routes.set('/backup/pull', () => ({ status: 202, body: { jobs: ['jq'] } }));
    backend.routes.set('/data/jobs/jq', () => ({
      status: 200,
      body: { ...backupJob('jq', 'running'), kind: 'pull' },
    }));
    const el = await render();
    button(el, 'Conversations')!.click();
    await settle();
    const browser = el.querySelector('mc-backup-browser')!;
    const rows = () => [...browser.querySelectorAll<HTMLElement>('.row')];
    // What can come here, by default: not the one that is the same.
    expect(rows().map((r) => text(r.querySelector('.rtitle')))).toEqual([
      'Conversation s1',
      'Conversation s2',
      'Conversation s4',
    ]);
    expect(text(rows()[1].querySelector('.badge'))).toBe('Newer in the backup');

    // One onto its own account: brought at once (a pull of it).
    await dnd(rows()[0], lane(el, 'Default').querySelector('.tile.here')!);
    expect(backend.bodyOf('POST /backup/pull')).toEqual({ account: 'default', ids: ['s1'] });
    expect(text(el.querySelector('.notice.ok'))).toBe('Bringing 1 conversation to Default…');

    // Selected ones onto another account's chip: Restore into it, preset.
    (rows()[0].querySelector('input') as HTMLInputElement).click();
    (rows()[1].querySelector('input') as HTMLInputElement).click();
    fixture.detectChanges();
    const work = [...browser.querySelectorAll<HTMLElement>('.target')].find(
      (t) => text(t.querySelector('.tname')) === 'Work',
    )!;
    await dnd(rows()[1], work);
    const restore = el.querySelector('mc-restore-sheet dialog') as HTMLDialogElement;
    expect(restore.open).toBe(true);
    expect(backend.urls).toContain(`/backup/index?account=default&into=${MOCK_ACCOUNT_WORK}`);
    // Of those dropped, the one a restore takes starts selected.
    expect(text(restore.querySelector('.actions .btn'))).toBe('Restore 1 session');
    (restore.querySelector('button[aria-label="Close"]') as HTMLButtonElement).click();
    await settle();

    // One Claude Code would delete again: the Pull dialog says so first.
    const pulls = backend.calls.filter((c) => c === 'POST /backup/pull').length;
    await dnd(rows()[2], lane(el, 'Default').querySelector('.tile.here')!);
    const pull = el.querySelector('mc-pull-sheet dialog') as HTMLDialogElement;
    expect(pull.open).toBe(true);
    expect(backend.calls.filter((c) => c === 'POST /backup/pull').length).toBe(pulls);
    expect(text(pull)).toContain('Pull 1 session');
  });

  it('moves conversations into a storage set an account is dropped on, and back out onto an account', async () => {
    asDesktop();
    current = {
      ...ready(),
      storage: [
        {
          id: 'stor_5d2c8e1f0a7b',
          label: 'Google Drive',
          path: '/Users/me/Drive/Mother Claude',
          kind: 'gdrive',
          primary: false,
          available: true,
          sets: [{ name: 'Shared', exists: true, sessions: 3, archived: 1 }],
        },
      ],
    } as unknown as BackupView;
    const el = await render();
    const shared = [...el.querySelectorAll<HTMLElement>('.stset')].find(
      (r) => text(r.querySelector('.stsname')) === 'Shared',
    )!;
    const selects = () =>
      [...el.querySelectorAll<HTMLSelectElement>('mc-move-data-sheet select')].map((x) => x.value);

    // An account onto the set: Move data from it into the set.
    await dnd(lane(el, 'Default').querySelector('.tile.here')!, shared);
    expect((el.querySelector('mc-move-data-sheet dialog') as HTMLDialogElement).open).toBe(true);
    expect(selects().slice(0, 2)).toEqual(['a:default', 's:stor_5d2c8e1f0a7b:Shared']);
    (
      el.querySelector('mc-move-data-sheet button[aria-label="Close"]') as HTMLButtonElement
    ).click();
    await settle();

    // The set onto an account: Move data out of it into that account.
    await dnd(shared, lane(el, 'Work').querySelector('.tile.here')!);
    expect((el.querySelector('mc-move-data-sheet dialog') as HTMLDialogElement).open).toBe(true);
    expect(selects().slice(0, 2)).toEqual(['s:stor_5d2c8e1f0a7b:Shared', `a:${MOCK_ACCOUNT_WORK}`]);
  });

  it('lets a phone run and set up the backups, all but the passphrase', async () => {
    current = ready();
    const el = await render();
    expect(el.querySelector('.phone-note')).toBeNull();
    expect(text(el)).toContain("Set, in this Mac's Keychain");
    // The passphrase stays on the Mac (docs/ACCOUNTS.md §13.9).
    expect(text(el.querySelector('.onmac'))).toBe('It is set and changed on the Mac.');
    expect(button(el, 'Change…')).toBeUndefined();
    expect(el.querySelector('mc-passphrase-sheet')).toBeNull();
    // Everything else is the phone's too.
    expect(el.querySelector('input[name="folder"]')).not.toBeNull();
    expect((el.querySelector('#backup-every') as HTMLSelectElement).value).toBe('6');
    expect(el.querySelector('.switch input')).not.toBeNull();
    expect(button(el, 'Back up now')?.disabled).toBe(false);
    expect(button(el, 'Pull…')?.disabled).toBe(false);
    const labels = [...el.querySelectorAll('.sacts .btn')].map((b) => b.getAttribute('aria-label'));
    expect(labels).toContain('Back up Default');
    expect(labels).toContain('Restore from the backup of Default');
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
  });

  it('backs up from a phone', async () => {
    current = ready();
    backend.routes.set('/backup/push', () => ({ status: 202, body: { jobs: ['job_b'] } }));
    backend.routes.set('/data/jobs/job_b', () => ({
      status: 200,
      body: backupJob('job_b', 'done'),
    }));
    const el = await render();
    button(el, 'Back up now')?.click();
    await settle();
    expect(backend.calls).toContain('POST /backup/push');
    expect(text(el.querySelector('.notice.ok'))).toContain('Backing up Default and Work');
  });

  it("gives a token account's row no buttons: its set is the Default's", async () => {
    asDesktop();
    current = ready();
    const el = await render();
    const labels = [...el.querySelectorAll('.sacts .btn')].map((b) => b.getAttribute('aria-label'));
    expect(labels).toContain('Back up Default');
    expect(labels).toContain('Back up Work');
    expect(labels.some((l) => (l ?? '').includes('Marcus (Pro)'))).toBe(false);
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
      "Its key (identity.age) isn't in the folder here",
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
    expect(dialog.open).toBe(true);
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
    expect(def).toContain('Also from another computer named marcus-mbp');
  });

  it('lets the desktop pick a suggested folder in one click', async () => {
    asDesktop();
    backend.routes.set('/backup/settings', (_m, body) => {
      current = view({ settings: { ...current.settings, ...(body as object) } });
      return { status: 200, body: current };
    });
    const el = await render();
    expect(text(el.querySelector('.sets'))).toContain('Choose a folder first');
    expect(button(el, 'Back up now')?.disabled).toBe(true);
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

  it("lets a phone set a Windows desktop's folder the way Windows writes it", async () => {
    asPhoneOf('windows');
    backend.routes.set('/backup/settings', (_m, body) => {
      current = view({ settings: { ...current.settings, ...(body as object) } });
      return { status: 200, body: current };
    });
    const el = await render();
    const input = el.querySelector('input[name="folder"]') as HTMLInputElement;
    expect(input.placeholder).toBe('D:\\Backups\\Mother Claude');
    expect(text(el.querySelector('.hint'))).toContain('On OneDrive');
    const type = async (value: string) => {
      input.value = value;
      input.dispatchEvent(new Event('input'));
      fixture.detectChanges();
      button(el, 'Save')?.click();
      await settle();
    };
    // Not a full path on either system: said in the desktop's terms.
    await type('NAS\\MotherClaude');
    expect(backend.calls).not.toContain('PUT /backup/settings');
    expect(text(el.querySelector('.notice.error'))).toBe(
      "Enter the folder's full path, starting with a drive like C:\\ or a share like \\\\server\\share (or ~ for your home folder).",
    );
    await type('D:\\NAS\\MotherClaude');
    expect(backend.bodyOf('PUT /backup/settings')).toEqual({ folder: 'D:\\NAS\\MotherClaude' });
    expect(text(el.querySelector('.notice.ok'))).toBe('Backups go to this folder now.');
  });

  it("shows the desktop's own refusal of a path its system can't take", async () => {
    asDesktop({ hostOs: 'macos' });
    backend.routes.set('/backup/settings', () => ({
      status: 422,
      body: {
        error: {
          code: 'invalid_folder',
          message:
            'D:\\NAS is not a full path on this computer: start it with / (or ~ for your home folder)',
        },
      },
    }));
    const el = await render();
    const input = el.querySelector('input[name="folder"]') as HTMLInputElement;
    expect(input.placeholder).toBe(
      '/Users/you/Library/Mobile Documents/com~apple~CloudDocs/Mother Claude',
    );
    expect(text(el.querySelector('.hint'))).toContain('Keep Downloaded');
    input.value = 'D:\\NAS';
    input.dispatchEvent(new Event('input'));
    fixture.detectChanges();
    button(el, 'Save')?.click();
    await settle();
    // A full path on Windows: sent, and the Mac says no.
    expect(backend.bodyOf('PUT /backup/settings')).toEqual({ folder: 'D:\\NAS' });
    expect(text(el.querySelector('.notice.error'))).toBe(
      'Not saved: D:\\NAS is not a full path on this computer: start it with / (or ~ for your home folder).',
    );
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
    expect(rows.every((r) => r.getAttribute('data-state') === 'done')).toBe(true);
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

  it('calls a Windows desktop a PC, and its passphrase kept on it', async () => {
    asDesktop({ hostOs: 'windows' });
    current = ready();
    const el = await render();
    expect(text(el)).toContain('Set, on this PC');
    expect(text(el)).toContain('It is kept on the PC.');
    expect(text(el)).not.toContain('Keychain');
    expect(text(el.querySelector('.sets'))).toContain('Backed up from this PC');
    (el.querySelector('[aria-label="Force backup of Default"]') as HTMLButtonElement).click();
    await settle();
    const what = text(el.querySelector('mc-force-backup-sheet .what'));
    expect(what).toContain("This PC's copy of every session it has replaces the backup's copy");
    expect(what).toContain("its next pull puts this PC's beside it");
    expect(what).not.toContain('Mac');
  });

  it('force-backs up a set after a confirmation that says what is replaced and what stays', async () => {
    asDesktop();
    current = ready();
    backend.routes.set('/backup/push', () => ({ status: 202, body: { jobs: ['jf1'] } }));
    backend.routes.set('/data/jobs/jf1', () => ({
      status: 200,
      body: {
        ...backupJob('jf1', 'done'),
        result: {
          archived: ['c4a2e3f5', 'd5b3f4a6'],
          unchanged: 3,
          skippedLive: [],
          failed: [],
          conflicts: [],
          newerInSet: [],
          config: null,
          bytes: 1,
          warnings: [],
          removed: 0,
          force: true,
          replaced: ['c4a2e3f5', 'd5b3f4a6'],
        },
      },
    }));
    const el = await render();
    // Only a set that exists has it (a token account's row has none).
    const labels = [...el.querySelectorAll('.sacts .btn')].map((b) => b.getAttribute('aria-label'));
    expect(labels).toContain('Force backup of Default');
    expect(labels).not.toContain('Force backup of Work');
    (el.querySelector('[aria-label="Force backup of Default"]') as HTMLButtonElement).click();
    await settle();
    const dialog = el.querySelector('mc-force-backup-sheet dialog') as HTMLDialogElement;
    expect(dialog.open).toBe(true);
    expect(text(dialog.querySelector('.title'))).toBe('Force backup of Default');
    const what = text(dialog.querySelector('.what'));
    expect(what).toContain(
      "This Mac's copy of every session it has replaces the backup's copy of Default wherever the two differ",
    );
    expect(what).toContain('also where another computer backed up a newer copy');
    expect(what).toContain('Sessions only in the backup stay as they are');
    expect(what).toContain('sessions running here are skipped');
    expect(what).toContain('stays in the backup folder for seven days');
    // Nothing is sent before it is confirmed.
    expect(backend.calls).not.toContain('POST /backup/push');
    button(dialog as unknown as HTMLElement, 'Force backup')?.click();
    await settle();
    expect(backend.bodyOf('POST /backup/push')).toEqual({ account: 'default', force: true });
    expect(text(el.querySelector('.notice.ok'))).toBe(
      "Force backup of Default: this Mac's copies replace the backup's…",
    );
    await new Promise((r) => setTimeout(r, 30));
    await settle();
    const row = el.querySelector('mc-data-job');
    expect(text(row?.querySelector('.jtitle'))).toBe('Force backup of Default');
    expect(text(row)).toContain("2 sessions backed up · 2 replaced the backup's copy");
    expect(text(row?.querySelector('mc-job-details p'))).toContain(
      'the folder keeps the replaced copies seven days',
    );
  });

  it('lets a phone see why it cannot force a backup', async () => {
    current = ready();
    const el = await render();
    (el.querySelector('[aria-label="Force backup of Default"]') as HTMLButtonElement).click();
    await settle();
    const dialog = el.querySelector('mc-force-backup-sheet dialog') as HTMLElement;
    expect(text(dialog.querySelector('.blocked'))).toContain('dangerous');
    expect(button(dialog, 'Force backup')?.disabled).toBe(true);
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
