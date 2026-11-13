import { ChangeDetectionStrategy, Component, computed, inject, input, signal } from '@angular/core';
import { FormsModule } from '@angular/forms';
import { RouterLink } from '@angular/router';

import { ApiService, errorMessage } from '../../core/api.service';
import { CapacityView, SettingsPatch } from '../../core/capacity';
import { tr } from '../../core/i18n/i18n';
import { TPipe } from '../../core/i18n/t.pipe';
import { samePath, trimSeparators } from '../../core/paths';
import { TeamView } from '../../core/teams';
import { formatUsd } from '../../shared/util';
import { isProblem, readAmount } from '../capacity/capacity-budgets.component';
import { readInt } from '../capacity/capacity-settings.component';

/** The key a map names a project by (its folder as written there). */
function keyOf(map: Record<string, unknown>, cwd: string): string | null {
  return Object.keys(map).find((k) => samePath(k, cwd)) ?? null;
}

/**
 * A project's team limits (its settings sheet, docs/UI-REDESIGN.md §9): how
 * many role sessions its teams run at once and what every session in its
 * folder may spend a day — its own, or every project's — with what it spent
 * today, and its teams. The values are Capacity's (*Settings*' project
 * limits, *Budgets*' project budgets, docs/ROUTING.md), changed here for
 * this project alone by the desktop or a paired phone.
 */
@Component({
  selector: 'mc-project-limits',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [FormsModule, RouterLink, TPipe],
  template: `
    @if (view(); as v) {
      @if (!editing()) {
        <dl class="kv">
          <dt>{{ 'Role sessions at once' | t }}</dt>
          <dd>
            {{ cap(v) }}
            <span class="muted small">{{
              ownCap(v) === null ? ('(every project’s)' | t) : ('(its own)' | t)
            }}</span>
          </dd>
          <dt>{{ 'Daily budget' | t }}</dt>
          <dd>
            {{ budget(v) }}
            <span class="muted small">{{
              ownBudget(v) === null ? ('(every project’s)' | t) : ('(its own)' | t)
            }}</span>
            @if (spent(); as s) {
              <span class="muted small">· {{ '{amount} spent today' | t: { amount: s } }}</span>
            }
          </dd>
        </dl>
        @if (teams().length) {
          <p class="small teams">
            {{ 'Its teams:' | t }}
            @for (t of teams(); track t.id; let last = $last) {
              <a [routerLink]="['/teams', t.id]">{{ t.label }}</a
              >{{ last ? '' : ',' }}
            }
          </p>
        }
        @if (v.canManage) {
          <button class="btn small" type="button" (click)="edit(v)">
            {{ 'Change the limits' | t }}
          </button>
        } @else {
          <p class="muted small">
            {{ 'Budgets are changed on the desktop app or a paired phone.' | t }}
          </p>
        }
      } @else {
        <form class="stack" (ngSubmit)="save(v)">
          <label class="field">
            <span class="field-label">{{ 'Role sessions at once' | t }}</span>
            <input
              name="cap"
              inputmode="numeric"
              [placeholder]="'Every project’s: {value}' | t: { value: v.settings.projectCap }"
              [ngModel]="capText()"
              (ngModelChange)="capText.set($event)"
            />
          </label>
          <label class="field">
            <span class="field-label">{{ 'Daily budget (USD)' | t }}</span>
            <input
              name="usd"
              inputmode="decimal"
              [placeholder]="'Every project’s: {value}' | t: { value: everyBudget(v) }"
              [ngModel]="usdText()"
              (ngModelChange)="usdText.set($event)"
            />
            <span class="field-hint">{{
              'What every session in its folder may spend since midnight. Leave a field empty for every project’s.'
                | t
            }}</span>
          </label>
          @if (error()) {
            <p class="error-text small" role="alert">{{ error() }}</p>
          }
          <div class="row">
            <button class="btn primary small" type="submit" [disabled]="saving()">
              {{ saving() ? ('Saving…' | t) : ('Save' | t) }}
            </button>
            <button class="btn ghost small" type="button" (click)="editing.set(false)">
              {{ 'Cancel' | t }}
            </button>
          </div>
        </form>
      }
    } @else if (error()) {
      <p class="error-text small" role="alert">{{ error() }}</p>
    } @else {
      <p class="muted small" role="status">{{ 'Loading…' | t }}</p>
    }
  `,
  styles: [
    `
      :host {
        display: flex;
        flex-direction: column;
        gap: 0.5rem;
        align-items: flex-start;
      }
      p {
        margin: 0;
      }
      .kv,
      form {
        width: 100%;
      }
      .teams a + a {
        margin-left: 0.25rem;
      }
    `,
  ],
})
export class ProjectLimitsComponent {
  /** The project's folder. */
  readonly cwd = input.required<string>();

