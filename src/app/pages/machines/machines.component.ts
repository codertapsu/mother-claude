import {
  Component,
  DestroyRef,
  computed,
  effect,
  inject,
  signal,
  untracked,
  viewChild,
} from '@angular/core';
import { FormsModule } from '@angular/forms';
import { Router, RouterLink } from '@angular/router';

import { ApiService, describeError } from '../../core/api.service';
import { AttentionService } from '../../core/attention.service';
import { AuthService } from '../../core/auth.service';
import { computerNoun, tr } from '../../core/i18n/i18n';
import { TPipe } from '../../core/i18n/t.pipe';
import {
  GRANT_NAMES,
  GrantName,
  Grants,
  MachineChecks,
  MachinesView,
  PeerMachine,
  PeerSessions,
  copyGrants,
  sameGrants,
} from '../../core/machines';
import { MachinesService } from '../../core/machines.service';
import { McpToolsStatus, PermissionInfo, ServicesInfo } from '../../core/models';
import { PermissionsService } from '../../core/permissions.service';
import { RemoteSessionsService } from '../../core/remote-sessions.service';
import { IconComponent } from '../../shared/icon.component';
import { PermissionsListComponent } from '../../shared/permissions-list.component';
import { SheetComponent } from '../../shared/sheet.component';
import { RelativeTimePipe } from '../../shared/time.pipes';
import { ServicesDetailsComponent } from '../services/services-details.component';
import { CheckListComponent } from './check-list.component';
import { ComputersHeadComponent } from './computers-head.component';
import { CopiesCardComponent } from './copies-card.component';
import { GrantsMatrixComponent } from './grants-matrix.component';
import { IntroduceOfferComponent } from './introduce-offer.component';
import {
  capabilityList,
  grantShort,
  grantValue,
  osName,
  pathName,
  thisComputerTitle,
} from './machines-text';

/**
 * *Computers › Overview* (docs/UI-REDESIGN.md §6.5; docs/PLAN-MACHINES.md,
 * spikes/machines/UX.md §1): a line when what your other computers asked of
 * this one waits in the Inbox; while it is off, the card that sets it up
 * (UX §1.1); this computer — its versions, its macOS permissions, Claude
 * Code tools, the checks that fail with their fix, and Claude Code's
 * background service (*Details*: the daemon, MCP servers, background jobs);
 * each paired computer with how it is reached, its versions (an older one
 * named with what it can't do yet, *Ask … to update*), its sessions, its
 * failing checks (*Ask … to fix*), what each may do on the other, rename,
 * *Revoke* and *Revoke everywhere*; *Pair your computers with each other*
 * (§5.12); the copies kept current (§5.9). Code sync, Activity and Options
 * are tabs of their own. The phone sees it read-only — pairing, grants,
 * revoking and the fixes belong to the desktop.
 */
@Component({
  selector: 'mc-machines',
  imports: [
    FormsModule,
    RouterLink,
    CheckListComponent,
    ComputersHeadComponent,
    CopiesCardComponent,
    GrantsMatrixComponent,
    IconComponent,
    IntroduceOfferComponent,
    PermissionsListComponent,
    RelativeTimePipe,
    ServicesDetailsComponent,
    SheetComponent,
    TPipe,
  ],
  templateUrl: './machines.component.html',
  styleUrl: './machines.component.scss',
})
export class MachinesComponent {
  private readonly api = inject(ApiService);
  private readonly auth = inject(AuthService);
  private readonly router = inject(Router);
  protected readonly machines = inject(MachinesService);
  private readonly remote = inject(RemoteSessionsService);
  private readonly attention = inject(AttentionService);
  protected readonly perms = inject(PermissionsService);

  private readonly head = viewChild(ComputersHeadComponent);

  protected readonly isDesktop = this.auth.isDesktop;
  protected readonly computer = computerNoun;
  protected readonly thisTitle = thisComputerTitle;
  protected readonly view = this.machines.view;
  protected readonly setup = this.machines.setup;
  protected readonly loadError = this.machines.error;
  protected readonly grantNames = GRANT_NAMES;

  protected readonly error = signal('');
  protected readonly notice = signal('');
  protected readonly busy = signal<string | null>(null);
  protected readonly checking = signal(false);

