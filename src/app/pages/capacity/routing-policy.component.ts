import { Component, computed, input, inject, signal } from '@angular/core';
import { FormsModule } from '@angular/forms';

import { ApiError, ApiService, errorMessage } from '../../core/api.service';
import {
  ClassView,
  EFFORTS,
  MODELS,
  ModelChoice,
  PolicyView,
  RouteNote,
  RoutingPolicy,
  choiceLabel,
  effortIdLabel,
  policyProblems,
  projectName,
  taskClassLabel,
  tier,
} from '../../core/capacity';
import { tr } from '../../core/i18n/i18n';
import { TPipe } from '../../core/i18n/t.pipe';
import { IconComponent } from '../../shared/icon.component';
import { TimeAgoComponent } from '../../shared/time-ago.component';

/** The routing policy: read by everyone, edited and reset on the desktop. */
@Component({
  selector: 'mc-routing-policy',
  imports: [FormsModule, IconComponent, TimeAgoComponent, TPipe],
  template: `
    <p class="lede">
      {{
        "In order: your override on a task or a role; escalation after failed gates; the budget gate (Opus work on Sonnet at high effort while the Opus weekly window or the project's budget is high); the task's class below; the role's default."
          | t
      }}
    </p>
    @if (loadError()) {
      <p class="notice error" role="alert">{{ loadError() }}</p>
    }
    @if (data(); as d) {
      @if (draft(); as p) {
        <form class="card stack editor" (ngSubmit)="save()">
          <h2>{{ 'Task classes' | t }}</h2>
          <div class="classes">
            @for (c of d.classes; track c.class) {
              <div class="class-row">
                <span class="name">{{ classLabel(c.label) }}</span>
                <select
                  [name]="'model-' + c.class"
                  [attr.aria-label]="'Model for {class} tasks' | t: { class: classLabel(c.label) }"
                  [ngModel]="classChoice(p, c.class)?.model ?? ''"
                  (ngModelChange)="setClass(c.class, 'model', $event)"
                >
                  <option value="">{{ "The role's default" | t }}</option>
                  @for (m of modelOptions(classChoice(p, c.class)?.model); track m) {
                    <option [value]="m">{{ m }}</option>
                  }
                </select>
                <select
                  [name]="'effort-' + c.class"
                  [attr.aria-label]="'Effort for {class} tasks' | t: { class: classLabel(c.label) }"
                  [disabled]="!classChoice(p, c.class)"
                  [ngModel]="classChoice(p, c.class)?.effort ?? ''"
                  (ngModelChange)="setClass(c.class, 'effort', $event)"
                >
                  <option value="">{{ "Model's default" | t }}</option>
                  @for (e of efforts; track e) {
                    <option [value]="e">{{ effortId(e) }}</option>
                  }
                </select>
                @if (problems()['classes.' + c.class]; as why) {
                  <span class="error-text small full">{{ why }}</span>
                }
              </div>
            }
          </div>

          <h2>{{ 'Escalation ladder' | t }}</h2>
          <p class="muted small">
            {{ 'Lowest rung first; each rung above the one before it.' | t }}
          </p>
          @for (r of p.ladder; track $index; let i = $index) {
            <div class="row rung">
              <span class="faint small">{{ i + 1 }}</span>
              <select
                [name]="'rung-model-' + i"
                [attr.aria-label]="'Rung {n} model' | t: { n: i + 1 }"
                [ngModel]="r.model"
                (ngModelChange)="setRung(i, 'model', $event)"
              >
                @for (m of modelOptions(r.model); track m) {
                  <option [value]="m">{{ m }}</option>
                }
              </select>
              <select
                [name]="'rung-effort-' + i"
                [attr.aria-label]="'Rung {n} effort' | t: { n: i + 1 }"
                [ngModel]="r.effort ?? ''"
                (ngModelChange)="setRung(i, 'effort', $event)"
              >
                <option value="">{{ "Model's default" | t }}</option>
                @for (e of efforts; track e) {
                  <option [value]="e">{{ effortId(e) }}</option>
                }
              </select>
              <button
                class="btn ghost icon-only small"
                type="button"
                [attr.aria-label]="'Remove rung {n}' | t: { n: i + 1 }"
                (click)="removeRung(i)"
              >
                <mc-icon name="x" [size]="14" />
              </button>
            </div>
          }
          <div class="row">
            <button class="btn small" type="button" (click)="addRung()">
              <mc-icon name="plus" [size]="14" /> {{ 'Add a rung' | t }}
            </button>
          </div>
          @if (problems()['ladder']; as why) {
            <p class="error-text small">{{ why }}</p>
          }
          <label class="switch">
            <input
              type="checkbox"
              name="fable"
              [ngModel]="p.fableOnEscalation"
              (ngModelChange)="patch({ fableOnEscalation: $event })"
            />
            <span class="switch-text"
              >{{ 'Escalation may climb onto Fable rungs' | t }}
              <span class="muted small">{{
                'Off: Fable runs only when you choose it.' | t
              }}</span></span
            >
          </label>
          <div class="pair">
            <label class="field">
              <span class="field-label">{{ 'Failed gates per step' | t }}</span>
              <input
                name="gates"
                type="number"
                min="1"
                max="5"
                [ngModel]="p.gatesPerStep"
                (ngModelChange)="patch({ gatesPerStep: num($event) })"
              />
              @if (problems()['gatesPerStep']; as why) {
                <span class="error-text small">{{ why }}</span>
              }
            </label>
            <label class="field">
              <span class="field-label">{{ 'Clean completions to step down' | t }}</span>
              <input
                name="clean"
                type="number"
                min="1"
                max="10"
                [ngModel]="p.cleanToStepDown"
                (ngModelChange)="patch({ cleanToStepDown: num($event) })"
              />
              @if (problems()['cleanToStepDown']; as why) {
                <span class="error-text small">{{ why }}</span>
              }
            </label>
          </div>

          <h2>{{ 'Budget gate' | t }}</h2>
          <div class="row">
            <span class="small">{{ 'Opus work runs on' | t }}</span>
            <select
              name="gateModel"
              [attr.aria-label]="'Budget gate model' | t"
              [ngModel]="p.gateChoice.model"
              (ngModelChange)="
                patch({ gateChoice: { model: $event, effort: p.gateChoice.effort } })
              "
            >
              @for (m of modelOptions(p.gateChoice.model); track m) {
                <option [value]="m">{{ m }}</option>
              }
            </select>
            <select
              name="gateEffort"
              [attr.aria-label]="'Budget gate effort' | t"
              [ngModel]="p.gateChoice.effort ?? ''"
              (ngModelChange)="patch({ gateChoice: choiceOf(p.gateChoice.model, $event) })"
            >
              <option value="">{{ "Model's default" | t }}</option>
              @for (e of efforts; track e) {
                <option [value]="e">{{ effortId(e) }}</option>
              }
            </select>
          </div>
          @if (problems()['gateChoice']; as why) {
            <p class="error-text small">{{ why }}</p>
          }

          <h2>{{ 'Fallback models' | t }}</h2>
          <p class="muted small">
            {{
              '--fallback-model for each family when its model is overloaded (up to three, comma-separated).'
                | t
            }}
          </p>
          @for (f of families; track f) {
            <label class="field">
              <span class="field-label">{{ f }}</span>
              <input
                [name]="'fallback-' + f"
                [placeholder]="'None' | t"
                [ngModel]="p.fallbacks[f] ?? ''"
                (ngModelChange)="setFallback(f, $event)"
              />
            </label>
          }

          @if (saveError()) {
            <p class="notice error" role="alert">{{ saveError() }}</p>
          }
          <div class="row">
            <button class="btn primary" type="submit" [disabled]="saving() || invalid()">
              {{ 'Save the policy' | t }}
            </button>
            <button class="btn" type="button" [disabled]="saving()" (click)="draft.set(null)">
              {{ 'Cancel' | t }}
            </button>
          </div>
        </form>
      } @else {
        <h2>{{ 'Task classes' | t }}</h2>
        <div class="card list">
          @for (c of d.classes; track c.class) {
            <div class="list-row">
              <span class="grow">{{ classLabel(c.label) }}</span>
              <span [class.muted]="!classChoice(d.policy, c.class)">{{
                classText(d.policy, c.class)
              }}</span>
            </div>
          }
        </div>

        <h2>{{ 'Escalation' | t }}</h2>
        <div class="card stack explain">
          <p class="ladder">
            @for (r of d.policy.ladder; track $index; let last = $last) {
              <span class="badge" [class.faint]="isFable(r) && !d.policy.fableOnEscalation">{{
                choice(r)
              }}</span>
              @if (!last) {
                <span class="faint" aria-hidden="true">→</span>
              }
            }
          </p>
          <p class="small">
            {{
              'Every {gates} failed gates of a task climb one rung (effort before model); a task class that escalated steps back down after {clean} clean completions.'
                | t: { gates: d.policy.gatesPerStep, clean: d.policy.cleanToStepDown }
            }}
            {{
              d.policy.fableOnEscalation
                ? ('Escalation may reach Fable.' | t)
                : ('Fable runs only when you choose it.' | t)
            }}
          </p>
          <p class="small">
            {{
              'Budget gate: {choice}. Fallbacks: {fallbacks}.'
                | t: { choice: choice(d.policy.gateChoice), fallbacks: fallbackText(d.policy) }
            }}
          </p>
        </div>

        @if (d.canEdit) {
          <div class="row actions">
            <button class="btn" type="button" (click)="edit()">
              <mc-icon name="edit" [size]="14" /> {{ 'Edit the policy' | t }}
            </button>
            @if (!d.isDefault) {
              @if (confirmReset()) {
                <span class="small">{{ 'Back to the built-in policy?' | t }}</span>
                <button
                  class="btn small danger"
                  type="button"
                  [disabled]="saving()"
                  (click)="reset()"
                >
                  {{ 'Reset' | t }}
                </button>
                <button class="btn small" type="button" (click)="confirmReset.set(false)">
                  {{ 'Keep' | t }}
                </button>
              } @else {
                <button class="btn ghost" type="button" (click)="confirmReset.set(true)">
                  {{ 'Reset to defaults' | t }}
                </button>
              }
            }
          </div>
        } @else {
          <p class="muted small">{{ 'The policy is edited on the desktop app.' | t }}</p>
        }
        @if (saveError()) {
          <p class="notice error" role="alert">{{ saveError() }}</p>
        }

        <h2>{{ 'Escalated now' | t }}</h2>
        @if (d.learned.length) {
          <ul class="card list plain">
            @for (l of d.learned; track l.project + l.class) {
              <li class="list-row small">
                <span class="grow">{{ learnedText(l) }}</span>
                <span class="muted">{{
                  '{n} of {total} clean completions'
                    | t: { n: l.clean, total: d.policy.cleanToStepDown }
                }}</span>
              </li>
            }
          </ul>
        } @else {
          <p class="muted">{{ 'No task class is escalated.' | t }}</p>
        }

        @if (d.escalations.length) {
          <h2>{{ 'Escalations' | t }}</h2>
          <ul class="card list plain">
            @for (n of d.escalations.slice(0, 10); track n.at + n.taskId) {
              <li class="list-row small">
                <span class="grow"
                  >{{ n.direction === 'up' ? '↑' : '↓' }}
                  {{
                    '{task} in {project}: {reason}'
                      | t: { task: n.taskId, project: project(n.project), reason: n.reason }
                  }}</span
                >
                <span class="muted"><mc-time [ms]="n.at" /></span>
              </li>
            }
          </ul>
        }
        @if (routes().length) {
          <h2>{{ 'Recent decisions' | t }}</h2>
          <ul class="card list plain">
            @for (r of routes().slice(0, 10); track r.at + r.project + (r.role ?? '')) {
              <li class="list-row small">
                <span class="grow"
                  ><strong>{{ choice(r.choice) }}</strong> {{ routeFor(r) }} · {{ r.reason }}</span
                >
                <span class="muted"><mc-time [ms]="r.at" /></span>
              </li>
            }
          </ul>
        }
      }
    } @else if (!loadError()) {
      <p class="muted">{{ 'Loading…' | t }}</p>
    }
  `,
  styles: [
    `
      .classes {
        display: flex;
        flex-direction: column;
        gap: 0.5rem;
      }
      .class-row {
        display: grid;
        grid-template-columns: minmax(7rem, 1fr) minmax(8rem, 1.2fr) minmax(8rem, 1fr);
        gap: 0.5rem;
        align-items: center;
      }
      .class-row .full {
        grid-column: 1 / -1;
      }
      @media (max-width: 520px) {
        .class-row {
          grid-template-columns: 1fr 1fr;
        }
        .class-row .name {
          grid-column: 1 / -1;
        }
      }
      .rung {
        flex-wrap: nowrap;
      }
      .rung select {
        min-width: 0;
      }
      .pair {
        display: grid;
        grid-template-columns: repeat(auto-fit, minmax(11rem, 1fr));
        gap: 0.75rem;
      }
      .editor {
        gap: 0.75rem;
      }
      .editor h2 {
        margin: 0.75rem 0 0;
        font-size: 1rem;
      }
      .list {
        padding-top: 0;
        padding-bottom: 0;
      }
      .plain {
        list-style: none;
        margin: 0;
      }
      .grow {
        flex: 1 1 auto;
        min-width: 0;
        overflow-wrap: anywhere;
      }
      .explain p {
        margin: 0;
      }
      .ladder {
        display: flex;
        flex-wrap: wrap;
        gap: 0.35rem;
        align-items: center;
        margin: 0;
      }
      .actions {
        margin-top: 0.75rem;
      }
      h2 {
        margin-top: 1.5rem;
      }
    `,
  ],
})
export class RoutingPolicyComponent {
  private api = inject(ApiService);
  readonly routes = input<RouteNote[]>([]);

