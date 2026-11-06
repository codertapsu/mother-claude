import { Component, computed, inject, input, output, signal } from '@angular/core';
import { FormsModule } from '@angular/forms';

import { ApiError, ApiService, errorMessage } from '../../core/api.service';
import { AuthService } from '../../core/auth.service';
import {
  Billing,
  CapacityView,
  Classification,
  SettingsPatch,
  pctLabel,
} from '../../core/capacity';
import { IconComponent } from '../../shared/icon.component';
import { formatTokens, formatUsd, shortPath } from '../../shared/util';
import { Problem, isProblem } from './capacity-budgets.component';

interface CapRow {
  key: string;
  cap: string;
}

/** A whole number in `lo..=hi`, or why not. */
export function readInt(
  v: string | number,
  lo: number,
  hi: number,
  field: string,
  what: string,
): number | Problem {
  const n = typeof v === 'number' ? v : Number(String(v).trim());
  return Number.isInteger(n) && n >= lo && n <= hi
    ? n
    : { field, message: `${what} must be a whole number from ${lo} to ${hi}.` };
}

/** A percentage in 1..=100, or why not. */
export function readPct(v: string | number, field: string, what: string): number | Problem {
  const n = typeof v === 'number' ? v : Number(String(v).trim());
  return Number.isFinite(n) && n >= 1 && n <= 100
    ? n
    : { field, message: `${what} must be between 1 and 100 %.` };
}

/** The scheduler's settings: caps, memory, plan-window thresholds, how
 * accounts pay and the optional Haiku classifier (the last two on the
 * desktop only). */
