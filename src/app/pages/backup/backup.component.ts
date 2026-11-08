import {
  Component,
  DestroyRef,
  computed,
  inject,
  linkedSignal,
  signal,
  viewChild,
} from '@angular/core';
import { FormsModule } from '@angular/forms';
import { RouterLink } from '@angular/router';

import { AccountsService } from '../../core/accounts.service';
import { ApiService } from '../../core/api.service';
import { AuthService } from '../../core/auth.service';
import {
  BACKUP_SCHEDULES,
  KEY_MISSING_TEXT,
  andList,
  backupErrorText,
  normalizeBackupView,
  normalizePullSkips,
  otherMachines,
  ownSets,
  scheduleLabel,
} from '../../core/backup';
import { jobIdOf, normalizeJob, plural, sentence } from '../../core/data-jobs';
import { DataJobsService } from '../../core/data-jobs.service';
import { BackupMachine, BackupSet, BackupView, JobView } from '../../core/models';
import { folderExample, folderPathProblem, lastSegment, machineNoun } from '../../core/paths';
import { RealtimeService } from '../../core/realtime.service';
import { StorageService } from '../../core/storage.service';
import { CopyButtonComponent } from '../../shared/copy-button.component';
import { DataJobComponent } from '../../shared/data-job.component';
import { IconComponent } from '../../shared/icon.component';
import { TimeAgoComponent } from '../../shared/time-ago.component';
import { relativeTime, shortPath } from '../../shared/util';
import { BackupBrowserComponent, BrowserBring, BrowserTarget } from './backup-browser.component';
import { FolderPairsComponent } from './folder-pairs.component';
import { ForceBackupSheetComponent } from './force-backup-sheet.component';
import { JobDetailsComponent } from './job-details.component';
import { PassphraseSheetComponent } from './passphrase-sheet.component';
import { PullSheetComponent } from './pull-sheet.component';
import { RestoreSheetComponent } from './restore-sheet.component';
import { StorageSectionComponent } from './storage-section.component';
import { SyncDrag, SyncDragItem, SyncDropTarget, dropText, targetKey } from './sync-drag';

/** The newest jobs listed. */
const JOBS_SHOWN = 8;

interface SetRow {
  set: BackupSet;
  /** The other machines that pushed to it. */
  others: BackupMachine[];
  /** Another Mac with this Mac's name pushed to it too. */
  sameName: boolean;
  /** A token account's row: the name of the account whose set has its
   * sessions ('' for a set of its own). */
  sharedWith: string;
  /** Its last backup's trouble, as a sentence ('' for none). */
  lastError: string;
}

/**
 * Settings › Backup & sync (docs/BACKUP.md, docs/ACCOUNTS.md §6.6): an
 * encrypted backup of each Claude account's data in a folder of your
 * choosing — the same folder carries conversations to another laptop. The
 * folder (with iCloud Drive and Google Drive suggested when the Mac has
 * them), the passphrase (generated and shown once, or your own; kept in the
 * Keychain), the schedule, Back up now, each account's last backup and the
 * other machines that pushed to it, Restore… and Pull… with their session
 * lists, Restore settings, Pull on launch, and the recent jobs with Undo.
 * One set per directory: a token account's sessions are in the Default's.
 * A passphrase change that did not finish, and a set whose key is not in
 * the folder here, are said up front. The Storage section lists the backup
 * folder and the folders added beside it, with their sets (Move data).
 * The desktop and paired phones alike (docs/ACCOUNTS.md §13.9), except the
 * passphrase, which is set on the Mac; a folder's trust needs the
 * dangerous right. Each set has Force backup… (this Mac's copies replace
 * the backup's, after a confirmation that says what is replaced and what
 * stays; the dangerous right too); force pull and force restore are in
 * their dialogs. Folder pairs map a project folder here to another
 * machine's ({@link FolderPairsComponent}).
 */