  protected readonly data = signal<PolicyView | null>(null);
  protected readonly loadError = signal('');
  protected readonly draft = signal<RoutingPolicy | null>(null);
  protected readonly saving = signal(false);
  protected readonly saveError = signal('');
  protected readonly confirmReset = signal(false);
  protected readonly problems = computed(() => {
    const d = this.draft();
    return d ? policyProblems(d) : {};
  });
  protected readonly invalid = computed(() => Object.keys(this.problems()).length > 0);

  protected readonly efforts = EFFORTS;
  protected readonly families = MODELS;
  protected readonly choice = choiceLabel;
  protected readonly classLabel = taskClassLabel;
  protected readonly effortId = effortIdLabel;
  protected readonly project = projectName;

  constructor() {
    void this.load();
  }

  async load(): Promise<void> {
    try {
      this.data.set(await this.api.getRoutingPolicy());
      this.loadError.set('');
    } catch (e) {
      this.loadError.set(
        tr('Could not load the routing policy: {error}', { error: errorMessage(e) }),
      );
    }
  }

  /** The aliases, plus a full id already in use (kept, not replaced). */
  protected modelOptions(current: string | undefined): string[] {
    return current && !(MODELS as readonly string[]).includes(current)
      ? [...MODELS, current]
      : [...MODELS];
  }

