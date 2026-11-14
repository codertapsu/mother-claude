import {
  ChangeDetectionStrategy,
  Component,
  computed,
  inject,
  input,
  linkedSignal,
  output,
  signal,
} from '@angular/core';

import { describeError } from '../../core/api.service';
import { AuthService } from '../../core/auth.service';
import { computerNoun, tr } from '../../core/i18n/i18n';
import { TPipe } from '../../core/i18n/t.pipe';
import { GrantScope } from '../../core/models';
import { NoticeService } from '../../core/notice.service';
import {
  PermissionRule,
  PermissionRulePatch,
  SCOPES,
  anyOf,
  cleanPatterns,
  needsDangerous,
  patternsHeading,
  scopeLabel,
  wider,
} from '../../core/permission-rules';
import { PermissionRulesService } from '../../core/permission-rules.service';
import { IconComponent } from '../../shared/icon.component';
import { PatternRowsComponent } from '../../shared/pattern-rows.component';
import { SheetComponent } from '../../shared/sheet.component';

/** Each sheet's radio groups have their own names. */
let sheets = 0;

/**
 * *Edit* on Permissions: what one allows — anything of its kind, or only
 * what matches its patterns (`mc-pattern-rows`) — and where it applies (its
 * session, its project, everywhere: the ones it has a session or a project
 * for). Allowing more for a project or everywhere needs the dangerous
 * right: without it those choices are disabled and say so; the server
 * decides the rest (`403`, `422` shown as they come). Saving changes only
 * what changed. Opens while `rule` is set; `closed` when done.
 */
@Component({
  selector: 'mc-rule-sheet',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [IconComponent, PatternRowsComponent, SheetComponent, TPipe],
  template: `
    <mc-sheet [open]="!!rule()" [title]="'Edit permission' | t" (closed)="closed.emit()">
      @if (rule(); as r) {
        <div class="form">
          <p class="for">
            @if (r.kind === 'tool') {
              <span class="tag mono">{{ r.tool }}</span>
            } @else {
              <mc-icon name="monitor" [size]="14" />
              <span>{{ r.machineName }}</span>
            }
          </p>
          @if (r.patternKind; as kind) {
            <fieldset>
              <legend class="field-label">{{ 'What it allows' | t }}</legend>
              <label class="radio" [class.locked]="anyLocked()">
                <input
                  type="radio"
                  [name]="group + '-what'"
                  [checked]="mode() === 'any'"
                  [disabled]="anyLocked()"
                  (change)="mode.set('any')"
                />
                {{ anyText() }}
              </label>
              <label class="radio">
                <input
                  type="radio"
                  [name]="group + '-what'"
                  [checked]="mode() === 'only'"
                  (change)="mode.set('only')"
                />
                {{ onlyText() }}
              </label>
            </fieldset>
            @if (mode() === 'only') {
              <mc-pattern-rows [(rows)]="rows" [kind]="kind" />
            }
          }
          <fieldset>
            <legend class="field-label">{{ 'Where it applies' | t }}</legend>
            @for (c of scopes(); track c.scope) {
              <label class="radio" [class.locked]="c.locked">
                <input
                  type="radio"
                  [name]="group + '-where'"
                  [checked]="scope() === c.scope"
                  [disabled]="c.locked"
                  (change)="scope.set(c.scope)"
                />
                {{ c.label }}
              </label>
            }
          </fieldset>
          @if (lockedText()) {
            <p class="muted small why">{{ lockedText() }}</p>
          }
          @if (error()) {
            <p class="error-text small" role="alert">{{ error() }}</p>
          }
          <div class="actions">
            <button
              class="btn primary"
              type="button"
              [disabled]="saving() || !valid()"
              (click)="save()"
            >
              {{ saving() ? ('Saving…' | t) : ('Save' | t) }}
            </button>
            <button class="btn ghost" type="button" (click)="closed.emit()">
              {{ 'Cancel' | t }}
            </button>
          </div>
        </div>
      }
    </mc-sheet>
  `,
  styles: [
    `
      .form {
        display: flex;
        flex-direction: column;
        gap: 0.75rem;
        padding-bottom: 0.25rem;
      }
      .for {
        display: flex;
        align-items: center;
        gap: 0.4rem;
        margin: 0;
        color: var(--muted);
      }
      .tag {
        font-size: 0.75rem;
        padding: 0.05rem 0.45rem;
        border-radius: 0.35rem;
        border: 1px solid var(--border);
      }
      fieldset {
        border: 0;
        margin: 0;
        padding: 0;
        display: flex;
        flex-direction: column;
        gap: 0.1rem;
        min-width: 0;
      }
      legend {
        padding: 0;
        margin-bottom: 0.25rem;
      }
      .radio {
        display: flex;
        align-items: center;
        gap: 0.5rem;
        min-height: 32px;
        font-size: 0.9rem;
        overflow-wrap: anywhere;
      }
      .radio.locked {
        color: var(--muted);
      }
      .why,
      .error-text {
        margin: 0;
      }
      .actions {
        display: flex;
        flex-wrap: wrap;
        gap: 0.45rem;
      }
    `,
  ],
})
export class RuleSheetComponent {
  private readonly permissions = inject(PermissionRulesService);
  private readonly auth = inject(AuthService);
  private readonly notices = inject(NoticeService);