@Component({
  selector: 'mc-backup',
  imports: [
    FormsModule,
    RouterLink,
    BackupBrowserComponent,
    CopyButtonComponent,
    DataJobComponent,
    FolderPairsComponent,
    ForceBackupSheetComponent,
    IconComponent,
    JobDetailsComponent,
    PassphraseSheetComponent,
    PullSheetComponent,
    RestoreSheetComponent,
    StorageSectionComponent,
    TimeAgoComponent,
  ],
  providers: [SyncDrag],
  templateUrl: './backup.component.html',
  styleUrl: './backup.component.scss',
})
export class BackupComponent {
  protected readonly drag = inject(SyncDrag);
  /** The backups column as a drop target. */
  protected readonly backups: SyncDropTarget = { kind: 'backups' };
  private readonly storageSection = viewChild(StorageSectionComponent);
  private api = inject(ApiService);
  private jobs = inject(DataJobsService);
  private realtime = inject(RealtimeService);
  private storage = inject(StorageService);
  protected readonly accounts = inject(AccountsService);
  protected readonly isDesktop = inject(AuthService).isDesktop;
  /** The desktop's system: the folder is typed its way. */
  protected readonly hostOs = inject(AuthService).hostOs;
  protected readonly folderPlaceholder = computed(() => folderExample(this.hostOs(), 'backup'));
  /** The desktop's computer: "this Mac", "this PC". */
  protected readonly mac = computed(() => machineNoun(this.hostOs()));
  /** Where the passphrase is kept: a Mac's Keychain, elsewhere on the
   * computer itself (docs/ACCOUNTS.md D3). */
  protected keptIn(article: 'this' | 'the'): string {
    return this.hostOs() === 'macos'
      ? `in ${article} Mac's Keychain`
      : `on ${article} ${this.mac()}`;
  }

  protected readonly view = signal<BackupView | null>(null);
  protected readonly loadError = signal('');
  protected readonly error = signal('');
  protected readonly notice = signal('');
  protected readonly saving = signal(false);
  /** `<account id>` or `all` while a push is being started. */
  protected readonly pushing = signal<string | null>(null);
  /** The folder field: the saved folder until edited. */
  protected readonly folder = linkedSignal(() => this.view()?.settings.folder ?? '');
  protected readonly passOpen = signal(false);
  protected readonly restoreFor = signal<string | null>(null);
  /** The Restore dialog's account to restore into and sessions to start
   * selected, when a drop chose them. */
  protected readonly restoreInto = signal<string | null>(null);
  protected readonly restoreIds = signal<string[] | null>(null);
  /** The Pull dialog's sessions to start selected (dropped ones). */
  protected readonly pullIds = signal<string[] | null>(null);
  /** The account whose backup's conversations are browsed. */
  protected readonly browsing = signal<string | null>(null);
  /** The pull dialog: one account (its id), every account (''), or closed. */
  protected readonly pullFor = signal<string | null>(null);
  /** The force backup confirmation: the set's account, or closed. */
  protected readonly forceFor = signal<string | null>(null);
  protected readonly forceSet = computed(
    () => this.view()?.sets.find((s) => s.account === this.forceFor()) ?? null,
  );
  /** "Restore settings": where each account's bundle was unpacked. */
  protected readonly configFolders = signal<ReadonlyMap<string, string>>(new Map());
  protected readonly configBusy = signal<string | null>(null);
  /** Jobs followed from here (newer than the view's copy; each names its
   * accounts, so its row titles itself). */
  private readonly live = signal<ReadonlyMap<string, JobView>>(new Map());
  private readonly following = new Set<string>();
  private readonly abort = new AbortController();

  protected readonly schedules = BACKUP_SCHEDULES;
  protected readonly scheduleLabel = scheduleLabel;
  protected readonly plural = plural;
  protected readonly keyMissingText = KEY_MISSING_TEXT;

