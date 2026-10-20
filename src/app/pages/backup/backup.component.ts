import { Component, DestroyRef, computed, inject, linkedSignal, signal } from '@angular/core';
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
  otherMachines,
  ownSets,
  scheduleLabel,
} from '../../core/backup';
import { jobIdOf, plural, sentence } from '../../core/data-jobs';
import { DataJobsService } from '../../core/data-jobs.service';
import { BackupMachine, BackupSet, BackupView, JobView } from '../../core/models';
import { RealtimeService } from '../../core/realtime.service';
import { CopyButtonComponent } from '../../shared/copy-button.component';
import { DataJobComponent } from '../../shared/data-job.component';
import { IconComponent } from '../../shared/icon.component';
import { TimeAgoComponent } from '../../shared/time-ago.component';
import { shortPath } from '../../shared/util';
import { JobDetailsComponent } from './job-details.component';
import { PassphraseSheetComponent } from './passphrase-sheet.component';
import { PullSheetComponent } from './pull-sheet.component';
import { RestoreSheetComponent } from './restore-sheet.component';

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
 * the folder here, are said up front. Everything that changes something is
 * the desktop's; phones see it all.
 */
@Component({
  selector: 'mc-backup',
  imports: [
    FormsModule,
    RouterLink,
    CopyButtonComponent,
    DataJobComponent,
    IconComponent,
    JobDetailsComponent,
    PassphraseSheetComponent,
    PullSheetComponent,
    RestoreSheetComponent,
    TimeAgoComponent,
  ],
  templateUrl: './backup.component.html',
  styleUrl: './backup.component.scss',
})
export class BackupComponent {
  private api = inject(ApiService);
  private jobs = inject(DataJobsService);
  private realtime = inject(RealtimeService);
  protected readonly accounts = inject(AccountsService);
  protected readonly isDesktop = inject(AuthService).isDesktop;

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
  /** The pull dialog: one account (its id), every account (''), or closed. */
  protected readonly pullFor = signal<string | null>(null);
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
      ...otherMachines(set.machines, v.host),
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
      const v = normalizeBackupView(await this.api.getBackup());
      if (!v) throw new Error('The Mac sent backup settings this app could not read.');
      this.view.set(v);
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
    if (f && !f.startsWith('/') && !f.startsWith('~')) {
      this.error.set("Enter the folder's full path, starting with / (or ~ for your home folder).");
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
      const v = normalizeBackupView(await this.api.setBackupSettings(patch));
      if (v) this.view.set(v);
      else await this.load();
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
   * being backed up already answers that job, which is followed once. */
  protected async push(account?: string): Promise<void> {
    if (this.pushing()) return;
    this.pushing.set(account ?? 'all');
    this.error.set('');
    this.notice.set('');
    try {
      const res = await this.api.pushBackup(account);
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
      } else this.notice.set(`Backing up ${who}…`);
    } catch (e) {
      this.error.set(backupErrorText(e));
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

  protected openPull(account: string | null): void {
    this.pullFor.set(account ?? '');
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
