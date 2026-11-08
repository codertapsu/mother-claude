import { Component, effect, inject, input, output, signal } from '@angular/core';
import { FormsModule } from '@angular/forms';

import { ApiService, describeError } from '../../core/api.service';
import { TPipe } from '../../core/i18n/t.pipe';
import { PRIORITY_LABELS, TASK_CLASSES, TeamTask, TeamView, lines } from '../../core/teams';
import { SheetComponent } from '../../shared/sheet.component';

/** Write a task packet from the dashboard (the lead hears about it). */
@Component({
  selector: 'mc-new-task-sheet',
  imports: [FormsModule, SheetComponent, TPipe],
  template: `
    <mc-sheet [open]="open()" [title]="'New task' | t" (closed)="closed.emit()">
      <form class="stack form" (ngSubmit)="create()">
        <label class="field">
          <span class="field-label">{{ 'Title' | t }}</span>
          <input
            name="title"
            required
            maxlength="200"
            [ngModel]="title()"
            (ngModelChange)="title.set($event)"
          />
        </label>
        <label class="field">
          <span class="field-label">{{ 'Goal' | t }}</span>
          <textarea
            name="goal"
            rows="3"
            [placeholder]="'What and why, and what done looks like' | t"
            [ngModel]="goal()"
            (ngModelChange)="goal.set($event)"
          ></textarea>
        </label>
        <label class="field">
          <span class="field-label">{{ 'Acceptance criteria (one per line)' | t }}</span>
          <textarea
            name="acceptance"
            rows="3"
            [ngModel]="acceptance()"
            (ngModelChange)="acceptance.set($event)"
          ></textarea>
        </label>
        <div class="grid">
          <label class="field">
            <span class="field-label">{{ 'Role' | t }}</span>
            <select name="role" [ngModel]="role()" (ngModelChange)="role.set($event)">
              @for (r of team().roles; track r.id) {
                <option [value]="r.id">{{ r.title }}</option>
              }
            </select>
          </label>
          <label class="field">
            <span class="field-label">{{ 'Priority' | t }}</span>
            <select name="priority" [ngModel]="priority()" (ngModelChange)="priority.set(+$event)">
              @for (p of priorities; track $index) {
                <option [value]="$index">{{ p | t }}</option>
              }
            </select>
          </label>
          <label class="field">
            <span class="field-label">{{ 'Class' | t }}</span>
            <select name="class" [ngModel]="klass()" (ngModelChange)="klass.set($event)">
              <option value="">—</option>
              @for (c of classes; track c) {
                <option [value]="c">{{ c }}</option>
              }
            </select>
          </label>
        </div>
        <label class="field">
          <span class="field-label">{{ 'Files (one per line)' | t }}</span>
          <textarea
            name="files"
            rows="2"
            class="mono"
            [ngModel]="files()"
            (ngModelChange)="files.set($event)"
          ></textarea>
        </label>
        <label class="field">
          <span class="field-label">{{ 'Constraints (one per line)' | t }}</span>
          <textarea
            name="constraints"
            rows="2"
            [ngModel]="constraints()"
            (ngModelChange)="constraints.set($event)"
          ></textarea>
        </label>
        @if (error()) {
          <p class="notice error" role="alert">{{ error() }}</p>
        }
        <div class="row">
          <span class="spacer"></span>
          <button class="btn" type="button" (click)="closed.emit()">{{ 'Cancel' | t }}</button>
          <button class="btn primary" type="submit" [disabled]="busy() || !title().trim()">
            {{ busy() ? ('Adding…' | t) : ('Add the task' | t) }}
          </button>
        </div>
      </form>
    </mc-sheet>
  `,
  styles: [
    `
      .grid {
        display: grid;
        grid-template-columns: repeat(auto-fit, minmax(9rem, 1fr));
        gap: 0.5rem;
      }
    `,
  ],
})
export class NewTaskSheetComponent {
  private api = inject(ApiService);

  readonly open = input(false);
  readonly team = input.required<TeamView>();
  readonly closed = output<void>();
  readonly created = output<TeamTask>();

  protected readonly priorities = PRIORITY_LABELS;
  protected readonly classes = TASK_CLASSES;
  protected readonly title = signal('');
  protected readonly goal = signal('');
  protected readonly acceptance = signal('');
  protected readonly role = signal('');
  protected readonly priority = signal(2);
  protected readonly klass = signal('');
  protected readonly files = signal('');
  protected readonly constraints = signal('');
  protected readonly busy = signal(false);
  protected readonly error = signal('');

  constructor() {
    effect(() => {
      if (!this.open()) return;
      const roles = this.team().roles;
      if (!roles.some((r) => r.id === this.role())) {
        this.role.set((roles.find((r) => !r.lead) ?? roles[0])?.id ?? '');
      }
      this.error.set('');
    });
  }

  protected async create(): Promise<void> {
    if (!this.title().trim() || this.busy()) return;
    this.busy.set(true);
    this.error.set('');
    try {
      const task = await this.api.createTeamTask(this.team().id, {
        title: this.title().trim(),
        goal: this.goal().trim(),
        acceptance: lines(this.acceptance()),
        role: this.role() || undefined,
        class: this.klass() || undefined,
        priority: this.priority(),
        files: lines(this.files()),
        constraints: lines(this.constraints()),
      });
      for (const s of [this.title, this.goal, this.acceptance, this.files, this.constraints]) {
        s.set('');
      }
      this.created.emit(task);
    } catch (e) {
      this.error.set(describeError(e).text);
    } finally {
      this.busy.set(false);
    }
  }
}
