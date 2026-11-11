import { Component, DestroyRef, computed, effect, inject, signal, untracked } from '@angular/core';
import { Router, RouterLink } from '@angular/router';

import { ApiError, ApiService, describeError } from '../../core/api.service';
import { AuthService } from '../../core/auth.service';
import { computerNoun, tr } from '../../core/i18n/i18n';
import { TPipe } from '../../core/i18n/t.pipe';
import {
  Blocker,
  GRANT_NAMES,
  Grants,
  PeerMachine,
  Readiness,
  copyGrants,
  sameGrants,
} from '../../core/machines';
import { MachinesService } from '../../core/machines.service';
import { IconComponent } from '../../shared/icon.component';
import { CheckListComponent } from './check-list.component';
import { GrantsMatrixComponent } from './grants-matrix.component';
import { JoinSheetComponent } from './join-sheet.component';
import { grantShort, grantValue, pathName, serverText, thisComputerTitle } from './machines-text';
import { OptionsFormComponent } from './options-form.component';
import { PairSheetComponent } from './pair-sheet.component';

interface Step {
  n: number;
  title: string;
}

/**
 * *Set up Machines* (docs/PLAN-MACHINES.md §5.14, spikes/machines/UX.md §3):
 * this computer's checks, pairing the others, every computer's checks side
 * by side, what each may do on the others, the options, then a test between
 * every pair — and only then *Turn on Machines*. Desktop only; the step left
 * at is kept, so the Machines page can offer to go on.
 */
@Component({
  selector: 'mc-machines-setup',
  imports: [
    RouterLink,
    CheckListComponent,
    GrantsMatrixComponent,
    IconComponent,
    JoinSheetComponent,
    OptionsFormComponent,
    PairSheetComponent,
    TPipe,
  ],
  templateUrl: './setup.component.html',
  styleUrl: './setup.component.scss',
})
export class MachinesSetupComponent {
  private readonly api = inject(ApiService);
  private readonly auth = inject(AuthService);
  private readonly router = inject(Router);
  protected readonly machines = inject(MachinesService);

  protected readonly isDesktop = this.auth.isDesktop;
  protected readonly computer = computerNoun;
  protected readonly thisTitle = thisComputerTitle;
  protected readonly view = this.machines.view;
  protected readonly setup = this.machines.setup;
  protected readonly grantNames = GRANT_NAMES;

  protected readonly step = signal(1);
  protected readonly error = signal('');
  protected readonly notice = signal('');
  protected readonly busy = signal<string | null>(null);
  protected readonly checking = signal(false);
  protected readonly pairOpen = signal(false);
  protected readonly joinOpen = signal(false);
  protected readonly testing = signal(false);
  protected readonly results = signal<Readiness[] | null>(null);
  /** The required checks the server listed when it refused to turn on. */
  protected readonly refused = signal<Blocker[]>([]);
  /** Step 4's edits, by computer. */
  protected readonly drafts = signal<Record<string, Grants>>({});

  protected readonly steps = computed<Step[]>(() => [
    { n: 1, title: thisComputerTitle() },
    { n: 2, title: tr('Your machines') },
    { n: 3, title: tr('Every machine') },
    { n: 4, title: tr('What each may do') },
    { n: 5, title: tr('Accounts and folders') },
    { n: 6, title: tr('Options') },
    { n: 7, title: tr('Test, then turn on') },
  ]);
  protected readonly peers = computed(() => this.view()?.peers ?? []);
  protected readonly selfChecks = computed(() => this.setup()?.self.checks ?? []);
  protected readonly peerChecks = computed(() => this.setup()?.peers ?? []);
  protected readonly blockers = computed(() => this.setup()?.blockers ?? 0);
  protected readonly lastTests = computed(() => this.results() ?? this.setup()?.tests ?? []);

  constructor() {
    const destroyRef = inject(DestroyRef);
    const release = this.machines.watch();
    const releaseSetup = this.machines.watchSetup();
    destroyRef.onDestroy(() => {
      release();
      releaseSetup();
    });
    let started = false;
    effect(() => {
      if (started || !this.isDesktop()) return;
      started = true;
      untracked(() => void this.begin());
    });
  }

