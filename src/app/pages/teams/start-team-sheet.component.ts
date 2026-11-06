import { Component, computed, effect, inject, input, output, signal } from '@angular/core';
import { FormsModule } from '@angular/forms';

import { AccountsService } from '../../core/accounts.service';
import { ApiService, describeError } from '../../core/api.service';
import { AuthService } from '../../core/auth.service';
import { ProjectInfo } from '../../core/models';
import {
  RoleOverride,
  TeamView,
  TemplateEntry,
  applyOverrides,
  needsDangerous,
} from '../../core/teams';
import { SheetComponent } from '../../shared/sheet.component';

const EFFORTS = ['low', 'medium', 'high', 'xhigh', 'max'];
const MODES = ['default', 'acceptEdits', 'plan', 'auto', 'bypassPermissions', 'dontAsk'];

/** Start a team: a template on one of the known projects, an account, the
 * lead's brief and per-role changes. */
@Component({
  selector: 'mc-start-team-sheet',
  imports: [FormsModule, SheetComponent],
  template: `
    <mc-sheet [open]="open()" title="Start a team" (closed)="closed.emit()">
      <form class="stack form" (ngSubmit)="start()">
        <label class="field">
          <span class="field-label">Project</span>
          <select name="project" [ngModel]="project()" (ngModelChange)="project.set($event)">
            <option value="" disabled>Choose a project…</option>
            @for (p of projects(); track p.path) {
              <option [value]="p.path">{{ p.name }} — {{ p.path }}</option>
            }
          </select>
          <span class="field-hint">A folder Claude Code has worked in (or trusted).</span>
        </label>

        <label class="field">
          <span class="field-label">Template</span>
          <select
            name="template"
            [ngModel]="templateId()"
            (ngModelChange)="templateId.set($event); overrides.set({})"
          >
            @for (t of templates(); track t.id) {
              <option [value]="t.id">{{ t.name }}</option>
            }
          </select>
        </label>

        @if (accounts.multi()) {
          <label class="field">
            <span class="field-label">Account</span>
            <select name="account" [ngModel]="account()" (ngModelChange)="account.set($event)">
              <option value="">The project's account</option>
              @for (a of accounts.list(); track a.id) {
                <option [value]="a.id">{{ a.name }}</option>
              }
            </select>
          </label>
        }

        <label class="field">
          <span class="field-label">Brief for the lead (optional)</span>
          <textarea
            name="brief"
            rows="4"
            placeholder="What should the team build?"
            [ngModel]="brief()"
            (ngModelChange)="brief.set($event)"
          ></textarea>
          <span class="field-hint">The lead's first message. Without one it waits quietly.</span>
        </label>

        @if (template(); as t) {
          <fieldset class="roles">
            <legend class="field-label">Roles</legend>
            @for (r of t.roles; track r.id) {
              <div class="role">
                <div class="row">
                  <strong>{{ r.title }}</strong>
                  @if (r.lead) {
                    <span class="badge accent">lead</span>
                  }
                  @if (r.worktree) {
                    <span class="badge">worktree</span>
                  }
                  @if (r.readOnly) {
                    <span class="badge">read-only</span>
                  }
                </div>
                <div class="grid">
                  <label class="field">
                    <span class="field-hint">Model</span>
                    <input
                      [name]="'model-' + r.id"
                      [attr.name]="'model-' + r.id"
                      [placeholder]="r.model || 'default'"
                      [ngModel]="overrides()[r.id]?.model ?? ''"
                      (ngModelChange)="set(r.id, 'model', $event)"
                    />
                  </label>
                  <label class="field">
                    <span class="field-hint">Effort</span>
                    <select
                      [name]="'effort-' + r.id"
                      [attr.name]="'effort-' + r.id"
                      [ngModel]="overrides()[r.id]?.effort ?? ''"
                      (ngModelChange)="set(r.id, 'effort', $event)"
                    >
                      <option value="">{{ r.effort || 'default' }}</option>
                      @for (e of efforts; track e) {
                        <option [value]="e">{{ e }}</option>
                      }
                    </select>
                  </label>
                  <label class="field">
                    <span class="field-hint">Permission mode</span>
                    <select
                      [name]="'mode-' + r.id"
                      [attr.name]="'mode-' + r.id"
                      [ngModel]="overrides()[r.id]?.permissionMode ?? ''"
                      (ngModelChange)="set(r.id, 'permissionMode', $event)"
                    >
                      <option value="">{{ r.permissionMode || 'default' }}</option>
                      @for (m of modes; track m) {
                        <option [value]="m">{{ m }}</option>
                      }
                    </select>
                  </label>
                </div>
                <label class="check">
                  <input
                    type="checkbox"
                    [name]="'autostart-' + r.id"
                    [attr.name]="'autostart-' + r.id"
                    [ngModel]="overrides()[r.id]?.autostart ?? !!r.autostart"
                    (ngModelChange)="setAutostart(r.id, $event, !!r.autostart)"
                  />
                  <span>Start with the team</span>
                </label>
              </div>
            }
          </fieldset>
        }

        @if (dangerous()) {
          <p class="notice warn">
            A role runs without permission prompts.
            @if (!auth.canDangerous()) {
              Only this computer, or a device allowed dangerous actions, can start this team.
            }
          </p>
        }
        @if (error()) {
          <p class="notice error" role="alert">{{ error() }}</p>
        }
        <div class="row">
          <span class="spacer"></span>
          <button class="btn" type="button" (click)="closed.emit()">Cancel</button>
          <button class="btn primary" type="submit" [disabled]="!canStart()">
            {{ busy() ? 'Starting…' : 'Start the team' }}
          </button>
        </div>
      </form>
    </mc-sheet>
  `,
  styles: [
    `
      .form {
        gap: 0.85rem;
      }
      .roles {
        border: 1px solid var(--border);
        border-radius: var(--radius);
        padding: 0.6rem 0.75rem;
        margin: 0;
        display: flex;
        flex-direction: column;
        gap: 0.75rem;
        min-width: 0;
      }
      .role {
        display: flex;
        flex-direction: column;
        gap: 0.35rem;
      }
      .grid {
        display: grid;
        grid-template-columns: repeat(auto-fit, minmax(9rem, 1fr));
        gap: 0.5rem;
      }
      .check {
        display: inline-flex;
        gap: 0.4rem;
        align-items: center;
        font-size: 0.85rem;
      }
    `,
  ],
})
export class StartTeamSheetComponent {
  private api = inject(ApiService);
  protected readonly auth = inject(AuthService);
  protected readonly accounts = inject(AccountsService);

