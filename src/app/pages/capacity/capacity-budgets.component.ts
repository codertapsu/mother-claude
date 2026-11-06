import { Component, computed, inject, input, output, signal } from '@angular/core';
import { FormsModule } from '@angular/forms';

import { ApiError, ApiService, errorMessage } from '../../core/api.service';
import {
  BudgetLevel,
  BudgetState,
  CapacityView,
  SettingsPatch,
  budgetTone,
  parseUsd,
  pctLabel,
} from '../../core/capacity';
import { RealtimeService } from '../../core/realtime.service';
import { IconComponent } from '../../shared/icon.component';
import { formatUsd } from '../../shared/util';

interface Row {
  key: string;
  usd: string;
}

/** Why a form value cannot be sent, and its field. */
export interface Problem {
  field: string;
  message: string;
}

export function isProblem(v: unknown): v is Problem {
  return typeof v === 'object' && v !== null && 'message' in v && 'field' in v;
}

/** A dollar field: `null` when empty (no budget). */
export function readAmount(text: string, field: string): number | null | Problem {
  const v = parseUsd(text);
  return v === undefined ? { field, message: `${text.trim()} is not an amount in dollars.` } : v;
}

/** Name → dollars rows; empty rows are skipped. */
export function readMap(
  rows: Row[],
  field: string,
  what: string,
): Record<string, number> | Problem {
  const out: Record<string, number> = {};
  for (const r of rows.filter((row) => row.key.trim() || row.usd.trim())) {
    if (!r.key.trim()) return { field, message: `Name the ${what} of each budget.` };
    const v = parseUsd(r.usd);
    if (v === undefined || v === null) {
      return { field, message: `Give ${r.key.trim()} an amount in dollars.` };
    }
    out[r.key.trim()] = v;
  }
  return out;
}

