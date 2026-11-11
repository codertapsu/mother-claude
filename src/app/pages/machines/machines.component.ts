import { Component, DestroyRef, computed, effect, inject, signal, untracked } from '@angular/core';
import { FormsModule } from '@angular/forms';
import { Router, RouterLink } from '@angular/router';

import { ApiService, describeError } from '../../core/api.service';
import { AuthService } from '../../core/auth.service';
import { computerNoun, tr } from '../../core/i18n/i18n';
import { TPipe } from '../../core/i18n/t.pipe';
import {
  GRANT_NAMES,
  GrantName,
  Grants,
  MachinesSettings,
  PeerMachine,
  copyGrants,
  sameGrants,
} from '../../core/machines';
import { MachinesService } from '../../core/machines.service';
import { IconComponent } from '../../shared/icon.component';
import { RelativeTimePipe } from '../../shared/time.pipes';
import { CheckListComponent } from './check-list.component';
import { GrantsMatrixComponent } from './grants-matrix.component';
import { JoinSheetComponent } from './join-sheet.component';
import { grantShort, grantValue, osName, pathName, thisComputerTitle } from './machines-text';
import { OptionsFormComponent, relayText } from './options-form.component';
import { PairSheetComponent } from './pair-sheet.component';

/**
 * Machines (docs/PLAN-MACHINES.md, spikes/machines/UX.md §1): this
 * computer, the paired ones with what each may do here, Machines health,
 * and the options. Off until the setup passed (§1.1). The phone sees it
 * read-only: pairing, grants, revoking and options belong to the desktop.
 */
@Component({
  selector: 'mc-machines',
  imports: [
    FormsModule,
    RouterLink,
    CheckListComponent,
    GrantsMatrixComponent,
    IconComponent,
    JoinSheetComponent,
    OptionsFormComponent,
    PairSheetComponent,
    RelativeTimePipe,
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

  protected versionsLine(p: {
    os: string;
    osVersion: string | null;
    versions: PeerMachine['versions'];
  }): string {
    const parts = [p.osVersion || this.os(p.os)];
    if (p.versions) {
      parts.push(tr('Mother Claude {v}', { v: p.versions.app }));
      if (p.versions.claude) parts.push(tr('Claude Code {v}', { v: p.versions.claude }));
    }
    return parts.join(' · ');
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
    await this.withBusy(check, async () => {
      this.machines.setup.set(await this.api.fixMachineCheck(check));
      await this.machines.load();
    });
  }

  protected async ask(machine: string, name: string, check: string): Promise<void> {
    await this.withBusy(`${machine}:${check}`, async () => {
      await this.api.askMachineFix(machine, check);
      this.notice.set(tr('Asked {name}: it shows the fix to whoever is at it.', { name }));
    });
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
