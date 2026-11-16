import {
  ChangeDetectionStrategy,
  Component,
  computed,
  effect,
  inject,
  input,
  output,
  signal,
  untracked,
} from '@angular/core';

import { describeError } from '../../core/api.service';
import { computerNoun, tr } from '../../core/i18n/i18n';
import { TPipe } from '../../core/i18n/t.pipe';
import { SessionChoice, SessionsChoiceView, SessionsShownMode } from '../../core/machines';
import { MachinesService } from '../../core/machines.service';
import { SheetComponent } from '../../shared/sheet.component';
import { RelativeTimePipe } from '../../shared/time.pipes';

/** A session's name in the list: its title, else its name, project or id. */
export function choiceName(s: SessionChoice): string {
  return s.title?.trim() || s.name?.trim() || s.projectName || s.id.slice(0, 8);
}

/**
 * *Sessions of DESKTOP-PC shown here* (docs/MACHINES.md, "Choosing which
 * sessions show here"): every session of a paired computer shows on this one
 * but the ones turned off, or only the ones chosen — and each of its
 * sessions as last heard, the hidden ones too, with a box to sync it to this
 * computer or stop. Each change applies at once, to every dashboard of this
 * computer (lists, counts, Inbox). Nothing changes on the other computer:
 * it is never told. The desktop or a paired phone may change it.
 */
@Component({
  selector: 'mc-sessions-shown-sheet',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [RelativeTimePipe, SheetComponent, TPipe],
  template: `
    <mc-sheet
      [open]="open()"
      [title]="'Sessions of {name} shown here' | t: { name: name() }"
      (closed)="closed.emit()"
    >
      <p class="muted small">
        {{
          'This only changes what this {computer} shows: nothing is deleted or changed on {name}.'
            | t: { computer: computer(), name: name() }
        }}
      </p>
      <fieldset class="modes" [disabled]="!choice() || busy() !== null">
        <legend class="visually-hidden">{{ 'Sessions shown here' | t }}</legend>
        <label class="mode">
          <input
            type="radio"
            name="sessions-shown"
            value="all"
            [checked]="mode() === 'all'"
            (click)="pickMode($event, 'all')"
          />
          <span>
            <span class="label">{{ 'All of {name}’s sessions' | t: { name: name() } }}</span>
            <span class="muted small">{{
              'New sessions there show here too; turn off the ones you don’t want.' | t
            }}</span>
          </span>
        </label>
        <label class="mode">
          <input
            type="radio"
            name="sessions-shown"
            value="chosen"
            [checked]="mode() === 'chosen'"
            (click)="pickMode($event, 'chosen')"
          />
          <span>
            <span class="label">{{ 'Only the ones you choose' | t }}</span>
            <span class="muted small">{{
              'A new session there shows here once you choose it.' | t
            }}</span>
          </span>
        </label>
      </fieldset>
      @if (error()) {
        <p class="notice error small" role="alert">{{ error() }}</p>
      }
      @if (choice(); as c) {
        @if (!c.loaded) {
          <p class="muted small">
            {{
              '{name}’s sessions have not arrived yet: they show once it is online.'
                | t: { name: name() }
            }}
          </p>
        } @else if (c.mode === 'chosen' && !shownCount()) {
          <p class="notice small">
            {{
              'None chosen yet: tick the sessions to show on this {computer}.'
                | t: { computer: computer() }
            }}
          </p>
        }
        <ul class="list">
          @for (s of c.sessions; track s.id) {
            <li>
              <label class="check">
                <input
                  type="checkbox"
                  [checked]="s.synced"
                  [disabled]="busy() !== null"
                  [attr.aria-label]="
                    'Sync {title} to this {computer}'
                      | t: { title: sessionName(s), computer: computer() }
                  "
                  (click)="flip($event, s)"
                />
                <span class="name">{{ sessionName(s) }}</span>
                @if (s.live) {
                  <span class="badge">{{ 'Running' | t }}</span>
                }
              </label>
              <span class="state small">
                @if (s.projectName) {
                  {{ s.projectName }} ·
                }
                @if (s.lastActivityAt) {
                  {{ s.lastActivityAt | relTime }} ·
                }
                {{ s.synced ? ('Shown here' | t) : ('Not synced' | t) }}
              </span>
            </li>
          } @empty {
            @if (c.loaded) {
              <li class="muted small">
                {{ 'No session of {name} is known here yet.' | t: { name: name() } }}
              </li>
            }
          }
        </ul>
        @if (c.sessions.length) {
          <p class="muted small">
            {{ '{n} of {total} show here.' | t: { n: shownCount(), total: c.sessions.length } }}
          </p>
        }
      } @else if (!error()) {
        <p class="muted small">{{ 'Loading…' | t }}</p>
      }
      <div class="row">
        <button type="button" class="btn ghost" (click)="closed.emit()">{{ 'Close' | t }}</button>
      </div>
    </mc-sheet>
  `,
  styles: [
    `
      .modes {
        border: 0;
        margin: 0.5rem 0;
        padding: 0;
        display: grid;
        gap: 0.35rem;
      }
      .mode {
        display: flex;
        align-items: flex-start;
        gap: 0.5rem;
        min-height: var(--tap);
        padding: 0.35rem 0;
      }
      .mode > span {
        display: flex;
        flex-direction: column;
        gap: 0.1rem;
      }
      .mode .label {
        font-weight: 600;
      }
      .list {
        list-style: none;
        margin: 0.5rem 0;
        padding: 0;
      }
      .list > li {
        display: flex;
        flex-direction: column;
        gap: 0.15rem;
        padding: 0.4rem 0;
        border-bottom: 1px solid var(--border);
      }
      .list > li:last-child {
        border-bottom: 0;
      }
      .check {
        display: flex;
        align-items: center;
        gap: 0.5rem;
        min-height: var(--tap);
      }
      .name {
        font-weight: 600;
        min-width: 0;
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
      }
      .state {
        padding-left: 1.6rem;
        color: var(--muted);
      }
      .row {
        display: flex;
        flex-wrap: wrap;
        gap: 0.5rem;
      }
    `,
  ],
})
export class SessionsShownSheetComponent {
  readonly open = input(false);
  /** The paired computer. */
  readonly machine = input('');
  /** Its name here. */
  readonly name = input('');
  readonly closed = output<void>();