  /** A class's entry (absent: the role's default). */
  protected classChoice(p: RoutingPolicy, cls: string): ModelChoice | undefined {
    return Object.prototype.hasOwnProperty.call(p.classes, cls) ? p.classes[cls] : undefined;
  }

  protected classText(p: RoutingPolicy, cls: string): string {
    const c = this.classChoice(p, cls);
    return c ? choiceLabel(c) : tr("the role's default");
  }

  protected num(v: unknown): number {
    return Number(v);
  }

  protected isFable(c: ModelChoice): boolean {
    return tier(c.model) >= 3;
  }

  protected choiceOf(model: string, effort: string): ModelChoice {
    return effort ? { model, effort } : { model };
  }

  protected fallbackText(p: RoutingPolicy): string {
    const entries = Object.entries(p.fallbacks);
    return entries.length ? entries.map(([k, v]) => `${k} → ${v}`).join(', ') : tr('none');
  }

  /** "implement tasks in app: 1 step up". */
  protected learnedText(l: ClassView): string {
    const params = { class: l.class, project: projectName(l.project), n: l.level };
    return l.level === 1
      ? tr('{class} tasks in {project}: {n} step up', params)
      : tr('{class} tasks in {project}: {n} steps up', params);
  }

  /** "for reviewer in app", "for a task in app". */
  protected routeFor(r: RouteNote): string {
    const project = projectName(r.project);
    return r.role
      ? tr('for {role} in {project}', { role: r.role, project })
      : tr('for a task in {project}', { project });
  }