/** Budgets with what is spent, and their editor (the desktop or a phone). */
@Component({
  selector: 'mc-capacity-budgets',
  imports: [FormsModule, IconComponent],
  template: `
    <p class="lede">
      Estimated dollars: on plan accounts (Pro, Max, Team) the work is paid in usage windows, so
      these are what it would cost through the API. Alerts at
      {{ pct(view().settings.budgets.nearPct) }}
      and at 100 %, as a notice on the desktop and by push.
    </p>

    @for (group of groups(); track group.title) {
      <h2>{{ group.title }}</h2>
      @if (group.states.length) {
        <div class="card list">
          @for (s of group.states; track s.scope + s.key) {
            <div class="budget" [attr.data-tone]="tone(s.level)">
              <div class="row">
                <span class="grow">{{ s.label }}</span>
                <span class="amount">{{ usd(s.spentUsd) }} of {{ usd(s.limitUsd) }}</span>
              </div>
              <span
                class="bar"
                role="meter"
                aria-valuemin="0"
                aria-valuemax="100"
                [attr.aria-label]="s.label"
                [attr.aria-valuenow]="clamp(s.pct)"
                ><span class="fill" [style.width.%]="clamp(s.pct)"></span
              ></span>
              <span class="muted small">
                {{ pct(s.pct) }}{{ levelText(s.scope, s.level) }}
                @if (s.note) {
                  · {{ s.note }}
                }
              </span>
            </div>
          }
        </div>
      } @else {
        <p class="muted">{{ group.empty }}</p>
      }
    }

    <h2>Change budgets</h2>
    @if (!view().canManage) {
      <p class="muted">Budgets are changed on the desktop app or a paired phone.</p>
    } @else if (!editing()) {
      <div class="card kv-card">
        <dl class="kv">
          <dt>Every project, per day</dt>
          <dd>{{ money(view().settings.budgets.projectDailyUsd) }}</dd>
          <dt>A role session</dt>
          <dd>
            {{ money(view().settings.budgets.sessionUsd) }}
            <span class="muted small">(--max-budget-usd on accounts that pay per token)</span>
          </dd>
          <dt>A task</dt>
          <dd>{{ money(view().settings.budgets.taskUsd) }}</dd>
          <dt>Alerts</dt>
          <dd>
            at {{ pct(view().settings.budgets.nearPct) }} and 100 % ·
            {{ alertsText() }}
          </dd>
        </dl>
        <button class="btn small" type="button" (click)="edit()">
          <mc-icon name="edit" [size]="14" /> Edit budgets
        </button>
      </div>
    } @else {
      <form class="card stack editor" (ngSubmit)="save()">
        <label class="field">
          <span class="field-label">Every project, per day (USD)</span>
          <input
            name="projectDaily"
            inputmode="decimal"
            placeholder="No budget"
            [class.invalid]="errorField() === 'budgets.projectDailyUsd'"
            [ngModel]="projectDaily()"
            (ngModelChange)="projectDaily.set($event)"
          />
          <span class="field-hint"
            >What every session in a project's folder spent since midnight.</span
          >
        </label>

        <fieldset class="rows">
          <legend class="field-label">A project's own daily budget</legend>
          @for (r of projects(); track $index; let i = $index) {
            <div class="row">
              <input
                class="grow"
                [name]="'project' + i"
                aria-label="Project folder"
                list="mc-known-projects"
                placeholder="/Users/me/dev/app"
                [ngModel]="r.key"
                (ngModelChange)="setProject(i, 'key', $event)"
              />
              <input
                class="usd"
                [name]="'projectUsd' + i"
                aria-label="Daily budget in USD"
                inputmode="decimal"
                placeholder="20"
                [ngModel]="r.usd"
                (ngModelChange)="setProject(i, 'usd', $event)"
              />
              <button
                class="btn ghost icon-only small"
                type="button"
                aria-label="Remove this budget"
                (click)="removeProject(i)"
              >
                <mc-icon name="x" [size]="14" />
              </button>
            </div>
          }
          <datalist id="mc-known-projects">
            @for (p of knownProjects(); track p) {
              <option [value]="p"></option>
            }
          </datalist>
          <button class="btn small" type="button" (click)="addProject()">
            <mc-icon name="plus" [size]="14" /> Add a project
          </button>
        </fieldset>

        <label class="field">
          <span class="field-label">A role session (USD)</span>
          <input
            name="session"
            inputmode="decimal"
            placeholder="No budget"
            [ngModel]="session()"
            (ngModelChange)="session.set($event)"
          />
          <span class="field-hint"
            >Passed as --max-budget-usd to role sessions on accounts that pay per token (an API key,
            a cloud provider); Claude Code stops the session there.</span
          >
        </label>

        <fieldset class="rows">
          <legend class="field-label">A role's own session budget</legend>
          @for (r of roles(); track $index; let i = $index) {
            <div class="row">
              <input
                class="grow"
                [name]="'role' + i"
                aria-label="Role id"
                placeholder="frontend"
                [ngModel]="r.key"
                (ngModelChange)="setRole(i, 'key', $event)"
              />
              <input
                class="usd"
                [name]="'roleUsd' + i"
                aria-label="Session budget in USD"
                inputmode="decimal"
                placeholder="5"
                [ngModel]="r.usd"
                (ngModelChange)="setRole(i, 'usd', $event)"
              />
              <button
                class="btn ghost icon-only small"
                type="button"
                aria-label="Remove this budget"
                (click)="removeRole(i)"
              >
                <mc-icon name="x" [size]="14" />
              </button>
            </div>
          }
          <button class="btn small" type="button" (click)="addRole()">
            <mc-icon name="plus" [size]="14" /> Add a role
          </button>
        </fieldset>

        <label class="field">
          <span class="field-label">A task (USD)</span>
          <input
            name="task"
            inputmode="decimal"
            placeholder="No budget"
            [ngModel]="task()"
            (ngModelChange)="task.set($event)"
          />
        </label>

        <label class="field">
          <span class="field-label">Alert at (%)</span>
          <input
            name="near"
            type="number"
            min="1"
            max="99"
            [ngModel]="near()"
            (ngModelChange)="near.set($event)"
          />
          <span class="field-hint"
            >Also where Opus work starts running on Sonnet for a project near its budget.</span
          >
        </label>

        <label class="switch">
          <input
            type="checkbox"
            name="desktop"
            [ngModel]="desktopAlerts()"
            (ngModelChange)="desktopAlerts.set($event)"
          />
          <span class="switch-text">Alert on the desktop</span>
        </label>
        <label class="switch">
          <input
            type="checkbox"
            name="push"
            [ngModel]="pushAlerts()"
            (ngModelChange)="pushAlerts.set($event)"
          />
          <span class="switch-text">Alert by push notification</span>
        </label>

        @if (error()) {
          <p class="notice error" role="alert">{{ error() }}</p>
        }
        <div class="row">
          <button class="btn primary" type="submit" [disabled]="saving()">Save budgets</button>
          <button class="btn" type="button" [disabled]="saving()" (click)="editing.set(false)">
            Cancel
          </button>
        </div>
      </form>
    }
  `,
  styles: [
    `
      .budget {
        display: flex;
        flex-direction: column;
        gap: 0.3rem;
        padding: 0.65rem 0;
      }
      .grow {
        flex: 1 1 auto;
        min-width: 0;
        overflow-wrap: anywhere;
      }
      .amount {
        font-variant-numeric: tabular-nums;
        color: var(--tone);
        font-weight: 600;
      }
      .bar {
        display: block;
        height: 0.45rem;
        border-radius: 0.25rem;
        background: var(--panel-3);
        overflow: hidden;
      }
      .fill {
        display: block;
        height: 100%;
        border-radius: inherit;
        background: var(--tone);
      }
      .list {
        padding-top: 0;
        padding-bottom: 0;
      }
      .kv-card {
        display: flex;
        flex-direction: column;
        gap: 0.75rem;
        align-items: flex-start;
      }
      .rows {
        border: 0;
        padding: 0;
        margin: 0;
        display: flex;
        flex-direction: column;
        gap: 0.5rem;
      }
      .rows .row {
        flex-wrap: nowrap;
      }
      .usd {
        width: 6.5rem;
        flex: 0 0 auto;
      }
      .editor {
        gap: 1rem;
      }
      .invalid {
        border-color: var(--red);
      }
      h2 {
        margin-top: 1.5rem;
      }
    `,
  ],
})
export class CapacityBudgetsComponent {
  private api = inject(ApiService);
  private realtime = inject(RealtimeService);

