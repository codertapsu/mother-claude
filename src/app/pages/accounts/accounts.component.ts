import {
  Component,
  computed,
  effect,
  inject,
  linkedSignal,
  signal,
  untracked,
} from '@angular/core';
import { FormsModule } from '@angular/forms';

import { DEFAULT_ACCOUNT, isAccountColor } from '../../core/accounts';
import { AccountsService } from '../../core/accounts.service';
import { ApiService, describeError } from '../../core/api.service';
import { AuthService } from '../../core/auth.service';
import { AccountDefaults, AccountView, ProjectInfo } from '../../core/models';
import { NowService } from '../../core/now.service';
import { PlanUsageService } from '../../core/plan-usage.service';
import {
  SignInLine,
  TOKEN_NO_USAGE,
  accountKindLabel,
  secretStoreText,
  signInLine,
  usageLine,
} from '../../shared/accounts';
import { CompactMeter, compactPlan } from '../../shared/fleet-summary.component';
import { IconComponent } from '../../shared/icon.component';
import { PlanUsageComponent } from '../../shared/plan-usage.component';
import { Tone, shortPath } from '../../shared/util';
import { AccountAddSheetComponent } from './account-add-sheet.component';
import { AccountSheetComponent } from './account-sheet.component';

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
 * its kind, directory, sign-in, live sessions and plan usage; on the
 * desktop adding, renaming, recolouring, signing in (a browser login or a
 * token), re-checking and removing them, the defaults (new sessions, the
 * API, per project) and the copy consents. Phones see all of it read-only:
 * their one account action is the picker when starting or continuing a
 * session (D9).
 */
@Component({
  selector: 'mc-accounts',
  imports: [
    FormsModule,
    AccountAddSheetComponent,
    AccountSheetComponent,
    IconComponent,
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
    return this.accounts.list().map((a) => {
      const roles: string[] = [];
      if (a.isDefault && a.name !== 'Default') roles.push('Default');
      if (multi && d?.newSession === a.id) roles.push('New sessions');
      if (multi && d?.bridge === a.id) roles.push('API');
      const usage = this.plan.usageOf(a.id);
      const compact = compactPlan(usage, now);
      return {
        a,
        color: isAccountColor(a.color) ? a.color : 'var(--accent)',
        kind: accountKindLabel(a),
        signIn: signInLine(a),
        live: live.get(a.id) ?? 0,
        roles,
        meters: compact?.meters ?? [],
        tone: compact?.tone ?? 'muted',
        usageText: usageLine(usage, now) || 'not read yet',
        hasUsage: !!compact,
        action: needs(a),
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

  constructor() {
    // Always fresh on opening (the desktop's read carries a sign-in link).
    void this.accounts.refresh();
    // The project picker's suggestions, for the desktop that edits rules.
    let projectsLoaded = false;
    effect(() => {
      if (!this.isDesktop() || projectsLoaded) return;
      projectsLoaded = true;
      untracked(() => void this.loadProjects());
    });
    effect((onCleanup) => {
      if (!this.accounts.list().some((a) => a.signingIn || a.checking)) return;
      const timer = setInterval(() => void this.accounts.refresh(), WATCH_MS);
      onCleanup(() => clearInterval(timer));
    });
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
    } catch (e) {
      this.error.set(`${a.name}: ${describeError(e).text}`);
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
    this.saving.set(true);
    this.error.set('');
    try {
      await this.accounts.setCopyConsent(from, to, false);
    } catch (e) {
      this.error.set(describeError(e).text);
    } finally {
      this.saving.set(false);
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
  if (a.signingIn) return null;
  if (a.kind === 'token') {
    return !a.tokenStored || a.attention === 'authFailed' ? 'token' : null;
  }
  return a.attention === 'signedOut' || a.attention === 'authFailed' ? 'signin' : null;
}
