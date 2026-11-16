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

import { ApiError, ApiService, describeError } from '../../core/api.service';
import { tr } from '../../core/i18n/i18n';
import { TPipe } from '../../core/i18n/t.pipe';
import { ProjectInfo } from '../../core/models';
import { AvailablePlugin, PluginScope } from '../../core/plugins';
import { SheetComponent } from '../../shared/sheet.component';

/** The places a plugin can go, as the sheet offers them (English, translated
 * where shown). */
const PLACES: { scope: PluginScope; label: string; hint: string }[] = [
  {
    scope: 'user',
    label: 'Everywhere',
    hint: 'Every session you start, in every project.',
  },
  {
    scope: 'project',
    label: 'One project, for everyone',
    hint: "Saved in the project's .claude/settings.json: whoever works on it (and commits it) gets it too.",
  },
  {
    scope: 'local',
    label: 'One project, only you',
    hint: "Saved in the project's .claude/settings.local.json, which stays on this computer.",
  },
];

/**
 * *Install {name}* (docs/PLUGINS.md): where it goes — everywhere, or one
 * project for everyone or only you, the project picked from the ones Claude
 * Code knows (or typed) — then Claude Code installs it. When its
 * marketplace runs a command to install it, the sheet shows that command
 * and runs it only on *Run it and install*.
 */
@Component({
  selector: 'mc-plugin-install-sheet',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [SheetComponent, TPipe],
  template: `
    <mc-sheet
      [open]="!!plugin()"
      [title]="'Install {name}' | t: { name: plugin()?.name ?? '' }"
      (closed)="closed.emit()"
    >
      @if (plugin(); as p) {
        @if (p.description) {
          <p class="muted small">{{ p.description }}</p>
        }
        @if (command(); as c) {
          <div class="notice warn small" role="alert">
            <p>
              {{
                'Its marketplace installs it by running this command on this computer. Run it only if you trust where it comes from:'
                  | t
              }}
            </p>
            <pre
              class="cmd"
            ><code>{{ c.command || ('(the marketplace shows no command text)' | t) }}</code></pre>
          </div>
          <div class="acts">
            <button
              class="btn danger"
              type="button"
              [disabled]="busy()"
              (click)="install(c.sha256)"
            >
              {{ busy() ? ('Installing…' | t) : ('Run it and install' | t) }}
            </button>
            <button class="btn ghost" type="button" (click)="closed.emit()">
              {{ 'Cancel' | t }}
            </button>
          </div>
        } @else {
          <fieldset class="places">
            <legend>{{ 'Where should it be on?' | t }}</legend>
            @for (o of places; track o.scope) {
              <label class="place">
                <input
                  type="radio"
                  name="scope"
                  [value]="o.scope"
                  [checked]="scope() === o.scope"
                  (change)="scope.set(o.scope)"
                />
                <span>
                  <strong>{{ o.label | t }}</strong>
                  <span class="muted small">{{ o.hint | t }}</span>
                </span>
              </label>
            }
          </fieldset>
          @if (scope() !== 'user') {
            <label class="field">
              <span>{{ 'Project' | t }}</span>
              <select [value]="project()" (change)="project.set($any($event.target).value)">
                <option value="">{{ 'Choose a project…' | t }}</option>
                @for (pr of projects(); track pr.path) {
                  <option [value]="pr.path">{{ pr.name }} — {{ pr.path }}</option>
                }
              </select>
            </label>
          }
          <p class="muted small">
            {{
              'A plugin can run commands on this computer (its hooks and MCP servers). Install plugins only from marketplaces you trust.'
                | t
            }}
          </p>
          @if (error()) {
            <p class="notice error small" role="alert">{{ error() }}</p>
          }
          <div class="acts">
            <button
              class="btn primary"
              type="button"
              [disabled]="busy() || (scope() !== 'user' && !project())"
              (click)="install()"
            >
              {{ busy() ? ('Installing…' | t) : ('Install' | t) }}
            </button>
            <button class="btn ghost" type="button" (click)="closed.emit()">
              {{ 'Cancel' | t }}
            </button>
          </div>
        }
      }
    </mc-sheet>
  `,
  styles: [
    `
      .places {
        border: 0;
        margin: 0.5rem 0;
        padding: 0;
        display: flex;
        flex-direction: column;
        gap: 0.5rem;
      }
      .places legend {
        font-weight: 500;
        margin-bottom: 0.35rem;
      }
      .place {
        display: flex;
        gap: 0.5rem;
        align-items: flex-start;
      }
      .place span {
        display: flex;
        flex-direction: column;
      }
      .place strong {
        font-weight: 500;
      }
      .field {
        display: flex;
        flex-direction: column;
        gap: 0.25rem;
        margin: 0.5rem 0;
      }
      .cmd {
        white-space: pre-wrap;
        overflow-wrap: anywhere;
        background: var(--panel-2, var(--panel));
        padding: 0.5rem;
        border-radius: 0.4rem;
        margin: 0.4rem 0 0;
      }
      .acts {
        display: flex;
        flex-wrap: wrap;
        gap: 0.5rem;
        margin-top: 0.75rem;
      }
    `,
  ],
})
export class PluginInstallSheetComponent {
  private readonly api = inject(ApiService);

  /** The plugin to install (null: closed). */
  readonly plugin = input<AvailablePlugin | null>(null);
  /** The account whose plugins (none: the Default). */
  readonly account = input<string>('');
  /** A project to offer first (the page's). */
  readonly initialProject = input<string>('');
  readonly closed = output<void>();
  /** It was installed: where. */
  readonly installed = output<{ plugin: AvailablePlugin; scope: PluginScope }>();

  protected readonly places = PLACES;
  protected readonly scope = signal<PluginScope>('user');
  protected readonly project = signal('');
  protected readonly projects = signal<ProjectInfo[]>([]);
  protected readonly busy = signal(false);
  protected readonly error = signal('');
  /** The command its marketplace runs, to be accepted. */
  protected readonly command = signal<{ command: string; sha256: string } | null>(null);
  protected readonly open = computed(() => !!this.plugin());

  constructor() {
    effect(() => {
      if (!this.open()) return;
      untracked(() => {
        this.scope.set('user');
        this.project.set(this.initialProject());
        this.error.set('');
        this.command.set(null);
        void this.loadProjects();
      });
    });
  }

  private async loadProjects(): Promise<void> {
    try {
      const { projects } = await this.api.getProjects();
      this.projects.set(projects ?? []);
    } catch {
      this.projects.set([]);
    }
  }

  protected async install(acceptCommand?: string): Promise<void> {
    const p = this.plugin();
    if (!p || this.busy()) return;
    const scope = this.scope();
    this.busy.set(true);
    this.error.set('');
    try {
      await this.api.installPlugin({
        plugin: p.id,
        scope,
        ...(scope !== 'user' ? { project: this.project() } : {}),
        ...(this.account() ? { account: this.account() } : {}),
        ...(acceptCommand ? { acceptCommand } : {}),
      });
      this.installed.emit({ plugin: p, scope });
    } catch (e) {
      if (e instanceof ApiError && e.code === 'confirm_command' && !acceptCommand) {
        this.command.set({ command: e.detail('command'), sha256: e.detail('sha256') });
      } else {
        this.command.set(null);
        this.error.set(describeError(e).text || tr('It could not be installed.'));
      }
    } finally {
      this.busy.set(false);
    }
  }
}
