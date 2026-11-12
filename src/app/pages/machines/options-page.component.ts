import { Component, DestroyRef, computed, inject, signal } from '@angular/core';
import { toSignal } from '@angular/core/rxjs-interop';
import { ActivatedRoute, RouterLink } from '@angular/router';

import { ApiService, describeError } from '../../core/api.service';
import { AuthService } from '../../core/auth.service';
import { copySize } from '../../core/copies';
import { computerNoun, tr } from '../../core/i18n/i18n';
import { TPipe } from '../../core/i18n/t.pipe';
import { CodeSettings, MachinesSettings, defaultCode } from '../../core/machines';
import { MachinesService } from '../../core/machines.service';
import { DesktopOnlyComponent } from '../../shared/desktop-only.component';
import { EmptyStateComponent } from '../../shared/empty-state.component';
import { AccountsFoldersComponent } from './accounts-folders.component';
import { ComputersHeadComponent } from './computers-head.component';
import { codeText } from './machines-text';
import { OptionsFormComponent, relayText } from './options-form.component';
import { ProjectOptionsComponent } from './project-options.component';

/**
 * *Computers › Options* (docs/UI-REDESIGN.md §6.5): how your computers meet
 * and move code, how much of the others' copies this one keeps and keeping
 * it awake (`mc-machines-options`, offered to the other computers); the
 * accounts on each computer and the folders that map a project between
 * them (`mc-machines-accounts-folders`); and each project's own options —
 * how its code moves, a recipe, its copies, code sync
 * (`mc-machines-project-options`) — all reachable without the setup. Then
 * *Run the setup again* and *Turn Computers off*. A phone reads them.
 */
