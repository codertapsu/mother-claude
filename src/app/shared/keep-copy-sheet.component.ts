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

import { ApiService, describeError } from '../core/api.service';
import { SessionCopiesView, SessionCopyMachine, copyStateText, copyTone } from '../core/copies';
import { computerNoun, tr } from '../core/i18n/i18n';
import { TPipe } from '../core/i18n/t.pipe';
import { NoticeService } from '../core/notice.service';
import { SheetComponent } from './sheet.component';

/**
 * *Keep a copy on …* (docs/PLAN-MACHINES.md §5.9): the computers a session
 * keeps a copy on — refreshed after each of its turns, so moving it there is
 * instant and that computer can go on with it while this one sleeps — each
 * with how its copy stands (*Copied at 14:32*, *Paused: …*). Its own choice,
 * or its project's (Machines › project options); clearing its own goes back
 * to the project's. Read from, and saved on, the computer the session lives
 * on: another computer's session page has that computer's scope, whose REST
 * client goes through its proxy.
 */
@Component({
  selector: 'mc-keep-copy-sheet',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [SheetComponent, TPipe],
  template: `
    <mc-sheet [open]="open()" [title]="'Keep a copy on…' | t" (closed)="closed.emit()">
      <p class="muted small">
        {{
          'A copy is refreshed after each turn of this session, so moving it is instant and the other {computer} can go on with it while this one sleeps. It needs that {computer}’s Move permission.'
            | t: { computer: computer() }
        }}
      </p>
      @if (error()) {
        <p class="notice error small" role="alert">{{ error() }}</p>
      }
      @if (view(); as v) {
        @if (!v.enabled) {
          <p class="notice small">{{ 'Computers is off on this computer.' | t }}</p>
        }
        @if (v.livesOn) {
          <p class="notice small">
            {{ 'It lives on another computer now: its copies are kept from there.' | t }}
          </p>
        }
        <ul class="list">
          @for (m of v.machines; track m.id) {
            <li>
              <label class="check">
                <input
                  type="checkbox"
                  [checked]="chosen().includes(m.id)"
                  [disabled]="busy() || !m.supported || !!v.livesOn"
                  (change)="toggle(m.id, $any($event.target).checked)"
                />
                <span class="name">{{ m.name }}</span>
                @if (m.inProject) {
                  <span class="badge">{{ 'its project' | t }}</span>
                }
              </label>
              <span class="state small" [attr.data-tone]="tone(m)">
                @if (!m.supported) {
                  {{ 'Update Mother Claude on {name} to keep copies there.' | t: { name: m.name } }}
                } @else if (m.on || m.status) {
                  {{ stateText(m) }}
                } @else {
                  {{ 'No copy there.' | t }}
                }
              </span>
            </li>
          } @empty {
            <li class="muted small">{{ 'No computer is paired yet.' | t }}</li>
          }
        </ul>
        @if (v.explicit && v.project.length) {
          <p class="muted small">
            {{ 'This session’s own choice: its project keeps copies elsewhere too.' | t }}
          </p>
        } @else if (!v.explicit && v.project.length) {
          <p class="muted small">{{ 'As its project’s options say.' | t }}</p>
        }
        <div class="row">
          <button
            type="button"
            class="btn primary"
            [disabled]="busy() || !changed() || !!v.livesOn"
            (click)="save()"
          >
            {{ busy() ? ('Saving…' | t) : ('Save' | t) }}
          </button>
          @if (v.explicit && v.project.length) {
            <button type="button" class="btn ghost" [disabled]="busy()" (click)="follow()">
              {{ 'As its project says' | t }}
            </button>
          }
          <button type="button" class="btn ghost" (click)="closed.emit()">
            {{ 'Close' | t }}
          </button>
        </div>
      } @else if (!error()) {
        <p class="muted small">{{ 'Loading…' | t }}</p>
      }
    </mc-sheet>
  `,
  styles: [
    `
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
      }
      .state {
        padding-left: 1.6rem;
        color: var(--muted);
      }
      .state[data-tone='ok'] {
        color: var(--green);
      }
      .state[data-tone='off'] {
        color: var(--amber);
      }
      .row {
        display: flex;
        flex-wrap: wrap;
        gap: 0.5rem;
      }
    `,
  ],
})
export class KeepCopySheetComponent {
  readonly open = input(false);
  /** The session. */
  readonly session = input<string>('');
  readonly closed = output<void>();

  private readonly api = inject(ApiService);
  private readonly notices = inject(NoticeService);

  protected readonly view = signal<SessionCopiesView | null>(null);
  protected readonly chosen = signal<string[]>([]);
  protected readonly busy = signal(false);
  protected readonly error = signal('');
  protected readonly computer = computerNoun;

  /** The choice differs from what the session keeps now. */
  protected readonly changed = computed(() => {
    const v = this.view();
    if (!v) return false;
    const now = v.machines.filter((m) => m.on).map((m) => m.id);
    const c = this.chosen();
    return now.length !== c.length || now.some((id) => !c.includes(id));
  });

  constructor() {
    effect(() => {
      if (!this.open() || !this.session()) return;
      untracked(() => void this.load());
    });
  }

  private async load(): Promise<void> {
    try {
      const v = await this.api.getSessionCopies(this.session());
      this.view.set(v);
      this.chosen.set(v.machines.filter((m) => m.on).map((m) => m.id));
      this.error.set('');
    } catch (e) {
      this.error.set(describeError(e).text);
    }
  }

  protected toggle(id: string, on: boolean): void {
    this.chosen.update((c) =>
      on ? [...c.filter((x) => x !== id), id] : c.filter((x) => x !== id),
    );
  }

  protected stateText(m: SessionCopyMachine): string {
    return copyStateText(m.status);
  }

  protected tone(m: SessionCopyMachine): string {
    return m.on || m.status ? copyTone(m.status) : '';
  }

  protected async save(): Promise<void> {
    await this.send(this.chosen());
  }

  /** Back to its project's choice. */
  protected async follow(): Promise<void> {
    await this.send(null);
  }

  private async send(machines: string[] | null): Promise<void> {
    this.busy.set(true);
    this.error.set('');
    try {
      const v = await this.api.setSessionCopies(this.session(), machines);
      this.view.set(v);
      this.chosen.set(v.machines.filter((m) => m.on).map((m) => m.id));
      const names = v.machines.filter((m) => m.on).map((m) => m.name);
      this.notices.success(
        names.length
          ? tr('A copy is kept on {names}.', { names: names.join(', ') })
          : tr('No copy of this session is kept on another computer.'),
      );
    } catch (e) {
      this.error.set(describeError(e).text);
    } finally {
      this.busy.set(false);
    }
  }
}