  // Renaming this computer or a paired one.
  protected readonly renaming = signal<string | null>(null);
  protected readonly renameText = signal('');
  // Changing a paired computer's grants.
  protected readonly editingGrants = signal<string | null>(null);
  protected readonly draftGrants = signal<Grants | null>(null);
  // Two-step revoke.
  protected readonly confirming = signal<string | null>(null);
  private confirmTimer?: ReturnType<typeof setTimeout>;

  /** Claude Code's background service, MCP servers and background jobs
   * (`GET /api/services`), and Claude Code tools (`GET /api/mcp`). */
  protected readonly services = signal<ServicesInfo | null>(null);
  protected readonly servicesError = signal('');
  protected readonly tools = signal<McpToolsStatus | null>(null);
  protected readonly detailsOpen = signal(false);
  protected readonly readingServices = signal(false);

  protected readonly peers = computed(() => this.view()?.peers ?? []);
  /** What your other computers asked of this one and is not answered yet:
   * it is answered in the Inbox (asks, introductions, a suggested setting). */
  protected readonly waiting = computed(
    () =>
      this.attention.asks().length +
      this.attention.intros().length +
      (this.attention.offer() ? 1 : 0),
  );
  /** In use: on, being set up, or something is paired. */
  protected readonly inUse = computed(() => {
    const v = this.view();
    return !!v && (v.enabled || v.setupStep !== null || v.peers.length > 0);
  });
  protected readonly selfChecks = computed(() => this.setup()?.self.checks ?? []);
  /** This computer's checks fail (its health). */
  protected readonly selfFailing = computed(() =>
    this.selfChecks().some((c) => c.state === 'blocker' || c.state === 'warning'),
  );
  /** Each paired computer's checks, by its id. */
  private readonly peerSetups = computed(
    () => new Map((this.setup()?.peers ?? []).map((p) => [p.machine, p])),
  );
  /** macOS's permissions (the desktop app on a Mac; none elsewhere). */
  protected readonly permissions = this.perms.permissions;

  constructor() {
    const destroyRef = inject(DestroyRef);
    const release = this.machines.watch();
    const releaseSetup = this.machines.watchSetup();
    destroyRef.onDestroy(() => {
      release();
      releaseSetup();
      clearTimeout(this.confirmTimer);
    });
    void this.machines.load();
    void this.loadServices();
    void this.loadTools();
    // The checks behind the health, once your computers are in use.
    let loaded = false;
    effect(() => {
      if (loaded || !this.inUse()) return;
      loaded = true;
      untracked(() => void this.machines.loadSetup());
    });
  }

  // ───────────────────────────────────────────────────────────── Words

  protected os(os: string): string {
    return osName(os);
  }

  protected path(p: PeerMachine): string {
    return pathName(p.path);
  }

  protected grantLabel(g: GrantName): string {
    return grantShort(g);
  }

  protected grantVal(grants: Grants, g: GrantName): string {
    return grantValue(grants[g]);
  }

  /** A paired computer's checks, once read. */
  protected peerSetup(p: PeerMachine): MachineChecks | undefined {
    return this.peerSetups().get(p.id);
  }

  /** It has a check that fails (required or recommended). */
  protected failing(s: MachineChecks | undefined): boolean {
    return !!s?.checks.some((c) => c.state === 'blocker' || c.state === 'warning');
  }

  /** How many of its sessions this computer knows, and how many run: live
   * from its frames once they came, else as `GET /api/machines` said. */
  protected sessionCounts(p: PeerMachine): PeerSessions | null {
    const live = this.remote.byId().get(p.id);
    if (live?.loaded) {
      return { total: live.sessions.length, running: live.sessions.filter((s) => s.live).length };
    }
    return p.sessions ?? null;
  }

  protected countsText(c: PeerSessions): string {
    return c.total === 1
      ? tr('1 session, {m} running', { m: c.running })
      : tr('{n} sessions, {m} running', { n: c.total, m: c.running });
  }

  protected versionsLine(p: {
    os: string;
    osVersion: string | null;
    versions: PeerMachine['versions'];
  }): string {
    const parts = [p.osVersion || this.os(p.os)];
    if (p.versions) {
      parts.push(tr('Mother Claude {v}', { v: p.versions.app }));
      parts.push(tr('protocol {n}', { n: p.versions.protocol }));
      if (p.versions.claude) parts.push(tr('Claude Code {v}', { v: p.versions.claude }));
    }
    return parts.join(' · ');
  }

