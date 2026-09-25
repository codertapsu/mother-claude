import { Component, computed, effect, inject, input, output, signal } from '@angular/core';
import { FormsModule } from '@angular/forms';

import { AuthService } from '../core/auth.service';
import { DefaultsService, FALLBACK_EFFORTS } from '../core/defaults.service';
import { LaunchOverrides, PermissionMode } from '../core/models';
import { PERMISSION_MODES, effortLabel } from './util';

const CUSTOM = '__custom__';

/**
 * Model / Effort / Permission-mode selectors for launching or continuing a
 * session. "Default" (empty) inherits the user's own Claude settings — exactly
 * what they last picked in VS Code or the CLI — shown inline so the
 * inheritance is visible. Parents read the chosen overrides via `value()`
 * (or the `changed` output). "Bypass permissions" is only offered to clients
 * allowed to take dangerous actions.
 */
@Component({
  selector: 'mc-launch-options',
  imports: [FormsModule],
  template: `
    <div class="launch-opts">
      <label class="field">
        <span class="field-label">Model</span>
        <select [ngModel]="model()" (ngModelChange)="model.set($event)">
          <option value="">Default{{ defaults()?.model ? ' — ' + defaults()?.model : '' }}</option>
          @for (m of defaults()?.models ?? []; track m.value) {
            <option [value]="m.value" [title]="m.description || ''">{{ m.label }}</option>
          }
          <option [value]="CUSTOM">Custom…</option>
        </select>
      </label>
      @if (model() === CUSTOM) {
        <label class="field">
          <span class="field-label">Custom model id</span>
          <input
            [ngModel]="customModel()"
            (ngModelChange)="customModel.set($event)"
            placeholder="e.g. claude-fable-5[1m]"
            spellcheck="false"
            autocapitalize="off"
          />
        </label>
      }
      <label class="field">
        <span class="field-label">Effort</span>
        <select [ngModel]="effort()" (ngModelChange)="effort.set($event)">
          <option value="">
            Default{{ defaults()?.effort ? ' — ' + label(defaults()!.effort!) : '' }}
          </option>
          @for (e of efforts(); track e) {
            <option [value]="e">{{ label(e) }}</option>
          }
        </select>
      </label>
      @if (showMode()) {
        <label class="field">
          <span class="field-label">Permissions</span>
          <select [ngModel]="mode()" (ngModelChange)="mode.set($event)">
            <option value="">Default — your settings</option>
            @for (m of modes; track m.value) {
              <option [value]="m.value" [disabled]="m.dangerous && !canDangerous()">
                {{ m.label }} — {{ m.hint }}
              </option>
            }
          </select>
        </label>
      }
    </div>
    @if (showMode() && !canDangerous()) {
      <p class="field-hint">
        “Bypass permissions” can only be chosen on the desktop (or on a device allowed to take
        dangerous actions).
      </p>
    }
  `,
  styles: [
    `
      .launch-opts {
        display: grid;
        grid-template-columns: repeat(auto-fit, minmax(11rem, 1fr));
        gap: 0.75rem;
      }
      .field-hint {
        margin: 0.4rem 0 0;
      }
    `,
  ],
})
export class LaunchOptionsComponent {
  private auth = inject(AuthService);
  private defaultsService = inject(DefaultsService);

  /** Offer the permission-mode selector. */
  readonly showMode = input(true);

  protected readonly CUSTOM = CUSTOM;
  protected readonly modes = PERMISSION_MODES;
  protected readonly canDangerous = this.auth.canDangerous;
  protected readonly defaults = this.defaultsService.defaults;
  protected readonly model = signal('');
  protected readonly customModel = signal('');
  protected readonly effort = signal('');
  protected readonly mode = signal<PermissionMode | ''>('');

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
    const mode = this.mode();
    if (mode && this.showMode()) out.permissionMode = mode;
    return out;
  });

  constructor() {
    void this.defaultsService.load();
    effect(() => this.changed.emit(this.value()));
  }

  protected label(effort: string): string {
    return effortLabel(effort);
  }
}