  protected edit(): void {
    const d = this.data();
    if (!d) return;
    this.draft.set(structuredClone(d.policy));
    this.saveError.set('');
  }

  protected patch(change: Partial<RoutingPolicy>): void {
    this.draft.update((p) => (p ? { ...p, ...change } : p));
  }

  protected setClass(cls: string, field: 'model' | 'effort', value: string): void {
    this.draft.update((p) => {
      if (!p) return p;
      const classes = { ...p.classes };
      if (field === 'model') {
        if (!value) delete classes[cls];
        else classes[cls] = this.choiceOf(value, classes[cls]?.effort ?? '');
      } else if (classes[cls]) {
        classes[cls] = this.choiceOf(classes[cls].model, value);
      }
      return { ...p, classes };
    });
  }

  protected setRung(i: number, field: 'model' | 'effort', value: string): void {
    this.draft.update((p) => {
      if (!p) return p;
      const ladder = p.ladder.map((r, j) =>
        j !== i
          ? r
          : field === 'model'
            ? this.choiceOf(value, r.effort ?? '')
            : this.choiceOf(r.model, value),
      );
      return { ...p, ladder };
    });
  }

  protected addRung(): void {
    this.draft.update((p) =>
      p ? { ...p, ladder: [...p.ladder, { model: 'opus', effort: 'max' }] } : p,
    );
  }

