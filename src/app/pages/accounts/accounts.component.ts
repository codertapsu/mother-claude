import {
  Component,
  DestroyRef,
  computed,
  effect,
  inject,
  linkedSignal,
  signal,
} from '@angular/core';
import { FormsModule } from '@angular/forms';

import { DEFAULT_ACCOUNT, isAccountColor, needsCopy } from '../../core/accounts';
import { AccountsService } from '../../core/accounts.service';
import { ApiService, describeError } from '../../core/api.service';
import { AuthService } from '../../core/auth.service';
import { normalizeJob } from '../../core/data-jobs';
import { DataJobsService } from '../../core/data-jobs.service';
import { AccountDefaults, AccountView, JobView, ProjectInfo } from '../../core/models';
import { accountLocation, anyTarget } from '../../core/move-data';
import { NowService } from '../../core/now.service';
import { PlanUsageService } from '../../core/plan-usage.service';
import { accountSigningIn, signInWaitText } from '../../core/sign-in';
import { StorageService } from '../../core/storage.service';
import {
  SignInLine,
  TOKEN_NO_USAGE,
  accountKindLabel,
  hooksPluginText,
  secretStoreText,
  signInErrorText,
  signInLine,
  usageLine,
} from '../../shared/accounts';
import { DataJobComponent } from '../../shared/data-job.component';
import { CompactMeter, compactPlan } from '../../shared/fleet-summary.component';
import { IconComponent } from '../../shared/icon.component';
import { MoveDataSheetComponent, MoveStart } from '../../shared/move-data-sheet.component';
import { PlanUsageComponent } from '../../shared/plan-usage.component';
import { Tone, shortPath } from '../../shared/util';
import { AccountAddSheetComponent } from './account-add-sheet.component';
import { AccountSheetComponent } from './account-sheet.component';
import { IdentityWarningComponent } from './identity-warning.component';

/** Recent moves and copies listed (their Undo lasts seven days). */
const MOVES_SHOWN = 6;

/** While a sign-in is being watched (or a check runs), ask this often:
 * `accounts` frames bring the change too, but not while polling. */
const WATCH_MS = 2_500;

interface Row {
  a: AccountView;
  color: string;
  kind: string;
  signIn: SignInLine;
  live: number;
  /** "Default", "New sessions", "API": what it is the default for. */
  roles: string[];
  meters: CompactMeter[];
  tone: Tone;
  /** Its plan usage, spoken. */
  usageText: string;
  hasUsage: boolean;
  /** The one thing it needs from the desktop now. */
  action: 'signin' | 'token' | null;
  /** Its sign-in runs (its panel is in its sheet). */
  signingIn: boolean;
  /** Another account's sign-in runs: "Waiting for Work's sign-in". */
  wait: string;
  /** Its email when the sign-in line doesn't say it ('' otherwise). */
  email: string;
  /** Its hooks plugin in a few words ('' for a token account: it uses the
   * Default's). */
  hooks: string;
  /** A directory account other than the Default: its plugin is its own to
   * install (the Default's is foreign control's, on the Remote page). */
  ownPlugin: boolean;
}

/** A pair of accounts whose copies the desktop may allow. */
interface ConsentPair {
  key: string;
  from: string;
  to: string;
  label: string;
}

interface ProjectRow {
  dir: string;
  id: string;
  name: string;
  /** The account still exists. */
  known: boolean;
}

/**
 * Settings › Accounts (docs/ACCOUNTS.md §6.1): every Claude account with
 * its kind, directory, sign-in, live sessions and plan usage; renaming and
 * recolouring them, the defaults (new sessions, the API, per project) and
 * the copy consents — allowed ahead of time, or revoked — and *Move or copy
 * data…* on each account, from the desktop or a paired phone (§13.9). On
 * the desktop only: adding, signing in (a browser login or a token),
 * re-checking and removing them, and the hooks plugin in a directory
 * account's own directory (the Default's is foreign control's).
 */