@Component({
  selector: 'mc-capacity-settings',
  imports: [FormsModule, IconComponent],
  template: `
    @if (!editing()) {
      <div class="card">
        <dl class="kv">
          <dt>Your limit</dt>
          <dd>{{ s().userCap }} role sessions at once (the lead not counted)</dd>
          <dt>A project's limit</dt>
          <dd>
            {{ s().projectCap }}
            @for (c of capEntries(); track c[0]) {
              <br /><span class="small">{{ short(c[0]) }}: {{ c[1] }}</span>
            }
          </dd>
          <dt>Memory</dt>
          <dd>
            {{ (s().reserveMb / 1024).toFixed(1) }} GB always free · a role session
            {{ s().sessionMb ? s().sessionMb + ' MB' : 'as measured (600 MB until then)' }}
          </dd>
          <dt>5-hour window</dt>
          <dd>
            only the lead starts at {{ pct(s().leadOnlyPct) }}, nothing at {{ pct(s().stopPct) }}
          </dd>
          <dt>Opus weekly window</dt>
          <dd>Opus work runs on Sonnet at high effort from {{ pct(s().opusGatePct) }}</dd>
        </dl>
        @if (view().canManage) {
          <button class="btn small edit" type="button" (click)="edit()">
            <mc-icon name="edit" [size]="14" /> Edit
          </button>
        }
      </div>
    } @else {
      <form class="card stack editor" (ngSubmit)="save()">
        <div class="pair">
          <label class="field">
            <span class="field-label">Your limit</span>
            <input
              name="userCap"
              type="number"
              min="1"
              max="16"
              [ngModel]="userCap()"
              (ngModelChange)="userCap.set($event)"
            />
            <span class="field-hint">Role sessions at once; the lead is not counted.</span>
          </label>
          <label class="field">
            <span class="field-label">A project's limit</span>
            <input
              name="projectCap"
              type="number"
              min="1"
              max="16"
              [ngModel]="projectCap()"
              (ngModelChange)="projectCap.set($event)"
            />
            <span class="field-hint">Role sessions in one repository at once.</span>
          </label>
        </div>
        <fieldset class="rows">
          <legend class="field-label">A project's own limit</legend>
          @for (r of caps(); track $index; let i = $index) {
            <div class="row">
              <input
                class="grow"
                [name]="'capProject' + i"
                aria-label="Project folder"
                placeholder="/Users/me/dev/app"
                [ngModel]="r.key"
                (ngModelChange)="setCap(i, 'key', $event)"
              />
              <input
                class="num"
                type="number"
                min="1"
                max="16"
                [name]="'cap' + i"
                aria-label="Limit"
                [ngModel]="r.cap"
                (ngModelChange)="setCap(i, 'cap', $event)"
              />
              <button
                class="btn ghost icon-only small"
                type="button"
                aria-label="Remove this limit"
                (click)="removeCap(i)"
              >
                <mc-icon name="x" [size]="14" />
              </button>
            </div>
          }
          <button class="btn small" type="button" (click)="addCap()">
            <mc-icon name="plus" [size]="14" /> Add a project
          </button>
        </fieldset>

        <div class="pair">
          <label class="field">
            <span class="field-label">Memory always free (GB)</span>
            <input
              name="reserve"
              inputmode="decimal"
              [ngModel]="reserveGb()"
              (ngModelChange)="reserveGb.set($event)"
            />
          </label>
          <label class="field">
            <span class="field-label">A role session needs (MB)</span>
            <input
              name="sessionMb"
              inputmode="numeric"
              placeholder="Measured"
              [ngModel]="sessionMb()"
              (ngModelChange)="sessionMb.set($event)"
            />
            <span class="field-hint">Empty: what live sessions measure.</span>
          </label>
        </div>

        <div class="pair">
          <label class="field">
            <span class="field-label">Only the lead starts at (%)</span>
            <input
              name="leadOnly"
              type="number"
              min="1"
              max="100"
              [ngModel]="leadOnly()"
              (ngModelChange)="leadOnly.set($event)"
            />
          </label>
          <label class="field">
            <span class="field-label">Nothing starts at (%)</span>
            <input
              name="stop"
              type="number"
              min="1"
              max="100"
              [ngModel]="stop()"
              (ngModelChange)="stop.set($event)"
            />
          </label>
          <label class="field">
            <span class="field-label">Opus gate at (%)</span>
            <input
              name="opusGate"
              type="number"
              min="1"
              max="100"
              [ngModel]="opusGate()"
              (ngModelChange)="opusGate.set($event)"
            />
          </label>
        </div>

        @if (error()) {
          <p class="notice error" role="alert">{{ error() }}</p>
        }
        <div class="row">
          <button class="btn primary" type="submit" [disabled]="saving()">Save</button>
          <button class="btn" type="button" [disabled]="saving()" (click)="editing.set(false)">
            Cancel
          </button>
        </div>
      </form>
    }

    @if (error() && !editing()) {
      <p class="notice error" role="alert">{{ error() }}</p>
    }

    <h2>How accounts pay</h2>
    <p class="muted small">
      Accounts that pay per token get --max-budget-usd on their role sessions; plan accounts are
      held by their usage windows instead.
    </p>
    <div class="card list">
      @for (a of view().accounts; track a.id) {
        <div class="list-row billing">
          <span class="grow">
            <strong>{{ a.name }}</strong
            ><br />
            <span class="muted small"
              >{{ billingText(a.billing.billing) }} — {{ a.billing.source }}</span
            >
          </span>
          @if (isDesktop()) {
            <select
              [attr.aria-label]="'How ' + a.name + ' pays'"
              [ngModel]="override(a.id)"
              (ngModelChange)="setBilling(a.id, $event)"
              [disabled]="saving()"
            >
              <option value="">As its sign-in says</option>
              <option value="plan">A plan</option>
              <option value="perToken">Per token</option>
            </select>
          }
        </div>
      }
    </div>

    <h2>Task classifier</h2>
    <div class="card stack">
      <p class="small">
        Tasks that arrive without a class get one from keywords in their title and goal. Claude
        Haiku can classify them instead: off by default because every task then costs a call ({{
          view().classifier.model
        }}, through the direct Messages API path, which needs an API key).
      </p>
      @if (isDesktop()) {
        <label class="switch">
          <input
            #haikuToggle
            type="checkbox"
            [checked]="view().classifier.enabled"
            [disabled]="saving()"
            (change)="setHaiku(haikuToggle.checked)"
          />
          <span class="switch-text">
            Classify with Claude Haiku (spends tokens)
            @if (view().classifier.whyNot; as why) {
              <span class="muted small">It cannot run now: {{ why }}.</span>
            }
          </span>
        </label>
      } @else {
        <p class="muted small">
          Haiku classifier: {{ view().classifier.enabled ? 'on' : 'off' }} (changed on the desktop).
        </p>
      }
      @if (view().classifier.stats.calls) {
        <p class="muted small">
          {{ view().classifier.stats.calls }} calls since the app started ·
          {{ tokens(view().classifier.stats.inputTokens + view().classifier.stats.outputTokens) }}
          tokens · {{ usd(view().classifier.stats.costUsd) }} (estimated)
        </p>
      }
      <form class="stack try" (ngSubmit)="tryIt(false)">
        <label class="field">
          <span class="field-label">Try it: a task title</span>
          <input name="tryTitle" [ngModel]="tryTitle()" (ngModelChange)="tryTitle.set($event)" />
        </label>
        <label class="field">
          <span class="field-label">Its goal</span>
          <input name="tryGoal" [ngModel]="tryGoal()" (ngModelChange)="tryGoal.set($event)" />
        </label>
        <div class="row">
          <button class="btn small" type="submit" [disabled]="trying()">Classify</button>
          @if (view().classifier.enabled && view().classifier.ready) {
            <button class="btn small" type="button" [disabled]="trying()" (click)="tryIt(true)">
              Ask Haiku
            </button>
          }
        </div>
        @if (tried(); as c) {
          <p class="small" role="status">
            <strong>{{ c.class }}</strong> · {{ c.reason }}
          </p>
        }
        @if (tryError()) {
          <p class="notice error small" role="alert">{{ tryError() }}</p>
        }
      </form>
    </div>
  `,
  styles: [
    `
      .editor {
        gap: 1rem;
      }
      .edit {
        margin-top: 0.75rem;
      }
      .pair {
        display: grid;
        grid-template-columns: repeat(auto-fit, minmax(11rem, 1fr));
        gap: 0.75rem;
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
      .grow {
        flex: 1 1 auto;
        min-width: 0;
        overflow-wrap: anywhere;
      }
      .num {
        width: 5rem;
        flex: 0 0 auto;
      }
      .list {
        padding-top: 0;
        padding-bottom: 0;
      }
      .billing select {
        width: auto;
        max-width: 12rem;
      }
      .try {
        border-top: 1px solid var(--border);
        padding-top: 0.75rem;
      }
      h2 {
        margin-top: 1.5rem;
      }
    `,
  ],
})
export class CapacitySettingsComponent {
  private api = inject(ApiService);
  protected readonly isDesktop = inject(AuthService).isDesktop;