  /** The permission to edit (null: closed). */
  readonly rule = input<PermissionRule | null>(null);
  readonly closed = output<void>();

  protected readonly group = `rule-sheet-${++sheets}`;
  /** Anything of its kind, or only what matches. */
  protected readonly mode = linkedSignal<'any' | 'only'>(() =>
    this.rule()?.patterns.length ? 'only' : 'any',
  );
  protected readonly rows = linkedSignal<string[]>(() => {
    const p = this.rule()?.patterns ?? [];
    return p.length ? [...p] : [''];
  });
  protected readonly scope = linkedSignal<GrantScope>(() => this.rule()?.scope ?? 'session');
  protected readonly saving = signal(false);
  protected readonly error = linkedSignal({ source: this.rule, computation: () => '' });

  protected readonly anyText = computed(() => {
    const r = this.rule();
    return r ? anyOf(r) : '';
  });
  protected readonly onlyText = computed(() =>
    patternsHeading(this.rule()?.patternKind ?? 'command'),
  );
  /** Its scopes: the ones it has a session or a project for, and
   * everywhere; widening to a project or everywhere needs the right. */
  protected readonly scopes = computed(() => {
    const r = this.rule();
    if (!r) return [];
    const session = r.sessionName || r.session?.slice(0, 8);
    return SCOPES.filter(
      (s) => s === r.scope || s === 'computer' || (s === 'project' ? !!r.project : !!r.session),
    ).map((scope) => ({
      scope,
      label: scopeLabel(r.kind, scope, { project: r.projectName, session }),
      locked: wider(r.scope, scope) && this.locked(scope),
    }));
  });
  /** Anything of its kind, where only some was allowed: more, which needs
   * the right for a project or everywhere. */
  protected readonly anyLocked = computed(
    () => !!this.rule()?.patterns.length && this.locked(this.scope()),
  );
  protected readonly lockedText = computed(() =>
    this.anyLocked() || this.scopes().some((c) => c.locked)
      ? tr(
          'To allow more for a project or everywhere, use the {computer}, or a device allowed to take dangerous actions (Phone & devices).',
          { computer: computerNoun() },
        )
      : '',
  );
  protected readonly valid = computed(
    () => this.mode() === 'any' || cleanPatterns(this.rows()).length > 0,
  );

  private locked(scope: GrantScope): boolean {
    return needsDangerous(scope) && !this.auth.canDangerous();
  }

  protected async save(): Promise<void> {
    const r = this.rule();
    if (!r || !this.valid() || this.saving()) return;
    const patch: PermissionRulePatch = {};
    if (r.patternKind) {
      const patterns = this.mode() === 'any' ? [] : cleanPatterns(this.rows());
      if (patterns.join('\n') !== r.patterns.join('\n')) patch.patterns = patterns;
    }
    if (this.scope() !== r.scope) patch.scope = this.scope();
    if (!Object.keys(patch).length) {
      this.closed.emit();
      return;
    }
    this.saving.set(true);
    this.error.set('');
    try {
      await this.permissions.update(r.id, patch);
      this.notices.success(tr('Permission saved'));
      this.closed.emit();
    } catch (e) {
      this.error.set(describeError(e).text);
    } finally {
      this.saving.set(false);
    }
  }
}