  private readonly machines = inject(MachinesService);

  protected readonly choice = signal<SessionsChoiceView | null>(null);
  /** What is being saved: `mode`, or a session's id. */
  protected readonly busy = signal<string | null>(null);
  protected readonly error = signal('');
  protected readonly computer = computerNoun;
  protected readonly sessionName = choiceName;

  protected readonly mode = computed<SessionsShownMode>(() => this.choice()?.mode ?? 'all');
  protected readonly shownCount = computed(
    () => this.choice()?.sessions.filter((s) => s.synced).length ?? 0,
  );

  constructor() {
    effect(() => {
      const machine = this.machine();
      if (!this.open() || !machine) return;
      untracked(() => void this.load(machine));
    });
  }

  private async load(machine: string): Promise<void> {
    this.choice.set(null);
    this.error.set('');
    try {
      this.choice.set(await this.machines.sessionsChoice(machine));
    } catch (e) {
      this.error.set(describeError(e).text);
    }
  }

  // A box or a choice shows what is saved: the click itself changes
  // nothing on screen; the server's answer does.

  protected pickMode(e: Event, mode: SessionsShownMode): void {
    e.preventDefault();
    if (this.mode() === mode || this.busy() !== null) return;
    void this.save('mode', () => this.machines.setSessionsMode(this.machine(), mode));
  }

  protected flip(e: Event, s: SessionChoice): void {
    e.preventDefault();
    if (this.busy() !== null) return;
    void this.save(s.id, () => this.machines.setSessionSynced(this.machine(), s.id, !s.synced));
  }

  private async save(what: string, call: () => Promise<SessionsChoiceView>): Promise<void> {
    this.busy.set(what);
    this.error.set('');
    try {
      this.choice.set(await call());
    } catch (e) {
      this.error.set(describeError(e).text);
    } finally {
      this.busy.set(null);
    }
  }
}

/** The line a paired computer's card says about its sessions shown here. */
export function shownLine(mode: SessionsShownMode, hidden: number, shown: number): string {
  if (mode === 'chosen') {
    return tr('Shown here: only the sessions you choose ({n})', { n: shown });
  }
  return hidden
    ? tr('Shown here: all of its sessions but {n}', { n: hidden })
    : tr('Shown here: all of its sessions');
}
