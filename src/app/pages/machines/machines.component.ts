import { Component, DestroyRef, computed, effect, inject, signal, untracked } from '@angular/core';
import { FormsModule } from '@angular/forms';
import { Router, RouterLink } from '@angular/router';

import { ApiService, describeError } from '../../core/api.service';
import { AuthService } from '../../core/auth.service';
import { copySize } from '../../core/copies';
import { computerNoun, tr } from '../../core/i18n/i18n';
import { TPipe } from '../../core/i18n/t.pipe';
import {
  CodeSettings,
  GRANT_NAMES,
  GrantName,
  Grants,
  MachinesSettings,
  MachinesView,
  PeerMachine,
  PeerSessions,
  Revocation,
  SettingsOffer,
  copyGrants,
  sameCode,
  sameGrants,
  sameRelay,
} from '../../core/machines';
import { MachinesService } from '../../core/machines.service';
import { MovesService } from '../../core/moves.service';
import { RemoteSessionsService } from '../../core/remote-sessions.service';
import { IconComponent } from '../../shared/icon.component';
import { MoveCardComponent } from '../../shared/move-card.component';
import { RelativeTimePipe } from '../../shared/time.pipes';
import { AsksCardComponent } from './asks-card.component';
import { CheckListComponent } from './check-list.component';
import { CopiesCardComponent } from './copies-card.component';
import { GrantsMatrixComponent } from './grants-matrix.component';
import { IntroSheetComponent } from './intro-sheet.component';
import { IntroduceOfferComponent } from './introduce-offer.component';
import { IntrosCardComponent } from './intros-card.component';
import { JoinSheetComponent } from './join-sheet.component';
import {
  capabilityList,
  codeText,
  grantShort,
  grantValue,
  osName,
  pathName,
  revocationText,
  thisComputerTitle,
} from './machines-text';
import { OptionsFormComponent, relayText } from './options-form.component';
import { PairSheetComponent } from './pair-sheet.component';
import { RunsCardComponent } from './runs-card.component';
import { SyncCardComponent } from './sync-card.component';

/**
 * Machines (docs/PLAN-MACHINES.md, spikes/machines/UX.md §1): this
 * computer, the paired ones with what each may do here and their versions
 * (an older one named with what it can't do yet, *Update it*), Machines
 * health, introductions (two of your computers pair with each other, each
 * confirming the six digits: §5.12), *Revoke everywhere*, the projects kept
 * in sync with another computer (*Code sync*, §5.15), the copies kept
 * current between computers (*Copies*, §5.9), the options, and *Recent
 * moves* of sessions to and from this computer (§1.2, §7.2–§7.3: cancel,
 * try again, undo). Off until the setup passed (§1.1). The phone sees it
 * read-only — pairing, grants, revoking and options belong to the desktop
 * — but may act on a move, as a device may.
 */
@Component({
  selector: 'mc-machines',
  imports: [
    FormsModule,
    RouterLink,
    AsksCardComponent,
    CheckListComponent,
    CopiesCardComponent,
    GrantsMatrixComponent,
    IconComponent,
    IntroSheetComponent,
    IntroduceOfferComponent,
    IntrosCardComponent,
    JoinSheetComponent,
    MoveCardComponent,
    OptionsFormComponent,
    PairSheetComponent,
    RelativeTimePipe,
    RunsCardComponent,
    SyncCardComponent,
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
  private readonly moves = inject(MovesService);

  protected readonly isDesktop = this.auth.isDesktop;
  protected readonly computer = computerNoun;
  protected readonly thisTitle = thisComputerTitle;
  protected readonly view = this.machines.view;
  protected readonly setup = this.machines.setup;
  protected readonly loadError = this.machines.error;
  protected readonly grantNames = GRANT_NAMES;

  protected readonly error = signal('');
  protected readonly notice = signal('');
  protected readonly pairOpen = signal(false);
  protected readonly joinOpen = signal(false);
  /** The pairing an accepted introduction runs. */
  protected readonly introOpen = signal(false);
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
  // The options card's form.
  protected readonly editingOptions = signal(false);

  protected readonly peers = computed(() => this.view()?.peers ?? []);
  /** This computer's moves of sessions, sent and received (UX §1.2). */
  protected readonly recentMoves = this.moves.here;
  protected readonly allMoves = signal(false);
  protected readonly shownMoves = computed(() => {
    const all = this.recentMoves();
    return this.allMoves() ? all : all.slice(0, 5);
  });
  /** Machines is on, or being set up, or something is paired. */
  protected readonly inUse = computed(() => {
    const v = this.view();
    return !!v && (v.enabled || v.setupStep !== null || v.peers.length > 0);
  });
  protected readonly selfChecks = computed(() => this.setup()?.self.checks ?? []);
  protected readonly peerChecks = computed(() => this.setup()?.peers ?? []);
  protected readonly healthText = computed(() => {
    const h = this.view()?.health;
    if (!h || h.state === 'unknown') return tr('Not checked yet');
    if (h.blockers)
      return h.blockers === 1 ? tr('1 required') : tr('{n} required', { n: h.blockers });
    if (h.warnings) {
      return h.warnings === 1 ? tr('1 recommended') : tr('{n} recommended', { n: h.warnings });
    }
    return tr('All good');
  });

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
    void this.moves.load();
    // The checks behind the health card, once Machines is in use.
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

  protected relay(s: MachinesSettings | undefined): string {
    return relayText(s?.relay);
  }

  protected codeText(c: CodeSettings | null | undefined): string {
    return codeText(c);
  }

  /** What this computer keeps of each other computer's copies. */
  protected copiesCapText(s: MachinesSettings): string {
    return tr('at most {size} of each computer’s', {
      size: copySize(s.copiesCapBytes ?? 2 * 1024 ** 3),
    });
  }

  /** The offer's relay differs from this computer's. */
  protected relayOffered(o: SettingsOffer, s: MachinesSettings): boolean {
    return !sameRelay(o.relay, s.relay);
  }

  /** The offer's code options differ from this computer's. */
  protected codeOffered(o: SettingsOffer, s: MachinesSettings): boolean {
    return !!o.code && !sameCode(o.code, s.code);
  }

  /** An ask was answered here: the view and the checks change. */
  protected askAnswered(): void {
    void this.machines.load();
    void this.machines.loadSetup();
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
      parts.push(tr('Machines protocol {n}', { n: p.versions.protocol }));
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

  /** The computers that may still revoke one, in a sentence. */
  protected revocationLine(r: Revocation): string {
    return revocationText(r);
  }

  // ─────────────────────────────────────────────────────────── Actions

  protected setUp(): void {
    void this.router.navigate(['/machines/setup']);
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

  protected manual(check: string): void {
    if (check === 'pairing') this.pairOpen.set(true);
    if (check === 'relay') this.editingOptions.set(true);
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
   * other paired computer — now, or when one that is away is back. */
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

  protected async answerOffer(accept: boolean): Promise<void> {
    await this.withBusy('offer', async () => {
      this.machines.apply(await this.api.answerSettingsOffer(accept));
    });
  }

  protected optionsSaved(): void {
    this.editingOptions.set(false);
    void this.machines.load();
  }

  protected async turnOff(): Promise<void> {
    await this.withBusy('enable', async () => {
      this.machines.apply(await this.api.setMachinesEnabled(false));
    });
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