  private readonly api = inject(ApiService);
  protected readonly view = signal<CapacityView | null>(null);
  private readonly allTeams = signal<TeamView[]>([]);
  protected readonly editing = signal(false);
  protected readonly saving = signal(false);
  protected readonly error = signal('');
  protected readonly capText = signal('');
  protected readonly usdText = signal('');

  /** Its teams (none started yet: none). */
  protected readonly teams = computed(() =>
    this.allTeams().filter((t) => samePath(t.project, this.cwd())),
  );
  /** What it spent today, while it has a budget. */
  protected readonly spent = computed(() => {
    const b = this.view()?.budgets.find(
      (s) => s.scope === 'project' && !!s.project && samePath(s.project, this.cwd()),
    );
    return b ? formatUsd(b.spentUsd) : '';
  });

  constructor() {
    void this.load();
  }

  protected ownCap(v: CapacityView): number | null {
    const key = keyOf(v.settings.projectCaps, this.cwd());
    return key ? v.settings.projectCaps[key] : null;
  }

  protected ownBudget(v: CapacityView): number | null {
    const key = keyOf(v.settings.budgets.projects, this.cwd());
    return key ? v.settings.budgets.projects[key] : null;
  }

  protected cap(v: CapacityView): number {
    return this.ownCap(v) ?? v.settings.projectCap;
  }

  protected budget(v: CapacityView): string {
    const usd = this.ownBudget(v) ?? v.settings.budgets.projectDailyUsd;
    return usd === null ? tr('no budget') : formatUsd(usd);
  }

  protected everyBudget(v: CapacityView): string {
    const usd = v.settings.budgets.projectDailyUsd;
    return usd === null ? tr('no budget') : formatUsd(usd);
  }

  protected edit(v: CapacityView): void {
    const cap = this.ownCap(v);
    const usd = this.ownBudget(v);
    this.capText.set(cap === null ? '' : String(cap));
    this.usdText.set(usd === null ? '' : String(usd));
    this.error.set('');
    this.editing.set(true);
  }

  /** Its own limit and budget set, or dropped (empty), the other
   * projects' kept as they are. */
  protected async save(v: CapacityView): Promise<void> {
    const cap = this.capText().trim()
      ? readInt(
          this.capText(),
          1,
          16,
          'projectCaps',
          tr("A project's limit must be a whole number from {lo} to {hi}."),
        )
      : null;
    if (isProblem(cap)) {
      this.error.set(cap.message);
      return;
    }
    const usd = readAmount(this.usdText(), 'budgets.projects');
    if (isProblem(usd)) {
      this.error.set(usd.message);
      return;
    }
    const at = trimSeparators(this.cwd().trim());
    const projectCaps = { ...v.settings.projectCaps };
    const capKey = keyOf(projectCaps, at);
    if (capKey) delete projectCaps[capKey];
    if (cap !== null) projectCaps[at] = cap;
    const projects = { ...v.settings.budgets.projects };
    const budgetKey = keyOf(projects, at);
    if (budgetKey) delete projects[budgetKey];
    if (usd !== null) projects[at] = usd;
    const patch: SettingsPatch = { projectCaps, budgets: { projects } };
    this.saving.set(true);
    this.error.set('');
    try {
      this.view.set(await this.api.setCapacitySettings(patch));
      this.editing.set(false);
    } catch (e) {
      this.error.set(errorMessage(e));
    } finally {
      this.saving.set(false);
    }
  }

  private async load(): Promise<void> {
    try {
      this.view.set(await this.api.getCapacity());
    } catch (e) {
      this.error.set(errorMessage(e));
    }
    try {
      this.allTeams.set((await this.api.getTeams()).teams ?? []);
    } catch {
      // Only a convenience: Teams lists them.
    }
  }
}
