import { ChangeDetectionStrategy, Component, computed, input, output } from '@angular/core';

import { tr } from '../../core/i18n/i18n';
import { TPipe } from '../../core/i18n/t.pipe';
import { CheckState, MachineAsk, SetupCheck, askOpen, byUrgency } from '../../core/machines';
import { IconComponent, IconName } from '../../shared/icon.component';
import { askText, checkTexts } from './machines-text';

const MARKS: Record<CheckState, { icon: IconName; tone: string }> = {
  ok: { icon: 'check', tone: 'green' },
  warning: { icon: 'alert', tone: 'amber' },
  blocker: { icon: 'xCircle', tone: 'red' },
  unknown: { icon: 'question', tone: 'faint' },
  na: { icon: 'circle', tone: 'faint' },
};

/**
 * One computer's setup checks (docs/PLAN-MACHINES.md §5.14), blockers
 * first: what is wrong, why it matters, and the way out. On this computer a
 * check's fix runs here (`fix`); `manual` ones are words, except pairing,
 * which opens the pair sheet (`manual`). On another computer a failing check
 * offers *Ask {name} to fix* (`ask`): nothing runs there until someone
 * confirms it there.
 */
@Component({
  selector: 'mc-machine-checks',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [IconComponent, TPipe],
  template: `
    <ul class="checks" [attr.aria-label]="label()">
      @for (c of ordered(); track c.id) {
        @let x = texts(c);
        <li class="check" [attr.data-state]="c.state">
          <span class="mark" [attr.data-tone]="mark(c).tone">
            <mc-icon [name]="mark(c).icon" [size]="16" />
            <span class="visually-hidden">{{ markLabel(c) }}</span>
          </span>
          <div class="cbody">
            <div class="ctitle">
              <strong>{{ x.title }}</strong>
              @if (c.state === 'blocker') {
                <span class="badge danger">{{ 'required' | t }}</span>
              } @else if (c.state === 'warning') {
                <span class="badge warn">{{ 'recommended' | t }}</span>
              }
            </div>
            @if (x.detail) {
              <span class="small">{{ x.detail }}</span>
            }
            @if (c.state !== 'ok' && c.state !== 'na' && x.why) {
              <span class="muted small">{{ x.why }}</span>
            }
            @if (c.state === 'blocker' && c.scope === 'account') {
              <span class="muted small">{{
                'Only its sessions wait for it: Machines works without.' | t
              }}</span>
            }
            @if (c.state === 'blocker' && c.scope === 'tools') {
              <span class="muted small">{{
                'Only the machine tools wait for it: Machines works without.' | t
              }}</span>
            }
            @if (failing(c) && c.fix) {
              <div class="cacts">
                @if (remote()) {
                  @let asked = askFor(c);
                  @if (asked && open(asked)) {
                    <span class="muted small">{{ askedText(asked) }}</span>
                  } @else if (c.fix.kind !== 'manual') {
                    <button
                      type="button"
                      class="btn small"
                      [disabled]="readonly() || busy() === c.id"
                      (click)="ask.emit(c.id)"
                    >
                      {{ 'Ask {name} to fix' | t: { name: machineName() } }}
                    </button>
                  }
                  @if (asked && !open(asked)) {
                    <span class="muted small">{{ askedText(asked) }}</span>
                  }
                  <span class="muted small">{{
                    'On {name}: {fix}' | t: { name: machineName(), fix: x.fix }
                  }}</span>
                } @else if (c.fix.kind === 'manual') {
                  @if (c.id === 'pairing' || c.id === 'relay') {
                    <button
                      type="button"
                      class="btn small"
                      [disabled]="readonly()"
                      (click)="manual.emit(c.id)"
                    >
                      {{ x.fix }}
                    </button>
                  } @else {
                    <span class="muted small">{{ x.fix }}</span>
                  }
                } @else {
                  <button
                    type="button"
                    class="btn small"
                    [class.primary]="c.state === 'blocker'"
                    [disabled]="readonly() || busy() === c.id"
                    (click)="fix.emit(c.id)"
                  >
                    {{ busy() === c.id ? ('Working…' | t) : x.fix }}
                  </button>
                  @if (c.fix.kind === 'admin') {
                    <span class="muted small">{{
                      'Windows asks for an administrator’s OK first.' | t
                    }}</span>
                  } @else if (c.fix.kind === 'settings') {
                    <span class="muted small">{{
                      'Opens the settings page where you switch it.' | t
                    }}</span>
                  }
                }
              </div>
            }
          </div>
        </li>
      } @empty {
        <li class="muted small">{{ 'No checks yet.' | t }}</li>
      }
    </ul>
  `,
  styles: [
    `
      .checks {
        list-style: none;
        margin: 0;
        padding: 0;
        display: flex;
        flex-direction: column;
      }
      .check {
        display: flex;
        gap: 0.6rem;
        padding: 0.55rem 0;
        border-bottom: 1px solid var(--border);
      }
      .check:last-child {
        border-bottom: 0;
      }
      .mark {
        flex: 0 0 auto;
        width: 1.6rem;
        height: 1.6rem;
        border-radius: 50%;
        display: grid;
        place-items: center;
        background: var(--tone-soft, var(--panel-2));
        color: var(--tone, var(--muted));
      }
      .cbody {
        min-width: 0;
        display: flex;
        flex-direction: column;
        gap: 0.15rem;
      }
      .ctitle {
        display: flex;
        flex-wrap: wrap;
        align-items: center;
        gap: 0.4rem;
      }
      .cacts {
        display: flex;
        flex-wrap: wrap;
        align-items: center;
        gap: 0.5rem;
        margin-top: 0.3rem;
      }
    `,
  ],
})
export class CheckListComponent {
  readonly checks = input.required<readonly SetupCheck[]>();
  /** Another computer's checks (its name): offers *Ask {name} to fix*. */
  readonly remote = input(false);
  readonly machineName = input('');
  /** (Another computer's) what this computer asked of it. */
  readonly asks = input<readonly MachineAsk[]>([]);
  /** The check whose fix runs now. */
  readonly busy = input<string | null>(null);
  /** No actions (a phone, or not the moment). */
  readonly readonly = input(false);
  /** Only the checks that fail. */
  readonly failingOnly = input(false);
  readonly label = input('');
  readonly fix = output<string>();
  readonly ask = output<string>();
  readonly manual = output<string>();

  protected readonly ordered = computed(() => {
    const list = byUrgency(this.checks());
    return this.failingOnly() ? list.filter((c) => this.failing(c)) : list;
  });

  protected failing(c: SetupCheck): boolean {
    return c.state === 'blocker' || c.state === 'warning';
  }

  protected mark(c: SetupCheck): { icon: IconName; tone: string } {
    return MARKS[c.state] ?? MARKS.unknown;
  }

  protected markLabel(c: SetupCheck): string {
    switch (c.state) {
      case 'ok':
        return tr('Fine');
      case 'warning':
        return tr('Recommended');
      case 'blocker':
        return tr('Required');
      case 'na':
        return tr('Does not apply');
      default:
        return tr('Cannot tell yet');
    }
  }

  protected texts(c: SetupCheck): ReturnType<typeof checkTexts> {
    return checkTexts(c);
  }

  /** The last fix of `c` this computer asked for. */
  protected askFor(c: SetupCheck): MachineAsk | null {
    return [...this.asks()].reverse().find((a) => a.kind === 'fix' && a.check === c.id) ?? null;
  }

  protected open(a: MachineAsk): boolean {
    return askOpen(a);
  }

  protected askedText(a: MachineAsk): string {
    return askText(a, this.machineName());
  }
}