  protected readonly settings = computed(() => this.view()?.settings ?? null);
  protected readonly ready = computed(() => {
    const v = this.view();
    return !!v?.settings.folder && v.passphraseSet;
  });
  /** Why nothing can be backed up yet ('' when it can). */
  protected readonly notReady = computed(() => {
    const v = this.view();
    if (!v) return '';
    if (!v.settings.folder) return 'Choose a folder first.';
    if (!v.passphraseSet) return 'Set a passphrase first.';
    return '';
  });
  /** The first backup is made: the setup guide goes. */
  protected readonly setupDone = computed(() =>
    (this.view()?.sets ?? []).some((s) => !!s.lastPush || s.exists),
  );
  /** The setup step to do now (1 folder, 2 passphrase, 3 first backup). */
  protected readonly setupStep = computed(() => {
    const v = this.view();
    if (!v?.settings.folder) return 1;
    if (!v.passphraseSet) return 2;
    return 3;
  });
  protected readonly knownSchedule = computed(() =>
    BACKUP_SCHEDULES.some((s) => s.hours === this.settings()?.everyHours),
  );
  protected readonly folderChanged = computed(
    () => this.folder().trim() !== (this.view()?.settings.folder ?? ''),
  );
  protected readonly rows = computed<SetRow[]>(() => {
    const v = this.view();
    if (!v) return [];
    return v.sets.map((set) => ({
      set,
      ...otherMachines(set.machines, v.host, v.machine),
      sharedWith: set.sharedWith ? this.nameOf(set.sharedWith) || 'Default' : '',
      lastError: sentence(set.lastError),
    }));
  });
  /** Some set of its own is there to restore or pull from. */
  protected readonly anySet = computed(() =>
    ownSets(this.view()?.sets ?? []).some((s) => s.exists),
  );
  protected readonly jobList = computed<JobView[]>(() => {
    const merged = new Map((this.view()?.jobs ?? []).map((j) => [j.id, j]));
    for (const [id, j] of this.live()) merged.set(id, j);
    return [...merged.values()].sort((a, b) => b.startedAt - a.startedAt).slice(0, JOBS_SHOWN);
  });
  protected readonly sets = computed(() => this.view()?.sets ?? []);
  protected readonly browsedSet = computed(
    () => this.sets().find((s) => s.account === this.browsing() && s.exists) ?? null,
  );
  /** The accounts a backup's conversations can be brought into: those with
   * a backup of their own (a token account's is the Default's). */
  protected readonly browseTargets = computed<BrowserTarget[]>(() =>
    this.rows()
      .filter((r) => !r.sharedWith)
      .map((r) => ({
        account: r.set.account,
        name: r.set.name,
        color: this.colorOf(r.set.account),
      })),
  );
  /** The backup folder, as people know it (its suggestion's name, else its
   * last folder). */
  protected readonly folderName = computed(() => {
    const v = this.view();
    const folder = v?.settings.folder ?? '';
    return v?.suggestions.find((s) => s.path === folder)?.label || lastSegment(folder) || folder;
  });

  constructor() {
    void this.accounts.ensureLoaded();
    void this.load();
    const sub = this.realtime.refetch$.subscribe(() => void this.load());
    inject(DestroyRef).onDestroy(() => {
      sub.unsubscribe();
      this.abort.abort();
    });
  }

  protected short(p: string | null | undefined): string {
    return shortPath(p ?? '');
  }

  async load(): Promise<void> {
    try {
      const raw = await this.api.getBackup();
      const v = normalizeBackupView(raw);
      if (!v) throw new Error('The Mac sent backup settings this app could not read.');
      this.view.set(v);
      // The storages are in the same answer.
      this.storage.accept(raw);
      this.loadError.set('');
      // Jobs that still run (started here or elsewhere) are followed.
      for (const j of v.jobs) if (j.state === 'running') this.follow(j);
    } catch (e) {
      this.loadError.set(backupErrorText(e));
    }
  }

  // ─────────────────────────────────────────────── Settings (desktop)

  protected async saveFolder(e: Event): Promise<void> {
    e.preventDefault();
    const f = this.folder().trim();
    // Either system's full path (or ~): the desktop has the last word.
    const problem = f ? folderPathProblem(f, this.hostOs()) : '';
    if (problem) {
      this.error.set(problem);
      return;
    }
    await this.saveSettings(
      { folder: f || null },
      f ? 'Backups go to this folder now.' : 'Backups are off: no folder is set.',
    );
  }