  private async begin(): Promise<void> {
    const v = await this.machines.load();
    if (v?.setupStep) this.step.set(v.setupStep);
    else void this.saveStep(1);
    await this.recheck();
  }

  // ───────────────────────────────────────────────────────────── Steps

  protected go(n: number): void {
    if (n < 1 || n > 7 || n === this.step()) return;
    this.step.set(n);
    this.error.set('');
    this.notice.set('');
    void this.saveStep(n);
    if (n === 3) void this.recheck();
  }

  private async saveStep(n: number): Promise<void> {
    try {
      this.machines.apply(await this.api.saveSetupProgress(n));
    } catch {
      // Only where to go on from: not worth an error.
    }
  }

  // ──────────────────────────────────────────────────────────── Checks

  protected async recheck(): Promise<void> {
    this.checking.set(true);
    try {
      this.machines.setup.set(await this.api.recheckMachines());
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
    if (check === 'relay') this.go(6);
  }

  // ──────────────────────────────────────────────────────────── Grants

  protected draftOf(p: PeerMachine): Grants {
    return this.drafts()[p.id] ?? p.grants;
  }

  protected setDraft(p: PeerMachine, g: Grants): void {
    this.drafts.update((d) => ({ ...d, [p.id]: copyGrants(g) }));
  }

  protected changed(p: PeerMachine): boolean {
    const d = this.drafts()[p.id];
    return !!d && !sameGrants(d, p.grants);
  }

  protected async saveGrants(p: PeerMachine): Promise<void> {
    const grants = this.drafts()[p.id];
    if (!grants) return;
    await this.withBusy(p.id, async () => {
      this.machines.apply(await this.api.updateMachine(p.id, { grants }));
      this.drafts.update((d) => {
        const next = { ...d };
        delete next[p.id];
        return next;
      });
      this.notice.set(tr('Saved what {name} may do here.', { name: p.name }));
    });
  }

  protected grantLabel(g: (typeof GRANT_NAMES)[number]): string {
    return grantShort(g);
  }

  protected grantVal(grants: Grants, g: (typeof GRANT_NAMES)[number]): string {
    return grantValue(grants[g]);
  }

  // ──────────────────────────────────────────────────── Test, turn on

  protected async test(): Promise<void> {
    this.testing.set(true);
    this.error.set('');
    try {
      const r = await this.api.testMachines();
      this.results.set(r.results ?? []);
    } catch (e) {
      this.error.set(describeError(e).text);
    } finally {
      this.testing.set(false);
    }
  }

  protected lineOf(r: Readiness, id: string): Readiness['tests'][number] | undefined {
    return r.tests.find((t) => t.id === id);
  }

  protected stateMark(state: string | undefined): string {
    return state === 'ok' ? '✓' : state === 'failed' ? '✕' : '–';
  }

  protected stateWord(state: string | undefined): string {
    return state === 'ok'
      ? tr('Passed')
      : state === 'failed'
        ? tr('Failed')
        : state === 'skipped'
          ? tr('Skipped')
          : tr('Not run');
  }

  protected testDetail(detail: string | undefined): string {
    return serverText(detail);
  }

  protected path(p: PeerMachine): string {
    return pathName(p.path);
  }

  protected async turnOn(): Promise<void> {
    this.refused.set([]);
    await this.withBusy('enable', async () => {
      try {
        this.machines.apply(await this.api.setMachinesEnabled(true));
        void this.router.navigate(['/machines']);
      } catch (e) {
        if (e instanceof ApiError && e.code === 'machines_blocked') {
          const list = e.extra['blockers'];
          const blockers = Array.isArray(list) ? (list as Blocker[]) : [];
          this.refused.set(blockers);
          this.error.set(
            blockers.length === 1
              ? tr('1 required check is left: fix it first.')
              : tr('{n} required checks are left: fix them first.', { n: blockers.length }),
          );
          await this.machines.loadSetup();
          return;
        }
        throw e;
      }
    });
  }

  protected optionsSaved(): void {
    this.notice.set(tr('Saved.'));
    void this.machines.load();
  }

  protected blockerTitle(b: Blocker): string {
    return serverText(b.title);
  }

  protected blockerDetail(b: Blocker): string {
    return serverText(b.detail);
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
