import { Component, computed, effect, inject, input, output, signal } from '@angular/core';
import { FormsModule } from '@angular/forms';

import { ApiService, describeError } from '../../core/api.service';
import { TemplateEntry, TemplateIssue, parseTemplate, templateJson } from '../../core/teams';
import { SheetComponent } from '../../shared/sheet.component';

export type TemplateSheetMode = 'view' | 'edit' | 'new';

/** A skeleton for a new template. */
const NEW_TEMPLATE = {
  id: 'my-team',
  name: 'My team',
  description: '',
  roles: [
    {
      id: 'lead',
      title: 'Lead',
      lead: true,
      charter: 'You plan the work, write one task per role and review what comes back.',
      model: 'opus',
      effort: 'high',
      permissionMode: 'default',
      paths: ['docs/**', '*.md'],
      autostart: true,
    },
    {
      id: 'dev',
      title: 'Developer',
      charter: 'You implement the tasks you are given, with tests.',
      model: 'sonnet',
      effort: 'medium',
      permissionMode: 'acceptEdits',
      worktree: true,
      paths: ['src/**'],
    },
  ],
  integration: { branch: 'main', mergeBy: 'lead', gate: 'npm test', review: true },
  idleStopMinutes: 30,
};

/** View a template, or edit one of the user's as JSON with validation. */
@Component({
  selector: 'mc-template-sheet',
  imports: [FormsModule, SheetComponent],
  template: `
    <mc-sheet [open]="open()" [title]="title()" size="full" (closed)="closed.emit()">
      <div class="stack">
        @if (readOnly()) {
          <p class="muted small">
            A built-in template cannot be changed: duplicate it to make your own.
          </p>
        } @else {
          <p class="muted small">
            Roles: id, title, lead, charter, model, effort, permissionMode, worktree, paths (globs
            relative to the project, gitignore style), readOnly, budgetUsd, autostart, addDirs,
            tools, classes. Exactly one role is the lead.
          </p>
        }
        <label class="field">
          <span class="field-label">Template (JSON)</span>
          <textarea
            class="mono json"
            name="json"
            spellcheck="false"
            [readOnly]="readOnly()"
            [ngModel]="text()"
            (ngModelChange)="text.set($event); checked.set(false)"
          ></textarea>
        </label>
        @if (parseError()) {
          <p class="notice error" role="alert">{{ parseError() }}</p>
        }
        @for (i of errors(); track $index) {
          <p class="notice error">{{ i.path ? i.path + ': ' : '' }}{{ i.message }}</p>
        }
        @for (i of warnings(); track $index) {
          <p class="notice warn">{{ i.path ? i.path + ': ' : '' }}{{ i.message }}</p>
        }
        @if (checked() && !errors().length && !parseError()) {
          <p class="notice ok">The template is valid.</p>
        }
        @if (error()) {
          <p class="notice error" role="alert">{{ error() }}</p>
        }
        <div class="row">
          <span class="spacer"></span>
          @if (readOnly()) {
            <button class="btn" type="button" [disabled]="busy()" (click)="duplicate()">
              Duplicate to edit
            </button>
          } @else {
            <button class="btn" type="button" [disabled]="busy()" (click)="check()">Check</button>
            <button class="btn primary" type="button" [disabled]="busy()" (click)="save()">
              {{ busy() ? 'Saving…' : 'Save' }}
            </button>
          }
        </div>
      </div>
    </mc-sheet>
  `,
  styles: [
    `
      .json {
        min-height: 50dvh;
        font-size: 0.78rem;
        line-height: 1.45;
        white-space: pre;
        overflow: auto;
      }
    `,
  ],
})
export class TemplateSheetComponent {
  private api = inject(ApiService);

  readonly open = input(false);
  readonly entry = input<TemplateEntry | null>(null);
  readonly mode = input<TemplateSheetMode>('view');
  readonly closed = output<void>();
  readonly saved = output<void>();
  readonly duplicated = output<void>();

  protected readonly text = signal('');
  protected readonly errors = signal<TemplateIssue[]>([]);
  protected readonly warnings = signal<TemplateIssue[]>([]);
  protected readonly parseError = signal('');
  protected readonly error = signal('');
  protected readonly checked = signal(false);
  protected readonly busy = signal(false);

  protected readonly readOnly = computed(() => this.mode() === 'view' || !!this.entry()?.builtin);
  protected readonly title = computed(() => {
    const e = this.entry();
    if (this.mode() === 'new' || !e) return 'New template';
    return this.readOnly() ? e.name : `Edit ${e.name}`;
  });

  constructor() {
    effect(() => {
      if (!this.open()) return;
      const e = this.entry();
      this.text.set(
        e && this.mode() !== 'new' ? templateJson(e) : JSON.stringify(NEW_TEMPLATE, null, 2),
      );
      this.errors.set(e?.report.errors ?? []);
      this.warnings.set(e?.report.warnings ?? []);
      this.parseError.set('');
      this.error.set('');
      this.checked.set(false);
    });
  }

  /** Validate on the server (what saving would say). */
  protected async check(): Promise<boolean> {
    const parsed = parseTemplate(this.text());
    this.parseError.set(parsed.error ?? '');
    if (!parsed.template) return false;
    try {
      const r = await this.api.validateTeamTemplate(parsed.template);
      this.errors.set(r.errors);
      this.warnings.set(r.warnings);
      this.checked.set(true);
      return r.ok;
    } catch (e) {
      this.error.set(describeError(e).text);
      return false;
    }
  }

  protected async save(): Promise<void> {
    this.busy.set(true);
    this.error.set('');
    try {
      if (!(await this.check())) return;
      const t = parseTemplate(this.text()).template;
      if (!t) return;
      const e = this.entry();
      if (this.mode() === 'new' || !e) {
        await this.api.createTeamTemplate(t);
      } else {
        await this.api.saveTeamTemplate(e.id, t);
      }
      this.saved.emit();
      this.closed.emit();
    } catch (e) {
      this.error.set(describeError(e).text);
    } finally {
      this.busy.set(false);
    }
  }

  protected async duplicate(): Promise<void> {
    const e = this.entry();
    if (!e) return;
    this.busy.set(true);
    try {
      await this.api.duplicateTeamTemplate(e.id);
      this.duplicated.emit();
      this.closed.emit();
    } catch (err) {
      this.error.set(describeError(err).text);
    } finally {
      this.busy.set(false);
    }
  }
}