@Component({
  selector: 'mc-accounts',
  imports: [
    FormsModule,
    AccountAddSheetComponent,
    AccountSheetComponent,
    DataJobComponent,
    IconComponent,
    IdentityWarningComponent,
    MoveDataSheetComponent,
    PlanUsageComponent,
  ],
  templateUrl: './accounts.component.html',
  styleUrl: './accounts.component.scss',
})
export class AccountsComponent {
  protected readonly accounts = inject(AccountsService);
  private plan = inject(PlanUsageService);
  private api = inject(ApiService);
  private clock = inject(NowService);
  private storage = inject(StorageService);
  protected readonly isDesktop = inject(AuthService).isDesktop;

  protected readonly tokenNoUsage = TOKEN_NO_USAGE;
  protected readonly adding = signal(false);
  /** The account whose sheet is open. */
  protected readonly managing = signal<string | null>(null);
  private readonly openUsage = signal<ReadonlySet<string>>(new Set());
  protected readonly busyId = signal<string | null>(null);
  protected readonly saving = signal(false);
  protected readonly error = signal('');
  protected readonly projectError = signal('');
  protected readonly projects = signal<ProjectInfo[]>([]);
  protected readonly newDir = signal('');
  /** The account a new project rule names: the first one that isn't the
   * Default, until another is picked. */
  protected readonly newDirAccount = linkedSignal(
    () => this.accounts.list().find((a) => !a.isDefault)?.id ?? DEFAULT_ACCOUNT,
  );

  protected readonly defaults = computed(() => this.accounts.view()?.defaults ?? null);
  protected readonly rows = computed<Row[]>(() => {
    const now = this.clock.now();
    const d = this.defaults();
    const multi = this.accounts.multi();
    const live = this.accounts.liveCounts();
    const list = this.accounts.list();
    return list.map((a) => {
      const roles: string[] = [];
      if (a.isDefault && a.name !== 'Default') roles.push('Default');
      if (multi && d?.newSession === a.id) roles.push('New sessions');
      if (multi && d?.bridge === a.id) roles.push('API');
      const usage = this.plan.usageOf(a.id);
      const compact = compactPlan(usage, now);
      const line = signInLine(a);
      return {
        a,
        color: isAccountColor(a.color) ? a.color : 'var(--accent)',
        kind: accountKindLabel(a),
        signIn: line,
        signingIn: accountSigningIn(a),
        wait: signInWaitText(a, list),
        email: a.email && !line.text.includes(a.email) ? a.email : '',
        live: live.get(a.id) ?? 0,
        roles,
        meters: compact?.meters ?? [],
        tone: compact?.tone ?? 'muted',
        usageText: usageLine(usage, now) || 'not read yet',
        hasUsage: !!compact,
        action: needs(a),
        hooks: hooksPluginText(a),
        ownPlugin: a.kind === 'directory' && !a.isDefault,
      };
    });
  });
  protected readonly projectRows = computed<ProjectRow[]>(() => {
    const d = this.defaults();
    if (!d) return [];
    return Object.entries(d.byProject)
      .sort(([a], [b]) => a.localeCompare(b))
      .map(([dir, id]) => {
        const a = this.accounts.byId(id);
        return { dir, id, name: a?.name ?? 'A removed account', known: !!a };
      });
  });
  protected readonly consent = computed(() =>
    (this.accounts.view()?.copyConsent ?? []).map((c) => ({
      ...c,
      fromName: this.nameOf(c.from),
      toName: this.nameOf(c.to),
    })),
  );
  protected readonly secretText = computed(() =>
    secretStoreText(this.accounts.view()?.secretStore),
  );
  /** Pairs of accounts in different directories not allowed yet: what the
   * desktop may allow before anyone continues a conversation across them. */
  protected readonly consentPairs = computed<ConsentPair[]>(() => {
    const list = this.accounts.list();
    const out: ConsentPair[] = [];
    for (const from of list) {
      for (const to of list) {
        if (from.id === to.id || !needsCopy(from, to)) continue;
        if (this.accounts.hasCopyConsent(from.id, to.id)) continue;
        out.push({
          key: `${from.id}>${to.id}`,
          from: from.id,
          to: to.id,
          label: `${from.name} → ${to.name}`,
        });
      }
    }
    return out;
  });
  /** The pair the form would allow: the first until another is picked. */
  protected readonly newPair = linkedSignal(() => this.consentPairs()[0]?.key ?? '');
  /** `<account id>:install|uninstall` while the hooks plugin changes. */
  protected readonly hooksBusy = signal<string | null>(null);
  protected readonly notice = signal('');
  /** Somewhere to move an account's data: another account, or a storage. */
  protected readonly moveOffered = computed(() =>
    anyTarget(this.accounts.list(), this.storage.list() ?? []),
  );
  /** Move data, opened from an account (null: closed). */
  protected readonly moveStart = signal<MoveStart | null>(null);
  /** Recent moves and copies (`GET /data/jobs`), newest first: Undo stays
   * here once Move data is closed. */
  protected readonly moves = signal<JobView[]>([]);
  private readonly jobs = inject(DataJobsService);
  private readonly stopFollowing = new AbortController();