  readonly view = input.required<CapacityView>();
  readonly changed = output<CapacityView>();

  protected readonly s = computed(() => this.view().settings);
  protected readonly capEntries = computed(() => Object.entries(this.s().projectCaps));
  protected readonly editing = signal(false);
  protected readonly saving = signal(false);
  protected readonly error = signal('');
  protected readonly userCap = signal<string | number>(3);
  protected readonly projectCap = signal<string | number>(4);
  protected readonly caps = signal<CapRow[]>([]);
  protected readonly reserveGb = signal('2');
  protected readonly sessionMb = signal('');
  protected readonly leadOnly = signal<string | number>(85);
  protected readonly stop = signal<string | number>(95);
  protected readonly opusGate = signal<string | number>(85);
  protected readonly tryTitle = signal('');
  protected readonly tryGoal = signal('');
  protected readonly tried = signal<Classification | null>(null);
  protected readonly tryError = signal('');
  protected readonly trying = signal(false);

  protected readonly pct = pctLabel;
  protected readonly short = shortPath;
  protected readonly usd = formatUsd;
  protected readonly tokens = formatTokens;

  protected billingText(b: Billing | undefined): string {
    return b === 'perToken'
      ? 'Pays per token'
      : b === 'plan'
        ? "Pays through its plan's usage windows"
        : 'Not known yet';
  }

  protected override(account: string): string {
    return this.s().billing[account] ?? '';
  }

  protected edit(): void {
    const s = this.s();
    this.userCap.set(s.userCap);
    this.projectCap.set(s.projectCap);
    this.caps.set(Object.entries(s.projectCaps).map(([key, cap]) => ({ key, cap: String(cap) })));
    this.reserveGb.set(String(Math.round((s.reserveMb / 1024) * 10) / 10));
    this.sessionMb.set(s.sessionMb === null ? '' : String(s.sessionMb));
    this.leadOnly.set(s.leadOnlyPct);
    this.stop.set(s.stopPct);
    this.opusGate.set(s.opusGatePct);
    this.error.set('');
    this.editing.set(true);
  }