@Component({
  selector: 'mc-computers-options',
  imports: [
    AccountsFoldersComponent,
    ComputersHeadComponent,
    DesktopOnlyComponent,
    EmptyStateComponent,
    OptionsFormComponent,
    ProjectOptionsComponent,
    RouterLink,
    TPipe,
  ],
  template: `
    <div class="page">
      <mc-computers-head />
      @if (error()) {
        <p class="notice error" role="alert">{{ error() }}</p>
      }

      @let v = machines.view();
      @if (!v) {
        @if (machines.error()) {
          <p class="notice error" role="alert">{{ machines.error() }}</p>
        } @else {
          <p class="muted" role="status">{{ 'Loading…' | t }}</p>
        }
      } @else if (!inUse()) {
        <mc-empty-state
          icon="settings"
          [heading]="'Not set up yet' | t"
          [text]="
            'Your computers’ options are chosen during the setup, and changed here afterwards.' | t
          "
          help="machines-setup"
        >
          @if (isDesktop()) {
            <a class="btn primary small" routerLink="/computers/setup">{{
              'Set up your computers' | t
            }}</a>
          }
        </mc-empty-state>
      } @else {
        <!-- ─────────────── How they meet, move code, keep copies ─────────────── -->
        <div class="section-head" id="relay">
          <h2>{{ 'General' | t }}</h2>
          <span class="spacer"></span>
          @if (isDesktop() && !editing()) {
            <button type="button" class="btn ghost small" (click)="editing.set(true)">
              {{ 'Change' | t }}
            </button>
          }
        </div>
        <section class="card options" [attr.aria-label]="'General' | t">
          @if (editing()) {
            <mc-machines-options
              [settings]="v.settings"
              [cancellable]="true"
              (saved)="saved()"
              (cancelled)="editing.set(false)"
            />
          } @else {
            <dl class="kv">
              <dt>{{ 'Relay' | t }}</dt>
              <dd>{{ relay(v.settings) }}</dd>
              <dt>{{ 'Code' | t }}</dt>
              <dd>{{ code(v.settings.code) }}</dd>
              <dt>{{ 'Copies' | t }}</dt>
              <dd>{{ copiesCap(v.settings) }}</dd>
              <dt>{{ 'Keep awake' | t }}</dt>
              <dd>
                {{
                  v.settings.keepAwake
                    ? ('while another computer uses this {computer}' | t: { computer: computer() })
                    : ('Off' | t)
                }}
              </dd>
            </dl>
          }
        </section>

        <!-- ─────────────── Accounts and folders (setup step 5) ─────────────── -->
        <div class="section-head" id="accounts-folders">
          <h2>{{ 'Accounts and folders' | t }}</h2>
        </div>
        <p class="muted small">
          {{
            'A session moves to the account of the same name, into the folder that maps its project. What needs someone at another computer is asked there.'
              | t
          }}
        </p>
        <mc-desktop-only [line]="'Changed on the desktop app.' | t">
          <section class="card">
            <mc-machines-accounts-folders />
          </section>
        </mc-desktop-only>

        <!-- ─────────────── Each project (setup step 6) ─────────────── -->
        <div class="section-head" id="each-project">
          <h2>{{ 'Each project' | t }}</h2>
        </div>
        <p class="muted small">
          {{
            'Each project can have its own: how its code moves; a recipe — what to ask Claude to run after a move, and the files git ignores that go with it; copies of its sessions kept on another computer; and code sync — its code kept the same on two computers, both ways, once both say yes.'
              | t
          }}
        </p>
        <section class="card">
          <mc-machines-project-options
            [defaults]="v.settings.code ?? defaults"
            [readonly]="!isDesktop()"
          />
        </section>

        @if (isDesktop()) {
          <div class="row page-foot">
            <a class="btn" routerLink="/computers/setup">{{ 'Run the setup again' | t }}</a>
            <span class="spacer"></span>
            @if (v.enabled) {
              <button type="button" class="btn ghost" [disabled]="busy()" (click)="turnOff()">
                {{ 'Turn Computers off' | t }}
              </button>
            }
          </div>
        }
      }
    </div>
  `,
  styles: [
    `
      .section-head {
        display: flex;
        align-items: center;
        gap: 0.5rem;
        margin: 1.4rem 0 0.55rem;
        scroll-margin-top: 4rem;
      }
      .section-head h2 {
        margin: 0;
      }
      .section-head + .muted {
        margin: -0.2rem 0 0.55rem;
      }
      .page-foot {
        margin-top: 1rem;
      }
    `,
  ],
})
export class OptionsPageComponent {
  private readonly api = inject(ApiService);
  protected readonly machines = inject(MachinesService);
  protected readonly isDesktop = inject(AuthService).isDesktop;
  protected readonly computer = computerNoun;
  protected readonly defaults = defaultCode();

  protected readonly error = signal('');
  protected readonly busy = signal(false);
  /** The general options' form is open: *Change*, or a check's fix that
   * sends here (`#relay`). */
  protected readonly editing = signal(false);

  protected readonly inUse = computed(() => {
    const v = this.machines.view();
    return !!v && (v.enabled || v.setupStep !== null || v.peers.length > 0);
  });

  constructor() {
    const release = this.machines.watch();
    inject(DestroyRef).onDestroy(release);
    void this.machines.load();
    const fragment = toSignal(inject(ActivatedRoute).fragment);
    if (fragment() === 'relay' && this.isDesktop()) this.editing.set(true);
  }

  protected relay(s: MachinesSettings): string {
    return relayText(s.relay);
  }

  protected code(c: CodeSettings | null | undefined): string {
    return codeText(c);
  }

  /** What this computer keeps of each other computer's copies. */
  protected copiesCap(s: MachinesSettings): string {
    return tr('at most {size} of each computer’s', {
      size: copySize(s.copiesCapBytes ?? 2 * 1024 ** 3),
    });
  }

  protected saved(): void {
    this.editing.set(false);
    void this.machines.load();
  }

  protected async turnOff(): Promise<void> {
    this.busy.set(true);
    this.error.set('');
    try {
      this.machines.apply(await this.api.setMachinesEnabled(false));
    } catch (e) {
      this.error.set(describeError(e).text);
    } finally {
      this.busy.set(false);
    }
  }
}