  constructor() {
    // Always fresh on opening (the desktop's read carries a sign-in link).
    void this.accounts.refresh();
    // The project picker's suggestions.
    void this.loadProjects();
    effect((onCleanup) => {
      if (!this.accounts.list().some((a) => accountSigningIn(a) || a.checking)) return;
      const timer = setInterval(() => void this.accounts.refresh(), WATCH_MS);
      onCleanup(() => clearInterval(timer));
    });
    void this.loadMoves();
    void this.storage.ensureLoaded();
    inject(DestroyRef).onDestroy(() => this.stopFollowing.abort());
  }

  /** Move data with everything of an account. */
  protected moveFrom(a: AccountView): void {
    this.moveStart.set({ from: accountLocation(a.id), what: 'all' });
  }

  /** Move data closed: its jobs join the list. */
  protected moveClosed(): void {
    this.moveStart.set(null);
    void this.loadMoves();
  }

  protected moveChanged(j: JobView): void {
    this.moves.update((list) => list.map((x) => (x.id === j.id ? j : x)));
  }

  protected short(p: string | undefined): string {
    return shortPath(p);
  }

  protected nameOf(id: string): string {
    return this.accounts.byId(id)?.name ?? 'A removed account';
  }

  protected liveText(n: number): string {
    return n === 0 ? 'No live sessions' : n === 1 ? '1 live session' : `${n} live sessions`;
  }

  protected isOpen(id: string): boolean {
    return this.openUsage().has(id);
  }