  readonly open = input(false);
  readonly templates = input<TemplateEntry[]>([]);
  readonly closed = output<void>();
  readonly started = output<TeamView>();

  protected readonly efforts = EFFORTS;
  protected readonly modes = MODES;
  protected readonly projects = signal<ProjectInfo[]>([]);
  protected readonly project = signal('');
  protected readonly templateId = signal('software-delivery');
  protected readonly account = signal('');
  protected readonly brief = signal('');
  protected readonly overrides = signal<Record<string, RoleOverride>>({});
  protected readonly busy = signal(false);
  protected readonly error = signal('');

  protected readonly template = computed(
    () => this.templates().find((t) => t.id === this.templateId()) ?? this.templates()[0],
  );
  protected readonly dangerous = computed(() => {
    const t = this.template();
    return !!t && needsDangerous(applyOverrides(t.roles, this.overrides()));
  });
  protected readonly canStart = computed(
    () =>
      !this.busy() &&
      !!this.project() &&
      !!this.template() &&
      !(this.dangerous() && !this.auth.canDangerous()),
  );

  constructor() {
    effect(() => {
      if (!this.open()) return;
      this.error.set('');
      void this.loadProjects();
    });
  }

  private async loadProjects(): Promise<void> {
    try {
      const { projects } = await this.api.getProjects();
      this.projects.set(
        [...projects].sort((a, b) => (b.lastActivityAt ?? 0) - (a.lastActivityAt ?? 0)),
      );
    } catch (e) {
      this.error.set(`Could not load the projects: ${describeError(e).text}`);
    }
  }

  protected set(role: string, key: 'model' | 'effort' | 'permissionMode', value: string): void {
    this.overrides.update((o) => {
      const next: RoleOverride = { ...(o[role] ?? {}) };
      const v = value.trim();
      if (v) {
        next[key] = v;
      } else {
        delete next[key];
      }
      return { ...o, [role]: next };
    });
  }

  protected setAutostart(role: string, value: boolean, template: boolean): void {
    this.overrides.update((o) => {
      const next: RoleOverride = { ...(o[role] ?? {}) };
      if (value === template) {
        delete next.autostart;
      } else {
        next.autostart = value;
      }
      return { ...o, [role]: next };
    });
  }

  protected async start(): Promise<void> {
    const t = this.template();
    if (!this.canStart() || !t) return;
    this.busy.set(true);
    this.error.set('');
    const overrides = Object.fromEntries(
      Object.entries(this.overrides()).filter(([, o]) => Object.keys(o).length > 0),
    );
    try {
      const team = await this.api.startTeam({
        project: this.project(),
        template: t.id,
        ...(this.account() ? { account: this.account() } : {}),
        ...(Object.keys(overrides).length ? { overrides } : {}),
        ...(this.brief().trim() ? { brief: this.brief().trim() } : {}),
      });
      this.brief.set('');
      this.overrides.set({});
      this.started.emit(team);
    } catch (e) {
      this.error.set(describeError(e).text);
    } finally {
      this.busy.set(false);
    }
  }
}