  protected async useSuggestion(path: string): Promise<void> {
    this.folder.set(path);
    await this.saveSettings({ folder: path }, 'Backups go to this folder now.');
  }

  protected async setSchedule(e: Event): Promise<void> {
    const sel = e.target as HTMLSelectElement;
    const hours = Number(sel.value);
    const before = this.settings()?.everyHours ?? 0;
    if (!Number.isFinite(hours) || hours === before) return;
    if (!(await this.saveSettings({ everyHours: hours }))) sel.value = String(before);
  }

  protected async setPullOnLaunch(on: boolean): Promise<void> {
    await this.saveSettings({ pullOnLaunch: on });
  }

  private async saveSettings(
    patch: { folder?: string | null; everyHours?: number; pullOnLaunch?: boolean },
    ok = '',
  ): Promise<boolean> {
    this.saving.set(true);
    this.error.set('');
    this.notice.set('');
    try {
      const raw = await this.api.setBackupSettings(patch);
      const v = normalizeBackupView(raw);
      if (v) {
        this.view.set(v);
        this.storage.accept(raw);
      } else await this.load();
      if (ok) this.notice.set(ok);
      return true;
    } catch (e) {
      this.error.set(`Not saved: ${backupErrorText(e)}`);
      return false;
    } finally {
      this.saving.set(false);
    }
  }

  // ─────────────────────────────────────────────── Backing up (desktop)

  /** Back up one set, or every set (`account` omitted). A set that is
   * being backed up already answers that job, which is followed once.
   * `force`: a force backup (confirmed in its dialog first). */
  protected async push(account?: string, force = false): Promise<void> {
    if (this.pushing()) return;
    this.pushing.set(account ?? 'all');
    this.error.set('');
    this.notice.set('');
    try {
      const res = await this.api.pushBackup(account, force);
      const ids = [...new Set((res?.jobs ?? []).map(jobIdOf).filter(Boolean))];
      const already = ids.filter((id) => this.following.has(id)).length;
      for (const id of ids) this.follow(id);
      const names = account
        ? [this.nameOf(account)]
        : ownSets(this.view()?.sets ?? []).map((s) => s.name);
      const who = andList(names.filter(Boolean)) || 'the accounts';
      if (!ids.length) this.notice.set('Nothing to back up.');
      else if (already === ids.length) {
        this.notice.set(
          account ? `${who} is being backed up already.` : 'Every backup is running already.',
        );
      } else if (force) {
        this.notice.set(
          `Force backup of ${who}: this ${this.mac()}'s copies replace the backup's…`,
        );
      } else this.notice.set(`Backing up ${who}…`);
      if (force) this.forceFor.set(null);
    } catch (e) {
      this.error.set(backupErrorText(e));
      if (force) this.forceFor.set(null);
    } finally {
      this.pushing.set(null);
    }
  }

  /** "Restore settings": the newest bundle unpacked for review. */
  protected async restoreSettings(s: BackupSet): Promise<void> {
    if (this.configBusy()) return;
    this.configBusy.set(s.account);
    this.error.set('');
    try {
      const res = await this.api.restoreBackupSettings(s.account);
      const folder = typeof res?.folder === 'string' ? res.folder : '';
      if (!folder) throw new Error('The Mac did not say where it unpacked them.');
      this.configFolders.update((prev) => new Map(prev).set(s.account, folder));
    } catch (e) {
      this.error.set(`${s.name}: ${backupErrorText(e)}`);
    } finally {
      this.configBusy.set(null);
    }
  }

  /** "Type a folder…": the folder field in Settings, focused. */
  protected focusFolder(e: Event): void {
    e.preventDefault();
    const input = document.querySelector<HTMLInputElement>('mc-backup input[name="folder"]');
    input?.scrollIntoView({ behavior: 'smooth', block: 'center' });
    input?.focus({ preventScroll: true });
  }