  protected setCap(i: number, field: keyof CapRow, value: string): void {
    this.caps.update((rows) => rows.map((r, j) => (j === i ? { ...r, [field]: value } : r)));
  }

  protected addCap(): void {
    this.caps.update((rows) => [...rows, { key: '', cap: '2' }]);
  }

  protected removeCap(i: number): void {
    this.caps.update((rows) => rows.filter((_, j) => j !== i));
  }

  /** The update the form describes, or why not. */
  protected patch(): SettingsPatch | Problem {
    const userCap = readInt(this.userCap(), 1, 16, 'userCap', 'Your limit');
    if (isProblem(userCap)) return userCap;
    const projectCap = readInt(this.projectCap(), 1, 16, 'projectCap', "A project's limit");
    if (isProblem(projectCap)) return projectCap;
    const projectCaps: Record<string, number> = {};
    for (const r of this.caps().filter((row) => row.key.trim() || row.cap.trim())) {
      if (!r.key.trim())
        return { field: 'projectCaps', message: 'Name the project of each limit.' };
      const cap = readInt(r.cap, 1, 16, 'projectCaps', `The limit of ${r.key.trim()}`);
      if (isProblem(cap)) return cap;
      projectCaps[r.key.trim()] = cap;
    }
    const gb = Number(this.reserveGb().trim());
    if (!Number.isFinite(gb) || gb < 0 || gb > 1024) {
      return { field: 'reserveMb', message: 'The memory kept free is 0 to 1024 GB.' };
    }
    let sessionMb: number | null = null;
    if (this.sessionMb().trim()) {
      const mb = readInt(this.sessionMb(), 50, 65_536, 'sessionMb', 'A session’s memory');
      if (isProblem(mb)) return mb;
      sessionMb = mb;
    }
    const leadOnly = readPct(this.leadOnly(), 'leadOnlyPct', 'Lead-only');
    if (isProblem(leadOnly)) return leadOnly;
    const stop = readPct(this.stop(), 'stopPct', 'Nothing-starts');
    if (isProblem(stop)) return stop;
    if (stop < leadOnly) {
      return { field: 'stopPct', message: 'Nothing-starts must be at or above lead-only.' };
    }
    const opusGate = readPct(this.opusGate(), 'opusGatePct', 'The Opus gate');
    if (isProblem(opusGate)) return opusGate;
    return {
      userCap,
      projectCap,
      projectCaps,
      reserveMb: Math.round(gb * 1024),
      sessionMb,
      leadOnlyPct: leadOnly,
      stopPct: stop,
      opusGatePct: opusGate,
    };
  }

  async save(): Promise<void> {
    const p = this.patch();
    if (isProblem(p)) {
      this.error.set(p.message);
      return;
    }
    if (await this.send(p)) this.editing.set(false);
  }

  protected setBilling(account: string, value: string): void {
    const next = { ...this.s().billing };
    if (value === 'plan' || value === 'perToken') next[account] = value;
    else delete next[account];
    void this.send({ billing: next });
  }

  protected setHaiku(on: boolean): void {
    void this.send({ haikuClassifier: on });
  }

  private async send(patch: SettingsPatch): Promise<boolean> {
    this.saving.set(true);
    try {
      this.changed.emit(await this.api.setCapacitySettings(patch));
      this.error.set('');
      return true;
    } catch (e) {
      const field = e instanceof ApiError && e.field ? ` (${e.field})` : '';
      this.error.set(`${errorMessage(e)}${field}`);
      return false;
    } finally {
      this.saving.set(false);
    }
  }

  async tryIt(haiku: boolean): Promise<void> {
    this.trying.set(true);
    this.tryError.set('');
    try {
      this.tried.set(await this.api.classifyTask(this.tryTitle(), this.tryGoal(), haiku));
    } catch (e) {
      this.tried.set(null);
      this.tryError.set(errorMessage(e));
    } finally {
      this.trying.set(false);
    }
  }
}
