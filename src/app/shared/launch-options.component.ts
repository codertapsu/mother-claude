import { Component, computed, effect, inject, output, signal } from '@angular/core';
import { FormsModule } from '@angular/forms';

import { ApiService } from '../core/api.service';
import { LaunchDefaults, LaunchOverrides } from '../core/models';

const CUSTOM = '__custom__';

/** Effort levels the CLI accepts; the server-provided list overrides this. */
const FALLBACK_EFFORTS = ['low', 'medium', 'high', 'xhigh', 'max'];

const EFFORT_LABELS: Record<string, string> = {
  low: 'Low',
  medium: 'Medium',
  high: 'High',
  xhigh: 'Extra High',
  max: 'Max',
};

/**
 * Model / Effort / Thinking selectors for launching or continuing a session.
 * "Default" (empty) inherits the user's own Claude settings — exactly what
 * they last picked in VS Code or the CLI — shown inline so the inheritance is
 * visible. Parents read the chosen overrides via `value()`.
 */
@Component({
  selector: 'mc-launch-options',
  imports: [FormsModule],
  template: `
    <div class="launch-opts">
      <label>
        <span class="lbl">Model</span>
        <select [ngModel]="model()" (ngModelChange)="model.set($event)">
          <option value="">Default{{ defaults()?.model ? ' — ' + defaults()?.model : '' }}</option>
          @for (m of defaults()?.models ?? []; track m.value) {
            <option [value]="m.value" [title]="m.description || ''">{{ m.label }}</option>
          }
          <option [value]="CUSTOM">Custom…</option>
        </select>
      </label>
      @if (model() === CUSTOM) {
        <label>
          <span class="lbl">Custom model id</span>
          <input
            [ngModel]="customModel()"
            (ngModelChange)="customModel.set($event)"
            placeholder="e.g. claude-fable-5[1m]"
          />
        </label>
      }
      <label>
        <span class="lbl">Effort</span>
        <select [ngModel]="effort()" (ngModelChange)="effort.set($event)">
          <option value="">
            Default{{ defaults()?.effort ? ' — ' + effortLabel(defaults()!.effort!) : '' }}
          </option>
          @for (e of efforts(); track e) {
            <option [value]="e">{{ effortLabel(e) }}</option>
          }
        </select>
      </label>
      <label>
        <span class="lbl">Thinking</span>
        <select [ngModel]="thinking()" (ngModelChange)="thinking.set($event)">
          <option value="">Default</option>
          <option value="on">On</option>
          <option value="off">Off</option>
        </select>
      </label>
    </div>
    <p class="muted hint">
      Default keeps your own Claude settings — the same model and effort you last used in
      VS&nbsp;Code or the CLI.
    </p>
  `,
  styles: [
    `
      .launch-opts {
        display: flex;
        gap: 0.6rem;
        flex-wrap: wrap;
      }
      label {
        display: flex;
        flex-direction: column;
        gap: 0.2rem;
        min-width: 0;
      }
      .lbl {
        font-size: 0.75rem;
        color: var(--muted);
      }
      select,
      input {
        min-width: 9rem;
      }
      .hint {
        margin: 0.3rem 0 0;
        font-size: 0.75rem;
      }
    `,
  ],
})
export class LaunchOptionsComponent {
  private api = inject(ApiService);

  protected readonly CUSTOM = CUSTOM;
  protected readonly defaults = signal<LaunchDefaults | null>(null);
  protected readonly model = signal('');
  protected readonly customModel = signal('');
  protected readonly effort = signal('');
  protected readonly thinking = signal('');

  protected readonly efforts = computed(() => {
    const list = this.defaults()?.efforts;
    return list?.length ? list : FALLBACK_EFFORTS;
  });

  /** "Custom…" chosen but no model id typed — parents should block launch. */
  readonly invalid = computed(() => this.model() === CUSTOM && !this.customModel().trim());

  /** Emitted on every change so parents can keep the overrides even if this
   * component is destroyed (e.g. the containing card unrenders on tab switch). */
  readonly changed = output<LaunchOverrides>();

  /** The overrides to send — only what differs from "Default". */
  readonly value = computed<LaunchOverrides>(() => {
    const out: LaunchOverrides = {};
    const model = this.model() === CUSTOM ? this.customModel().trim() : this.model();
    if (model) out.model = model;
    if (this.effort()) out.effort = this.effort();
    const thinking = this.thinking();
    if (thinking === 'on' || thinking === 'off') out.thinking = thinking;
    return out;
  });

  constructor() {
    void this.load();
    effect(() => this.changed.emit(this.value()));
  }

  protected effortLabel(effort: string): string {
    return EFFORT_LABELS[effort] ?? effort;
  }

  private async load(): Promise<void> {
    try {
      this.defaults.set(await this.api.getDefaults());
    } catch {
      // Selectors still work with the fallback lists; Default stays unlabeled.
    }
  }
}