  readonly view = input.required<CapacityView>();
  readonly changed = output<CapacityView>();

  protected readonly editing = signal(false);
  protected readonly saving = signal(false);
  protected readonly error = signal('');
  protected readonly errorField = signal('');
  protected readonly projectDaily = signal('');
  protected readonly session = signal('');
  protected readonly task = signal('');
  protected readonly near = signal<string | number>('80');
  protected readonly desktopAlerts = signal(true);
  protected readonly pushAlerts = signal(true);
  protected readonly projects = signal<Row[]>([]);
  protected readonly roles = signal<Row[]>([]);

  protected readonly groups = computed(() => {
    const all = this.view().budgets;
    const of = (scope: BudgetState['scope']) => all.filter((s) => s.scope === scope);
    return [
      {
        title: 'Projects today',
        states: of('project'),
        empty: 'No project has a daily budget.',
      },
      {
        title: 'Role sessions',
        states: of('session'),
        empty: 'No role session runs with a budget.',
      },
      { title: 'Tasks', states: of('task'), empty: 'No running task has a budget.' },
    ];
  });
  /** Folders the dashboard knows, for the project field's suggestions. */
  protected readonly knownProjects = computed(() =>
    [...new Set(this.realtime.sessions().map((s) => s.cwd))].filter(Boolean).sort(),
  );
  protected readonly alertsText = computed(() => {
    const a = this.view().settings.alerts;
    if (a.desktop && a.push) return 'desktop and push';
    if (a.desktop) return 'desktop only';
    if (a.push) return 'push only';
    return 'off';
  });