  protected removeRung(i: number): void {
    this.draft.update((p) => (p ? { ...p, ladder: p.ladder.filter((_, j) => j !== i) } : p));
  }

  protected setFallback(family: string, value: string): void {
    this.draft.update((p) => {
      if (!p) return p;
      const fallbacks = { ...p.fallbacks };
      const v = value.replace(/\s+/g, '');
      if (v) fallbacks[family] = v;
      else delete fallbacks[family];
      return { ...p, fallbacks };
    });
  }

  async save(): Promise<void> {
    const p = this.draft();
    if (!p || this.invalid()) return;
    this.saving.set(true);
    try {
      this.data.set(await this.api.setRoutingPolicy(p));
      this.draft.set(null);
      this.saveError.set('');
    } catch (e) {
      const field = e instanceof ApiError && e.field ? ` (${e.field})` : '';
      this.saveError.set(`${errorMessage(e)}${field}`);
    } finally {
      this.saving.set(false);
    }
  }

  async reset(): Promise<void> {
    this.saving.set(true);
    try {
      this.data.set(await this.api.resetRoutingPolicy());
      this.confirmReset.set(false);
      this.saveError.set('');
    } catch (e) {
      this.saveError.set(errorMessage(e));
    } finally {
      this.saving.set(false);
    }
  }
}
