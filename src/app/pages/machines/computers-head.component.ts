import { ChangeDetectionStrategy, Component, computed, inject, signal } from '@angular/core';

import { AuthService } from '../../core/auth.service';
import { tr } from '../../core/i18n/i18n';
import { TPipe } from '../../core/i18n/t.pipe';
import { MachinesService } from '../../core/machines.service';
import { HubTabsComponent } from '../../shared/hub-tabs.component';
import { IconComponent } from '../../shared/icon.component';
import { PageHeadComponent, PageState } from '../../shared/page-head.component';
import { JoinSheetComponent } from './join-sheet.component';
import { PairSheetComponent } from './pair-sheet.component';

/**
 * The head of every Computers page (docs/UI-REDESIGN.md §6.5): the name,
 * how your computers stand (✓ All good · ! 1 recommended · ✕ 2 required),
 * *Pair a computer* and *Join…* on the desktop once they are in use, one
 * sentence with *How it works ›*, and the tabs — Overview · Code sync ·
 * Activity · Options. A phone reads them; pairing is the desktop's.
 */
@Component({
  selector: 'mc-computers-head',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [
    HubTabsComponent,
    IconComponent,
    JoinSheetComponent,
    PageHeadComponent,
    PairSheetComponent,
    TPipe,
  ],
  template: `
    <mc-page-head
      [heading]="'Computers' | t"
      [lede]="
        'Use your other computers from here: see and drive their sessions, and move a session — with its code — to another computer and back.'
          | t
      "
      help="machines-setup"
      [state]="state()"
    >
      @if (isDesktop() && inUse()) {
        <button type="button" class="btn primary" (click)="pairOpen.set(true)">
          <mc-icon name="plus" [size]="15" /> {{ 'Pair a computer' | t }}
        </button>
        <button type="button" class="btn" (click)="joinOpen.set(true)">{{ 'Join…' | t }}</button>
      }
    </mc-page-head>
    <mc-hub-tabs place="computers" />
    @if (!isDesktop()) {
      <p class="notice small phone-note">
        {{ 'Your computers are set up from the desktop app.' | t }}
      </p>
    }
    @if (isDesktop()) {
      <mc-pair-machine-sheet [open]="pairOpen()" (closed)="pairOpen.set(false)" />
      <mc-join-machine-sheet [open]="joinOpen()" (closed)="joinOpen.set(false)" />
    }
  `,
  styles: [
    `
      :host {
        display: block;
      }
    `,
  ],
})
export class ComputersHeadComponent {
  private readonly machines = inject(MachinesService);
  protected readonly isDesktop = inject(AuthService).isDesktop;

  protected readonly pairOpen = signal(false);
  protected readonly joinOpen = signal(false);

  /** In use: on, being set up, or something is paired. */
  protected readonly inUse = computed(() => {
    const v = this.machines.view();
    return !!v && (v.enabled || v.setupStep !== null || v.peers.length > 0);
  });

  /** How your computers stand, while they are in use and were checked. */
  protected readonly state = computed<PageState | null>(() => {
    const h = this.machines.view()?.health;
    if (!this.inUse() || !h || h.state === 'unknown') return null;
    if (h.blockers) {
      return {
        tone: 'danger',
        text: h.blockers === 1 ? tr('1 required') : tr('{n} required', { n: h.blockers }),
      };
    }
    if (h.warnings) {
      return {
        tone: 'warn',
        text: h.warnings === 1 ? tr('1 recommended') : tr('{n} recommended', { n: h.warnings }),
      };
    }
    return { tone: 'ok', text: tr('All good') };
  });

  /** Open the pairing (a check's manual fix). */
  pair(): void {
    this.pairOpen.set(true);
  }
}