  protected readonly usd = formatUsd;
  protected readonly pct = pctLabel;

  protected tone(level: BudgetLevel): string {
    return budgetTone(level);
  }

  protected clamp(p: number): number {
    return Math.max(0, Math.min(100, p));
  }

  protected money(v: number | null): string {
    return v === null ? 'no budget' : formatUsd(v);
  }

  protected levelText(scope: BudgetState['scope'], level: BudgetLevel): string {
    if (level === 'over') {
      return scope === 'project' ? ' · new workers wait until tomorrow' : ' · reached';
    }
    if (level === 'near') return scope === 'project' ? ' · Opus work runs on Sonnet' : ' · near';
    return '';
  }

  protected edit(): void {
    const b = this.view().settings.budgets;
    const a = this.view().settings.alerts;
    const str = (v: number | null) => (v === null ? '' : String(v));
    this.projectDaily.set(str(b.projectDailyUsd));
    this.session.set(str(b.sessionUsd));
    this.task.set(str(b.taskUsd));
    this.near.set(b.nearPct);
    this.desktopAlerts.set(a.desktop);
    this.pushAlerts.set(a.push);
    this.projects.set(Object.entries(b.projects).map(([key, v]) => ({ key, usd: String(v) })));
    this.roles.set(Object.entries(b.roles).map(([key, v]) => ({ key, usd: String(v) })));
    this.error.set('');
    this.errorField.set('');
    this.editing.set(true);
  }

  protected setProject(i: number, field: keyof Row, value: string): void {
    this.projects.update((rows) => rows.map((r, j) => (j === i ? { ...r, [field]: value } : r)));
  }

  protected setRole(i: number, field: keyof Row, value: string): void {
    this.roles.update((rows) => rows.map((r, j) => (j === i ? { ...r, [field]: value } : r)));
  }

  protected addProject(): void {
    this.projects.update((rows) => [...rows, { key: '', usd: '' }]);
  }

  protected removeProject(i: number): void {
    this.projects.update((rows) => rows.filter((_, j) => j !== i));
  }

  protected addRole(): void {
    this.roles.update((rows) => [...rows, { key: '', usd: '' }]);
  }

  protected removeRole(i: number): void {
    this.roles.update((rows) => rows.filter((_, j) => j !== i));
  }

  /** The update the form describes, or why not. */
  protected patch(): SettingsPatch | Problem {
    const daily = readAmount(this.projectDaily(), 'budgets.projectDailyUsd');
    if (isProblem(daily)) return daily;
    const session = readAmount(this.session(), 'budgets.sessionUsd');
    if (isProblem(session)) return session;
    const task = readAmount(this.task(), 'budgets.taskUsd');
    if (isProblem(task)) return task;
    const projects = readMap(this.projects(), 'budgets.projects', 'project');
    if (isProblem(projects)) return projects;
    const roles = readMap(this.roles(), 'budgets.roles', 'role');
    if (isProblem(roles)) return roles;
    const near = Number(this.near());
    if (!Number.isFinite(near) || near < 1 || near > 99) {
      return { field: 'budgets.nearPct', message: 'Alert between 1 and 99 %.' };
    }
    return {
      budgets: {
        projectDailyUsd: daily,
        projects,
        sessionUsd: session,
        roles,
        taskUsd: task,
        nearPct: near,
      },
      alerts: { desktop: this.desktopAlerts(), push: this.pushAlerts() },
    };
  }

  async save(): Promise<void> {
    const p = this.patch();
    if (isProblem(p)) {
      this.error.set(p.message);
      this.errorField.set(p.field);
      return;
    }
    this.saving.set(true);
    try {
      const v = await this.api.setCapacitySettings(p);
      this.changed.emit(v);
      this.editing.set(false);
      this.error.set('');
      this.errorField.set('');
    } catch (e) {
      this.error.set(errorMessage(e));
      this.errorField.set(e instanceof ApiError ? (e.field ?? '') : '');
    } finally {
      this.saving.set(false);
    }
  }
}