  // ─────────────────────────────────────────────── Lanes: drag and drop

  protected accountTarget(r: SetRow): SyncDropTarget {
    return { kind: 'account', account: r.set.account };
  }

  protected colorOf(account: string): string {
    return this.accounts.byId(account)?.color ?? 'var(--accent)';
  }

  /** What an account here is, under its name. */
  protected kindText(r: SetRow): string {
    const a = this.accounts.byId(r.set.account);
    const kind = r.sharedWith
      ? 'Token account'
      : !a || a.isDefault
        ? 'Default account'
        : a.kind === 'directory'
          ? 'Its own directory'
          : 'Account';
    return a?.live ? `${kind} · ${plural(a.live, 'running session')}` : kind;
  }

  /** How the lane between an account and its backup stands. */
  protected laneState(r: SetRow): string {
    if (r.sharedWith) return 'shared';
    if (this.jobRunsFor(r.set.account)) return 'busy';
    if (r.set.keyMissing || r.lastError) return 'problem';
    if (!r.set.lastPush) return 'never';
    const every = this.settings()?.everyHours ?? 0;
    const stale = every > 0 && Date.now() - r.set.lastPush > every * 2 * 3_600_000;
    return stale ? 'stale' : 'ok';
  }

  protected laneText(r: SetRow): string {
    switch (this.laneState(r)) {
      case 'shared':
        return `In ${r.sharedWith}'s backup`;
      case 'busy':
        return 'Working…';
      case 'problem':
        return r.set.keyMissing ? 'Key missing here' : 'Last backup failed';
      case 'never':
        return this.ready() ? 'Not backed up yet' : 'Not set up';
      default:
        return `Backed up ${relativeTime(r.set.lastPush)}`;
    }
  }

  protected canDragAccount(r: SetRow): boolean {
    return this.ready() && !r.sharedWith && !this.pushing();
  }

  protected canDragSet(r: SetRow): boolean {
    return this.ready() && !r.sharedWith && r.set.exists && !r.set.keyMissing;
  }

  protected dragAccount(e: DragEvent, r: SetRow): void {
    if (!this.canDragAccount(r)) {
      e.preventDefault();
      return;
    }
    this.drag.start(e, { kind: 'account', account: r.set.account, name: r.set.name }, r.set.name);
  }

  protected dragSet(e: DragEvent, r: SetRow): void {
    if (!this.canDragSet(r)) {
      e.preventDefault();
      return;
    }
    this.drag.start(
      e,
      { kind: 'set', account: r.set.account, name: r.set.name },
      `${r.set.name}'s backup`,
    );
  }

  /** What dropping the dragged item on `t` does ('' when it can't). */
  protected dropLabel(t: SyncDropTarget): string {
    const item = this.drag.item();
    if (!item || !this.ready()) return '';
    if (t.kind === 'account') {
      const row = this.rows().find((r) => r.set.account === t.account);
      if (!row || row.sharedWith) return '';
    }
    return dropText(item, t, (a) => this.nameOf(a));
  }

  protected over(e: DragEvent, t: SyncDropTarget): void {
    if (!this.dropLabel(t)) return;
    e.preventDefault();
    if (e.dataTransfer) e.dataTransfer.dropEffect = 'copy';
    this.drag.over.set(targetKey(t));
  }

  protected leave(t: SyncDropTarget): void {
    if (this.drag.over() === targetKey(t)) this.drag.over.set('');
  }

  protected drop(e: DragEvent, t: SyncDropTarget): void {
    const label = this.dropLabel(t);
    const item = this.drag.item();
    this.drag.end();
    if (!label || !item) return;
    e.preventDefault();
    this.perform(item, t);
  }

