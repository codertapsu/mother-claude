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
import { fill, tr } from '../../core/i18n/i18n';
import { TPipe } from '../../core/i18n/t.pipe';
import { IconComponent } from '../../shared/icon.component';
import { formatTokens, formatUsd, shortPath } from '../../shared/util';
import { Problem, isProblem } from './capacity-budgets.component';

interface CapRow {
  key: string;
  cap: string;
}

/** A whole number in `lo..=hi`, or why not: `message`, its `{lo}` and
 * `{hi}` filled in. */
export function readInt(
  v: string | number,
  lo: number,
  hi: number,
  field: string,
  message: string,
): number | Problem {
  const n = typeof v === 'number' ? v : Number(String(v).trim());
  return Number.isInteger(n) && n >= lo && n <= hi
    ? n
    : { field, message: fill(message, { lo, hi }) };
}

/** A percentage in 1..=100, or why not (`message`). */
export function readPct(v: string | number, field: string, message: string): number | Problem {
  const n = typeof v === 'number' ? v : Number(String(v).trim());
  return Number.isFinite(n) && n >= 1 && n <= 100 ? n : { field, message };
}

/** The scheduler's settings: caps, memory, plan-window thresholds, how
 * accounts pay and the optional Haiku classifier (the last two on the
 * desktop only). */
