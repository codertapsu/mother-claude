import { Component, computed, effect, inject, input, output, signal } from '@angular/core';
import { FormsModule } from '@angular/forms';
import { RouterLink } from '@angular/router';

import { ApiService, describeError } from '../../core/api.service';
import { tr } from '../../core/i18n/i18n';
import { TPipe } from '../../core/i18n/t.pipe';
import { PRIORITY_LABELS, STATUS_LABELS, TaskPatch, TeamTask, TeamView } from '../../core/teams';
import { SheetComponent } from '../../shared/sheet.component';
import { TimeAgoComponent } from '../../shared/time-ago.component';

/** One task: its packet, progress, result and history, and what the user
 * can do with it (reprioritise, reassign, note, accept, send back, cancel,
 * reopen). */
@Component({
  selector: 'mc-task-sheet',
  imports: [FormsModule, RouterLink, SheetComponent, TimeAgoComponent, TPipe],
  template: `
    <mc-sheet [open]="open()" [title]="heading()" (closed)="closed.emit()">
      @if (task(); as t) {
        <div class="stack">
          <div class="row">
            <span [class]="'badge ' + tone(t)">{{ statusLabel(t) }}</span>
            @if (t.role) {
              <span class="badge">{{ t.role }}</span>
            }
            @if (t.class) {
              <span class="badge">{{ t.class }}</span>
            }
            @if (t.escalations) {
              <span class="badge info">{{ 'escalated {n}' | t: { n: t.escalations } }}</span>
            }
            @if (t.claimedBy) {
              <a class="small mono" [routerLink]="['/session', t.claimedBy]">{{
                holder(t.claimedBy)
              }}</a>
            }
          </div>
          @if (t.blocked) {
            <p class="notice warn">{{ 'Blocked: {reason}' | t: { reason: t.blocked } }}</p>
          }
          @if (t.goal) {
            <p class="goal">{{ t.goal }}</p>
          }
          @if (t.acceptance?.length) {
            <div>
              <div class="field-label">{{ 'Acceptance' | t }}</div>
              <ul>
                @for (a of t.acceptance; track $index) {
                  <li>{{ a }}</li>
                }
              </ul>
            </div>
          }
          @if (t.files?.length) {
            <div>
              <div class="field-label">{{ 'Files' | t }}</div>
              <div class="mono small wrap">{{ t.files.join(', ') }}</div>
            </div>
          }
          @if (t.constraints?.length) {
            <div>
              <div class="field-label">{{ 'Constraints' | t }}</div>
              <ul>
                @for (c of t.constraints; track $index) {
                  <li>{{ c }}</li>
                }
              </ul>
            </div>
          }
          @if (t.model) {
            <div class="small muted">
              {{ t.model.model }}{{ t.model.effort ? ' @ ' + t.model.effort : '' }}
              @if (t.modelReason) {
                — {{ t.modelReason }}
              }
            </div>
          }
          @if (t.result) {
            <div class="card result">
              <div class="field-label">{{ 'Result' | t }}</div>
              <p class="goal">{{ t.result.summary }}</p>
              @if (t.result.branch) {
                <div class="mono small">{{ 'branch {name}' | t: { name: t.result.branch } }}</div>
              }
              @if (t.result.changedFiles?.length) {
                <div class="mono small wrap">{{ t.result.changedFiles.join(', ') }}</div>
              }
            </div>
          }
          @if (t.progress?.length) {
            <div>
              <div class="field-label">{{ 'Progress' | t }}</div>
              @for (n of t.progress; track $index) {
                <div class="small note">
                  <mc-time class="faint" [ms]="n.at" /> <strong>{{ n.by }}</strong> {{ n.text }}
                </div>
              }
            </div>
          }

          <div class="grid">
            <label class="field">
              <span class="field-label">{{ 'Priority' | t }}</span>
              <select
                [disabled]="busy() || ended()"
                [ngModel]="t.priority"
                (ngModelChange)="patch({ priority: +$event })"
              >
                @for (p of priorities; track $index) {
                  <option [value]="$index">{{ p | t }}</option>
                }
              </select>
            </label>
            <label class="field">
              <span class="field-label">{{ 'Role' | t }}</span>
              <select
                [disabled]="busy() || ended()"
                [ngModel]="t.role ?? ''"
                (ngModelChange)="patch({ role: $event })"
              >
                @for (r of team().roles; track r.id) {
                  <option [value]="r.id">{{ r.title }}</option>
                }
              </select>
            </label>
          </div>
          <label class="field">
            <span class="field-label">{{ 'Note to the session' | t }}</span>
            <textarea rows="2" [ngModel]="note()" (ngModelChange)="note.set($event)"></textarea>
          </label>
          @if (t.status === 'review') {
            <label class="check small">
              <input
                type="checkbox"
                [ngModel]="escalate()"
                (ngModelChange)="escalate.set($event)"
              />
              <span>{{ 'Sending it back: give it a stronger model or more effort' | t }}</span>
            </label>
          }
          @if (error()) {
            <p class="notice error" role="alert">{{ error() }}</p>
          }
          <div class="row actions">
            <button
              class="btn small"
              type="button"
              [disabled]="busy() || !note().trim()"
              (click)="addNote()"
            >
              {{ 'Add note' | t }}
            </button>
            @if (t.blocked) {
              <button
                class="btn small"
                type="button"
                [disabled]="busy()"
                (click)="patch({ blocked: '' })"
              >
                {{ 'Unblock' | t }}
              </button>
            }
            <span class="spacer"></span>
            @if (t.status === 'review') {
              <button class="btn small" type="button" [disabled]="busy()" (click)="sendBack()">
                {{ 'Send back' | t }}
              </button>
              <button
                class="btn small ok"
                type="button"
                [disabled]="busy()"
                (click)="patch({ status: 'done' })"
              >
                {{ 'Accept' | t }}
              </button>
            }
            @if (ended()) {
              <button
                class="btn small"
                type="button"
                [disabled]="busy()"
                (click)="patch({ status: 'todo' })"
              >
                {{ 'Reopen' | t }}
              </button>
            } @else {
              <button class="btn small danger" type="button" [disabled]="busy()" (click)="cancel()">
                {{ 'Cancel task' | t }}
              </button>
            }
          </div>
          @if (t.history?.length) {
            <details>
              <summary class="muted small">{{ 'History' | t }}</summary>
              @for (h of t.history; track $index) {
                <div class="small note">
                  <mc-time class="faint" [ms]="h.at" /> {{ h.by }} · {{ h.action }}
                  @if (h.detail) {
                    <span class="muted"> — {{ h.detail }}</span>
                  }
                </div>
              }
            </details>
          }
        </div>
      }
    </mc-sheet>
  `,
  styles: [
    `
      .goal {
        white-space: pre-wrap;
        margin: 0;
      }
      ul {
        margin: 0.2rem 0 0;
        padding-left: 1.2rem;
      }
      .wrap {
        overflow-wrap: anywhere;
      }
      .note {
        margin-top: 0.2rem;
        overflow-wrap: anywhere;
      }
      .grid {
        display: grid;
        grid-template-columns: repeat(auto-fit, minmax(9rem, 1fr));
        gap: 0.5rem;
      }
      .check {
        display: inline-flex;
        gap: 0.4rem;
        align-items: center;
      }
      .actions {
        gap: 0.35rem;
      }
      .result {
        margin: 0;
      }
    `,
  ],
})
export class TaskSheetComponent {
  private api = inject(ApiService);

