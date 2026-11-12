import {
  ChangeDetectionStrategy,
  Component,
  computed,
  effect,
  inject,
  input,
  output,
  signal,
  untracked,
} from '@angular/core';
import { FormsModule } from '@angular/forms';
import { RouterLink } from '@angular/router';

import { ApiService, describeError } from '../../core/api.service';
import { tr } from '../../core/i18n/i18n';
import { TPipe } from '../../core/i18n/t.pipe';
import { ProjectOptions } from '../../core/machines';
import { MachinesService } from '../../core/machines.service';
import { ProjectInfo } from '../../core/models';
import { samePath } from '../../core/paths';
import { SyncService } from '../../core/sync.service';
import { SheetComponent } from '../../shared/sheet.component';

/**
 * *Keep a project in sync…* (docs/UI-REDESIGN.md §6.5, docs/PLAN-MACHINES.md
 * §5.15), on Computers › Code sync: the project, the computer to keep it the
 * same with, and the files git ignores that travel with it (changed in
 * *Options › Each project*: adding one needs the dangerous right). Saving
 * asks the other computer once; syncing starts when someone there says
 * yes. No more going through the setup.
 */
@Component({
  selector: 'mc-keep-in-sync-sheet',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [FormsModule, RouterLink, SheetComponent, TPipe],
  template: `
    <mc-sheet [open]="open()" [title]="'Keep a project in sync' | t" (closed)="closed.emit()">
      <form class="stack" (ngSubmit)="save()">
        @if (error()) {
          <p class="notice error small" role="alert">{{ error() }}</p>
        }
        <label class="field">
          <span class="field-label">{{ 'Project' | t }}</span>
          <select name="project" [ngModel]="project()" (ngModelChange)="project.set($event)">
            <option value="">{{ 'Choose a project' | t }}</option>
            @for (c of candidates(); track c.path) {
              <option [value]="c.path">{{ c.name }} — {{ c.path }}</option>
            }
          </select>
        </label>
        @if (synced(); as p) {
          <p class="muted small" role="status">
            {{ 'Already kept in sync with {name}.' | t: { name: p.machineName } }}
          </p>
        }

        <fieldset class="with">
          <legend class="field-label">{{ 'With' | t }}</legend>
          @for (m of peers(); track m.id) {
            <label class="check">
              <input
                type="radio"
                name="machine"
                [value]="m.id"
                [ngModel]="machine()"
                (ngModelChange)="machine.set($event)"
              />
              {{ m.name }}
              @if (!m.online) {
                <span class="muted small">{{ '(offline)' | t }}</span>
              }
            </label>
          }
        </fieldset>

        @if (project()) {
          <p class="small carried">
            {{ 'Files git ignores that travel with it:' | t }}
            @if (carried().length) {
              <span class="mono">{{ carried().join(', ') }}</span>
            } @else {
              <span class="muted">{{ 'none' | t }}</span>
            }
            <a routerLink="/computers/options" fragment="each-project" (click)="closed.emit()">{{
              'Change' | t
            }}</a>
          </p>
        }

        <p class="muted small">
          {{
            'Its commits, its branch and its uncommitted changes go to the other computer a few seconds after they change, both ways; files git ignores stay, but for the ones it carries. Nothing is merged: when both changed it, you choose which to keep.'
              | t
          }}
        </p>
        @if (machineName(); as name) {
          <p class="small">
            {{ '{name} is asked once; syncing starts when someone there says yes.' | t: { name } }}
          </p>
        }

        <div class="row">
          <button
            type="submit"
            class="btn primary"
            [disabled]="!project() || !machine() || !!synced() || busy()"
          >
            {{ busy() ? ('Asking…' | t) : ('Keep it in sync' | t) }}
          </button>
          <button type="button" class="btn ghost" (click)="closed.emit()">
            {{ 'Cancel' | t }}
          </button>
        </div>
      </form>
    </mc-sheet>
  `,
  styles: [
    `
      fieldset.with {
        border: 0;
        margin: 0;
        padding: 0;
        display: flex;
        flex-direction: column;
        gap: 0.25rem;
      }
      .carried {
        display: flex;
        flex-wrap: wrap;
        gap: 0.35rem;
        align-items: baseline;
        margin: 0;
      }
      .carried .mono {
        overflow-wrap: anywhere;
      }
    `,
  ],
})
export class KeepInSyncSheetComponent {
  private readonly api = inject(ApiService);
  private readonly machines = inject(MachinesService);
  private readonly sync = inject(SyncService);

  readonly open = input(false);
  readonly closed = output<void>();
  /** Asked: the other computer's name. */
  readonly asked = output<string>();

  protected readonly project = signal('');
  protected readonly machine = signal('');
  protected readonly busy = signal(false);
  protected readonly error = signal('');
  protected readonly candidates = signal<ProjectInfo[]>([]);
  /** Projects with options of their own (the files they carry). */
  private readonly options = signal<ProjectOptions[]>([]);

  protected readonly peers = computed(() => this.machines.view()?.peers ?? []);
  protected readonly machineName = computed(
    () => this.peers().find((p) => p.id === this.machine())?.name ?? '',
  );
  /** The project chosen is kept in sync already (or asked). */
  protected readonly synced = computed(() => {
    const p = this.sync.pairOf(this.project());
    return p && p.status.state !== 'off' ? p : null;
  });
  /** The files git ignores the project chosen carries. */
  protected readonly carried = computed(() => {
    const project = this.project();
    const o = this.options().find((x) => !!x.project && samePath(x.project, project));
    return o?.recipe?.carryIgnored ?? [];
  });

  constructor() {
    // Each time it opens: the projects, their options, and a computer
    // picked (the first one online).
    effect(() => {
      if (!this.open()) return;
      untracked(() => {
        this.error.set('');
        this.project.set('');
        const online = this.peers().find((p) => p.online) ?? this.peers()[0];
        this.machine.set(online?.id ?? '');
        void this.load();
      });
    });
  }

  private async load(): Promise<void> {
    try {
      const [projects, settings] = await Promise.all([
        this.api.getProjects(),
        this.api.getMachinesSettings(),
      ]);
      this.candidates.set(projects.projects);
      this.options.set(settings.projects ?? []);
    } catch (e) {
      this.error.set(describeError(e).text);
    }
  }

  protected async save(): Promise<void> {
    const project = this.project();
    const machine = this.machine();
    if (!project || !machine || this.busy()) return;
    this.busy.set(true);
    this.error.set('');
    try {
      this.sync.apply(await this.api.setSync(project, machine, true));
      this.asked.emit(this.machineName() || tr('the other computer'));
      this.closed.emit();
    } catch (e) {
      this.error.set(describeError(e).text);
    } finally {
      this.busy.set(false);
    }
  }
}