  /** This computer's own versions (for an older/newer line). */
  private mine(): PeerMachine['versions'] {
    return this.view()?.self.versions ?? null;
  }

  /** `p` runs an older Mother Claude than this computer. */
  protected olderAppText(p: PeerMachine): string {
    return tr('{name} runs an older Mother Claude ({v}; this {computer}: {w}).', {
      name: p.name,
      v: p.versions?.app ?? '',
      w: this.mine()?.app ?? '',
      computer: this.computer(),
    });
  }

  /** This computer runs an older Mother Claude than `p`. */
  protected newerAppText(p: PeerMachine): string {
    return tr('This {computer} runs an older Mother Claude than {name} ({v}).', {
      name: p.name,
      v: p.versions?.app ?? '',
      computer: this.computer(),
    });
  }

  protected olderClaudeText(p: PeerMachine): string {
    return tr('{name} runs an older Claude Code ({v}; here: {w}).', {
      name: p.name,
      v: p.versions?.claude ?? '',
      w: this.mine()?.claude ?? '',
    });
  }

  protected newerClaudeText(p: PeerMachine): string {
    return tr('Claude Code here is older than on {name} ({v}).', {
      name: p.name,
      v: p.versions?.claude ?? '',
    });
  }

  /** What `p` can't do until it updates, in a sentence. */
  protected lacksText(p: PeerMachine): string {
    return tr('{name} can’t {what} until it updates.', {
      name: p.name,
      what: capabilityList(p.skew?.lacks ?? []),
    });
  }

  /** A macOS permission's mark: ✓ granted, ✕ missing, – asked when needed. */
  protected permMark(p: PermissionInfo): string {
    return p.granted === true ? '✓' : p.granted === false ? '✕' : '–';
  }

  protected permState(p: PermissionInfo): string {
    return p.granted === true
      ? tr('granted')
      : p.granted === false
        ? tr('not granted')
        : tr('ask when prompted');
  }

  /** Claude Code's background service, in a line: running or not, and its
   * background jobs. */
  protected readonly servicesLine = computed(() => {
    const s = this.services();
    if (!s) return '';
    const parts = [s.daemon?.reachable ? tr('running') : tr('not running')];
    const n = s.backgroundJobs?.length ?? 0;
    if (n) parts.push(n === 1 ? tr('1 background job') : tr('{n} background jobs', { n }));
    return parts.join(' · ');
  });

  // ─────────────────────────────────────────────────────────── Actions

  protected setUp(): void {
    void this.router.navigate(['/computers/setup']);
  }

  protected async recheck(): Promise<void> {
    this.checking.set(true);
    this.error.set('');
    try {
      this.machines.setup.set(await this.api.recheckMachines());
      await this.machines.load();
    } catch (e) {
      this.error.set(describeError(e).text);
    } finally {
      this.checking.set(false);
    }
  }

  protected async fix(check: string): Promise<void> {
    // *Update Mother Claude*: the Software updates page installs it.
    if (check === 'app') {
      void this.router.navigate(['/settings'], { fragment: 'updates' });
      return;
    }
    await this.withBusy(check, async () => {
      this.machines.setup.set(await this.api.fixMachineCheck(check));
      await this.machines.load();
      // An account's sign-in goes on in Accounts (its link, its code).
      if (check.startsWith('account:')) void this.router.navigate(['/accounts']);
    });
  }

  protected async ask(machine: string, name: string, check: string): Promise<void> {
    await this.withBusy(`${machine}:${check}`, async () => {
      await this.api.askMachineFix(machine, check);
      this.notice.set(tr('Asked {name}: it shows the fix to whoever is at it.', { name }));
    });
  }

  /** *Ask DESKTOP-PC to update* (Mother Claude: `app`; Claude Code:
   * `claudeCode`): shown there, done only when someone there confirms it. */
  protected async askUpdate(p: PeerMachine, check: 'app' | 'claudeCode'): Promise<void> {
    await this.withBusy(`${p.id}:${check}`, async () => {
      await this.api.askMachineFix(p.id, check);
      this.notice.set(
        tr('Asked {name}: it shows the update to whoever is at it.', { name: p.name }),
      );
    });
  }