@Component({
  selector: 'mc-capacity-settings',
  imports: [FormsModule, IconComponent, TPipe],
  template: `
    @if (!editing()) {
      <div class="card">
        <dl class="kv">
          <dt>{{ 'Your limit' | t }}</dt>
          <dd>
            {{
              s().userCap === 1
                ? ('1 role session at once (the lead not counted)' | t)
                : ('{n} role sessions at once (the lead not counted)' | t: { n: s().userCap })
            }}
          </dd>
          <dt>{{ "A project's limit" | t }}</dt>
          <dd>
            {{ s().projectCap }}
            @for (c of capEntries(); track c[0]) {
              <br /><span class="small">{{ short(c[0]) }}: {{ c[1] }}</span>
            }
          </dd>
          <dt>{{ 'Memory' | t }}</dt>
          <dd>
            {{ '{gb} GB always free' | t: { gb: (s().reserveMb / 1024).toFixed(1) } }} ·
            {{
              s().sessionMb
                ? ('a role session {mb} MB' | t: { mb: s().sessionMb })
                : ('a role session as measured (600 MB until then)' | t)
            }}
          </dd>
          <dt>{{ '5-hour window' | t }}</dt>
          <dd>
            {{
              'only the lead starts at {lead}, nothing at {stop}'
                | t: { lead: pct(s().leadOnlyPct), stop: pct(s().stopPct) }
            }}
          </dd>
          <dt>{{ 'Opus weekly window' | t }}</dt>
          <dd>
            {{
              'Opus work runs on Sonnet at high effort from {pct}'
                | t: { pct: pct(s().opusGatePct) }
            }}
          </dd>
        </dl>
        @if (view().canManage) {
          <button class="btn small edit" type="button" (click)="edit()">
            <mc-icon name="edit" [size]="14" /> {{ 'Edit' | t }}
          </button>
        }
      </div>
    } @else {
      <form class="card stack editor" (ngSubmit)="save()">
        <div class="pair">
          <label class="field">
            <span class="field-label">{{ 'Your limit' | t }}</span>
            <input
              name="userCap"
              type="number"
              min="1"
              max="16"
              [ngModel]="userCap()"
              (ngModelChange)="userCap.set($event)"
            />
            <span class="field-hint">{{
              'Role sessions at once; the lead is not counted.' | t
            }}</span>
          </label>
          <label class="field">
            <span class="field-label">{{ "A project's limit" | t }}</span>
            <input
              name="projectCap"
              type="number"
              min="1"
              max="16"
              [ngModel]="projectCap()"
              (ngModelChange)="projectCap.set($event)"
            />
            <span class="field-hint">{{ 'Role sessions in one repository at once.' | t }}</span>
          </label>
        </div>
        <fieldset class="rows">
          <legend class="field-label">{{ "A project's own limit" | t }}</legend>
          @for (r of caps(); track $index; let i = $index) {
            <div class="row">
              <input
                class="grow"
                [name]="'capProject' + i"
                [attr.aria-label]="'Project folder' | t"
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
                [attr.aria-label]="'Limit' | t"
                [ngModel]="r.cap"
                (ngModelChange)="setCap(i, 'cap', $event)"
              />
              <button
                class="btn ghost icon-only small"
                type="button"
                [attr.aria-label]="'Remove this limit' | t"
                (click)="removeCap(i)"
              >
                <mc-icon name="x" [size]="14" />
              </button>
            </div>
          }
          <button class="btn small" type="button" (click)="addCap()">
            <mc-icon name="plus" [size]="14" /> {{ 'Add a project' | t }}
          </button>
        </fieldset>

        <div class="pair">
          <label class="field">
            <span class="field-label">{{ 'Memory always free (GB)' | t }}</span>
            <input
              name="reserve"
              inputmode="decimal"
              [ngModel]="reserveGb()"
              (ngModelChange)="reserveGb.set($event)"
            />
          </label>
          <label class="field">
            <span class="field-label">{{ 'A role session needs (MB)' | t }}</span>
            <input
              name="sessionMb"
              inputmode="numeric"
              [placeholder]="'Measured' | t"
              [ngModel]="sessionMb()"
              (ngModelChange)="sessionMb.set($event)"
            />
            <span class="field-hint">{{ 'Empty: what live sessions measure.' | t }}</span>
          </label>
        </div>

        <div class="pair">
          <label class="field">
            <span class="field-label">{{ 'Only the lead starts at (%)' | t }}</span>
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
            <span class="field-label">{{ 'Nothing starts at (%)' | t }}</span>
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
            <span class="field-label">{{ 'Opus gate at (%)' | t }}</span>
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
          <button class="btn primary" type="submit" [disabled]="saving()">
            {{ 'Save' | t }}
          </button>
          <button class="btn" type="button" [disabled]="saving()" (click)="editing.set(false)">
            {{ 'Cancel' | t }}
          </button>
        </div>
      </form>
    }

    @if (error() && !editing()) {
      <p class="notice error" role="alert">{{ error() }}</p>
    }

    <h2>{{ 'How accounts pay' | t }}</h2>
    <p class="muted small">
      {{
        'Accounts that pay per token get --max-budget-usd on their role sessions; plan accounts are held by their usage windows instead.'
          | t
      }}
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
              [attr.aria-label]="'How {name} pays' | t: { name: a.name }"
              [ngModel]="override(a.id)"
              (ngModelChange)="setBilling(a.id, $event)"
              [disabled]="saving()"
            >
              <option value="">{{ 'As its sign-in says' | t }}</option>
              <option value="plan">{{ 'A plan' | t }}</option>
              <option value="perToken">{{ 'Per token' | t }}</option>
            </select>
          }
        </div>
      }
    </div>

    <h2>{{ 'Task classifier' | t }}</h2>
    <div class="card stack">
      <p class="small">
        {{
          'Tasks that arrive without a class get one from keywords in their title and goal. Claude Haiku can classify them instead: off by default because every task then costs a call ({model}, through the direct Messages API path, which needs an API key).'
            | t: { model: view().classifier.model }
        }}
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
            {{ 'Classify with Claude Haiku (spends tokens)' | t }}
            @if (view().classifier.whyNot; as why) {
              <span class="muted small">{{ 'It cannot run now: {why}.' | t: { why } }}</span>
            }
          </span>
        </label>
      } @else {
        <p class="muted small">
          {{
            view().classifier.enabled
              ? ('Haiku classifier: on (changed on the desktop).' | t)
              : ('Haiku classifier: off (changed on the desktop).' | t)
          }}
        </p>
      }
      @if (view().classifier.stats.calls) {
        <p class="muted small">
          {{
            '{calls} calls since the app started · {tokens} tokens · {usd} (estimated)'
              | t
                : {
                    calls: view().classifier.stats.calls,
                    tokens: tokens(
                      view().classifier.stats.inputTokens + view().classifier.stats.outputTokens
                    ),
                    usd: usd(view().classifier.stats.costUsd),
                  }
          }}
        </p>
      }
      <form class="stack try" (ngSubmit)="tryIt(false)">
        <label class="field">
          <span class="field-label">{{ 'Try it: a task title' | t }}</span>
          <input name="tryTitle" [ngModel]="tryTitle()" (ngModelChange)="tryTitle.set($event)" />
        </label>
        <label class="field">
          <span class="field-label">{{ 'Its goal' | t }}</span>
          <input name="tryGoal" [ngModel]="tryGoal()" (ngModelChange)="tryGoal.set($event)" />
        </label>
        <div class="row">
          <button class="btn small" type="submit" [disabled]="trying()">
            {{ 'Classify' | t }}
          </button>
          @if (view().classifier.enabled && view().classifier.ready) {
            <button class="btn small" type="button" [disabled]="trying()" (click)="tryIt(true)">
              {{ 'Ask Haiku' | t }}
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
      ? tr('Pays per token')
      : b === 'plan'
        ? tr("Pays through its plan's usage windows")
        : tr('Not known yet');
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
    const userCap = readInt(
      this.userCap(),
      1,
      16,
      'userCap',
      tr('Your limit must be a whole number from {lo} to {hi}.'),
    );
    if (isProblem(userCap)) return userCap;
    const projectCap = readInt(
      this.projectCap(),
      1,
      16,
      'projectCap',
      tr("A project's limit must be a whole number from {lo} to {hi}."),
    );
    if (isProblem(projectCap)) return projectCap;
    const projectCaps: Record<string, number> = {};
    for (const r of this.caps().filter((row) => row.key.trim() || row.cap.trim())) {
      if (!r.key.trim())
        return { field: 'projectCaps', message: tr('Name the project of each limit.') };
      const cap = readInt(
        r.cap,
        1,
        16,
        'projectCaps',
        tr('The limit of {project} must be a whole number from {lo} to {hi}.', {
          project: r.key.trim(),
        }),
      );
      if (isProblem(cap)) return cap;
      projectCaps[r.key.trim()] = cap;
    }
    const gb = Number(this.reserveGb().trim());
    if (!Number.isFinite(gb) || gb < 0 || gb > 1024) {
      return { field: 'reserveMb', message: tr('The memory kept free is 0 to 1024 GB.') };
    }
    let sessionMb: number | null = null;
    if (this.sessionMb().trim()) {
      const mb = readInt(
        this.sessionMb(),
        50,
        65_536,
        'sessionMb',
        tr('A session’s memory must be a whole number from {lo} to {hi}.'),
      );
      if (isProblem(mb)) return mb;
      sessionMb = mb;
    }
    const leadOnly = readPct(
      this.leadOnly(),
      'leadOnlyPct',
      tr('Lead-only must be between 1 and 100 %.'),
    );
    if (isProblem(leadOnly)) return leadOnly;
    const stop = readPct(this.stop(), 'stopPct', tr('Nothing-starts must be between 1 and 100 %.'));
    if (isProblem(stop)) return stop;
    if (stop < leadOnly) {
      return { field: 'stopPct', message: tr('Nothing-starts must be at or above lead-only.') };
    }
    const opusGate = readPct(
      this.opusGate(),
      'opusGatePct',
      tr('The Opus gate must be between 1 and 100 %.'),
    );
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