  protected toggleUsage(id: string): void {
    this.openUsage.update((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  }

  protected reload(): void {
    void this.accounts.refresh();
  }

  protected async signIn(a: AccountView): Promise<void> {
    if (this.busyId()) return;
    this.busyId.set(a.id);
    this.error.set('');
    try {
      await this.accounts.login(a.id);
      // Its sheet shows the sign-in: its link, a private window, Cancel.
      this.managing.set(a.id);
    } catch (e) {
      this.error.set(`${a.name}: ${signInErrorText(e, (id) => this.accounts.byId(id)?.name)}`);
    } finally {
      this.busyId.set(null);
    }
  }

  // ───────────────────────────────────────────── Defaults (desktop)

  protected async setNewSession(e: Event): Promise<void> {
    const d = this.defaults();
    const sel = e.target as HTMLSelectElement;
    if (!d || !sel.value || sel.value === d.newSession) return;
    const before = d.newSession;
    if (!(await this.save({ ...d, newSession: sel.value }))) sel.value = before;
  }

  protected async setBridge(e: Event): Promise<void> {
    const d = this.defaults();
    const sel = e.target as HTMLSelectElement;
    if (!d || !sel.value || sel.value === d.bridge) return;
    const before = d.bridge;
    if (!(await this.save({ ...d, bridge: sel.value }))) sel.value = before;
  }

  protected async setProject(dir: string, e: Event): Promise<void> {
    const d = this.defaults();
    const sel = e.target as HTMLSelectElement;
    const before = d?.byProject[dir];
    if (!d || !sel.value || sel.value === before) return;
    const ok = await this.save({ ...d, byProject: { ...d.byProject, [dir]: sel.value } });
    if (!ok && before) sel.value = before;
  }

  protected async removeProject(dir: string): Promise<void> {
    const d = this.defaults();
    if (!d) return;
    const byProject = Object.fromEntries(Object.entries(d.byProject).filter(([k]) => k !== dir));
    await this.save({ ...d, byProject });
  }

  protected async addProject(e: Event): Promise<void> {
    e.preventDefault();
    const d = this.defaults();
    if (!d) return;
    const dir = this.newDir().trim().replace(/\/+$/, '') || '/';
    if (!dir.startsWith('/')) {
      this.projectError.set("Enter the folder's full path, starting with /.");
      return;
    }
    if (dir.split('/').some((part) => part === '.' || part === '..')) {
      this.projectError.set('Enter the path without . or .. in it.');
      return;
    }
    const id = this.newDirAccount();
    if (!this.accounts.byId(id)) return;
    this.projectError.set('');
    if (await this.save({ ...d, byProject: { ...d.byProject, [dir]: id } })) this.newDir.set('');
  }

  protected async revoke(from: string, to: string): Promise<void> {
    await this.consentChange(from, to, false);
  }

  protected async allowPair(e: Event): Promise<void> {
    e.preventDefault();
    const p = this.consentPairs().find((x) => x.key === this.newPair());
    if (p) await this.consentChange(p.from, p.to, true);
  }

  private async consentChange(from: string, to: string, granted: boolean): Promise<void> {
    this.saving.set(true);
    this.error.set('');
    try {
      await this.accounts.setCopyConsent(from, to, granted);
    } catch (e) {
      this.error.set(describeError(e).text);
    } finally {
      this.saving.set(false);
    }
  }

  /** Desktop: the hooks plugin in a directory account's own directory. */
  protected async setHooks(a: AccountView, install: boolean): Promise<void> {
    if (this.hooksBusy()) return;
    this.hooksBusy.set(`${a.id}:${install ? 'install' : 'uninstall'}`);
    this.error.set('');
    this.notice.set('');
    try {
      await this.accounts.setHooksPlugin(a.id, install);
      this.notice.set(
        install
          ? `The hooks plugin is in ${a.name}'s directory now. Its sessions started before need /reload-plugins.`
          : `The hooks plugin is no longer in ${a.name}'s directory.`,
      );
    } catch (e) {
      this.error.set(`${a.name}: ${describeError(e).text}`);
    } finally {
      this.hooksBusy.set(null);
    }
  }

  private async save(next: AccountDefaults): Promise<boolean> {
    this.saving.set(true);
    this.error.set('');
    try {
      await this.accounts.setDefaults(next);
      return true;
    } catch (e) {
      this.error.set(`Could not save the defaults: ${describeError(e).text}`);
      return false;
    } finally {
      this.saving.set(false);
    }
  }

  /** The moves and copies the Mac still knows of; a running one is followed. */
  private async loadMoves(): Promise<void> {
    let list: JobView[];
    try {
      const { jobs } = await this.api.listDataJobs();
      list = (Array.isArray(jobs) ? jobs : [])
        .map(normalizeJob)
        .filter((j): j is JobView => !!j && (j.kind === 'move' || j.kind === 'copy'))
        .sort((a, b) => b.startedAt - a.startedAt)
        .slice(0, MOVES_SHOWN);
    } catch {
      // Only a convenience: the Move/Copy dialog has its own.
      return;
    }
    this.moves.set(list);
    for (const j of list) {
      if (j.state !== 'running') continue;
      void this.jobs
        .follow(j, (next) => this.moveChanged(next), this.stopFollowing.signal)
        .catch(() => undefined);
    }
  }

  private async loadProjects(): Promise<void> {
    try {
      const { projects } = await this.api.getProjects();
      this.projects.set(projects ?? []);
    } catch {
      // Only suggestions: any folder can still be typed.
    }
  }
}

/** What an account needs from the desktop now: a sign-in for a directory
 * account that is signed out (or failed), a token for a token account. */
function needs(a: AccountView): Row['action'] {
  if (accountSigningIn(a)) return null;
  if (a.kind === 'token') {
    return !a.tokenStored || a.attention === 'authFailed' ? 'token' : null;
  }
  return a.attention === 'signedOut' || a.attention === 'authFailed' ? 'signin' : null;
}