  /** *Update it* here: Mother Claude on the Software updates page; Claude
   * Code by its own `claude update` (this computer's check's fix). */
  protected async updateHere(what: 'app' | 'claudeCode'): Promise<void> {
    if (what === 'app') {
      void this.router.navigate(['/settings'], { fragment: 'updates' });
      return;
    }
    await this.fix('claudeCode');
  }

  /** A check whose fix is a place: pairing opens the pairing, the relay
   * its option. */
  protected manual(check: string): void {
    if (check === 'pairing') this.head()?.pair();
    if (check === 'relay') void this.router.navigate(['/computers/options'], { fragment: 'relay' });
  }

  protected startRename(id: string, current: string): void {
    this.renaming.set(id);
    this.renameText.set(current);
  }

  protected async saveRename(id: string): Promise<void> {
    const name = this.renameText().trim();
    this.renaming.set(null);
    await this.withBusy(id, async () => {
      if (id === 'self') {
        if (!name) return;
        this.machines.apply(await this.api.renameThisMachine(name));
      } else {
        this.machines.apply(await this.api.updateMachine(id, { name }));
      }
    });
  }

  protected startGrants(p: PeerMachine): void {
    this.editingGrants.set(p.id);
    this.draftGrants.set(copyGrants(p.grants));
  }

  protected async saveGrants(p: PeerMachine): Promise<void> {
    const grants = this.draftGrants();
    this.editingGrants.set(null);
    if (!grants || sameGrants(grants, p.grants)) return;
    await this.withBusy(p.id, async () => {
      this.machines.apply(await this.api.updateMachine(p.id, { grants }));
    });
  }

  protected armed(key: string): boolean {
    return this.confirming() === key;
  }

  protected async revoke(p: PeerMachine): Promise<void> {
    const key = `revoke:${p.id}`;
    if (!this.armed(key)) {
      this.confirming.set(key);
      clearTimeout(this.confirmTimer);
      this.confirmTimer = setTimeout(() => this.confirming.set(null), 4000);
      return;
    }
    this.confirming.set(null);
    await this.withBusy(p.id, async () => {
      this.machines.apply(await this.api.revokeMachine(p.id));
      this.notice.set(tr('Revoked {name}. Its connections were cut.', { name: p.name }));
    });
  }

  /** *Revoke everywhere* (two taps): revoked here at once, and on every
   * other paired computer — now, or when one that is away is back (the
   * Inbox says which are still to be told). */
  protected async revokeEverywhere(p: PeerMachine): Promise<void> {
    const key = `everywhere:${p.id}`;
    if (!this.armed(key)) {
      this.confirming.set(key);
      clearTimeout(this.confirmTimer);
      this.confirmTimer = setTimeout(() => this.confirming.set(null), 4000);
      return;
    }
    this.confirming.set(null);
    await this.withBusy(p.id, async () => {
      const v: MachinesView = await this.api.revokeEverywhere(p.id);
      this.machines.apply(v);
      const r = v.revocations?.find((x) => x.machine === p.id);
      this.notice.set(
        r && r.pending.length
          ? tr('Revoked {name} here. {others} will revoke it when they next connect.', {
              name: p.name,
              others: r.pending.map((x) => x.name).join(', '),
            })
          : tr('Revoked {name} on all your computers.', { name: p.name }),
      );
    });
  }

  /** *Details*: what Claude Code runs here besides sessions, read again. */
  protected openDetails(): void {
    this.detailsOpen.set(true);
    void this.loadServices();
  }

  protected async loadServices(): Promise<void> {
    this.readingServices.set(true);
    try {
      this.services.set(await this.api.getServices());
      this.servicesError.set('');
    } catch (e) {
      this.servicesError.set(describeError(e).text);
    } finally {
      this.readingServices.set(false);
    }
  }

  private async loadTools(): Promise<void> {
    try {
      this.tools.set(await this.api.getMcpTools(false));
    } catch {
      // The line is left out.
    }
  }

  private async withBusy(key: string, fn: () => Promise<void>): Promise<void> {
    this.busy.set(key);
    this.error.set('');
    this.notice.set('');
    try {
      await fn();
    } catch (e) {
      this.error.set(describeError(e).text);
    } finally {
      this.busy.set(null);
    }
  }
}