  readonly open = input(false);
  readonly team = input.required<TeamView>();
  readonly task = input<TeamTask | null>(null);
  readonly closed = output<void>();
  readonly changed = output<TeamTask>();

  protected readonly priorities = PRIORITY_LABELS;
  protected readonly note = signal('');
  protected readonly escalate = signal(false);
  protected readonly busy = signal(false);
  protected readonly error = signal('');

  protected readonly heading = computed(() => {
    const t = this.task();
    return t ? `#${t.id} ${t.title}` : tr('Task');
  });
  protected readonly ended = computed(() =>
    ['done', 'failed', 'cancelled'].includes(this.task()?.status ?? ''),
  );

  constructor() {
    effect(() => {
      if (this.open()) {
        this.error.set('');
        this.escalate.set(false);
      }
    });
  }

  protected statusLabel(t: TeamTask): string {
    return tr(STATUS_LABELS[t.status] ?? t.status);
  }

  protected tone(t: TeamTask): string {
    switch (t.status) {
      case 'done':
        return 'ok';
      case 'failed':
        return 'danger';
      case 'review':
        return 'warn';
      case 'claimed':
      case 'in_progress':
        return 'working';
      default:
        return '';
    }
  }

  /** The role whose session holds it, else the session's short id. */
  protected holder(session: string): string {
    const r = this.team().roles.find((x) => x.sessionId === session);
    return r ? r.sessionName : session.slice(0, 8);
  }

  protected async patch(patch: TaskPatch): Promise<void> {
    const t = this.task();
    if (!t || this.busy()) return;
    this.busy.set(true);
    this.error.set('');
    try {
      this.changed.emit(await this.api.updateTeamTask(this.team().id, t.id, patch));
    } catch (e) {
      this.error.set(describeError(e).text);
    } finally {
      this.busy.set(false);
    }
  }

  protected async addNote(): Promise<void> {
    await this.patch({ note: this.note().trim() });
    if (!this.error()) this.note.set('');
  }

  protected async sendBack(): Promise<void> {
    await this.patch({
      status: 'todo',
      ...(this.note().trim() ? { note: this.note().trim() } : {}),
      ...(this.escalate() ? { escalate: true } : {}),
    });
    if (!this.error()) this.note.set('');
  }

  protected async cancel(): Promise<void> {
    if (!confirm(tr('Cancel this task? Its session is told to stop working on it.'))) return;
    await this.patch({ status: 'cancelled' });
  }
}