  /** A drop, done: backing up starts at once; bringing a backup back opens
   * its dialog (Pull for its own account, Restore for another), which says
   * what comes; dropped conversations come at once onto their own account
   * (a pull of them, undoable) unless Claude Code would delete some again. */
  private perform(item: SyncDragItem, t: SyncDropTarget): void {
    if (t.kind === 'backups') {
      if (item.kind === 'account') void this.push(item.account);
      return;
    }
    if (t.kind === 'storage') return;
    if (item.kind === 'stored') {
      this.storageSection()?.bringBackInto(item.store, item.set, t.account);
    } else if (item.kind === 'set') {
      if (item.account === t.account) this.openPull(item.account);
      else this.openRestore(item.account, t.account);
    } else if (item.kind === 'sessions') {
      if (item.set === t.account)
        void this.bring({ set: item.set, ids: item.ids, prune: item.prune });
      else this.openRestore(item.set, t.account, item.ids);
    }
  }

  protected toggleBrowse(account: string): void {
    this.browsing.update((cur) => (cur === account ? null : account));
  }

  /** Open the Restore dialog for `set`'s backup, into `into` with `ids`
   * selected when given. */
  protected openRestore(
    set: string,
    into: string | null = null,
    ids: string[] | null = null,
  ): void {
    this.restoreInto.set(into);
    this.restoreIds.set(ids);
    this.restoreFor.set(set);
  }

  protected closeRestore(): void {
    this.restoreFor.set(null);
    this.restoreInto.set(null);
    this.restoreIds.set(null);
  }

  /** Bring conversations of an account's backup to that account (a pull of
   * them: new ones come, newer ones replace the copy here — kept for undo —
   * and both sides of a conflict are kept). Some Claude Code would delete
   * again: the Pull dialog first, which says so. */
  protected async bring(b: BrowserBring): Promise<void> {
    if (!b.ids.length) return;
    if (b.prune) {
      this.openPull(b.set, b.ids);
      return;
    }
    const name = this.nameOf(b.set) || 'the account';
    this.error.set('');
    this.notice.set('');
    try {
      const res = await this.api.pullBackup({ account: b.set, ids: b.ids });
      const skipped = normalizePullSkips(res?.skipped).map((m) => sentence(m.message));
      const raw = (res?.jobs ?? [])[0];
      const first = normalizeJob(raw) ?? jobIdOf(raw);
      if (first) this.follow(first);
      this.notice.set(
        first
          ? `Bringing ${plural(b.ids.length, 'conversation')} to ${name}…`
          : skipped.join(' ') || 'Nothing to bring.',
      );
    } catch (e) {
      this.error.set(`${name}: ${backupErrorText(e)}`);
    }
  }

  /** A data job runs for this account (backup, pull, restore). */
  private jobRunsFor(account: string): boolean {
    return this.jobList().some(
      (j) =>
        j.state === 'running' && (j.account === account || j.from === account || j.to === account),
    );
  }

  /** A restore or pull started in its dialog. */
  protected tracked(id: string): void {
    this.follow(id);
  }

  /** Open the passphrase dialog (also to finish an unfinished change). */
  protected openPassphrase(): void {
    this.passOpen.set(true);
  }

  protected jobChanged(j: JobView): void {
    this.live.update((prev) => new Map(prev).set(j.id, j));
  }

  protected openPull(account: string | null, ids: string[] | null = null): void {
    this.pullIds.set(ids);
    this.pullFor.set(account ?? '');
  }

  protected closePull(): void {
    this.pullFor.set(null);
    this.pullIds.set(null);
  }

  private nameOf(account: string): string {
    return (
      this.view()?.sets.find((s) => s.account === account)?.name ??
      this.accounts.byId(account)?.name ??
      ''
    );
  }

  /** Follow a job until it ends, then read the view again (last backups,
   * machines). */
  private follow(first: JobView | string): void {
    const id = typeof first === 'string' ? first : first.id;
    if (!id || this.following.has(id)) return;
    this.following.add(id);
    void this.jobs
      .follow(first, (j) => this.jobChanged(j), this.abort.signal)
      .catch(() => undefined)
      .finally(() => {
        this.following.delete(id);
        if (!this.abort.signal.aborted && this.following.size === 0) void this.load();
      });
  }
}
