import {
  ChangeDetectionStrategy,
  Component,
  DestroyRef,
  inject,
  input,
  signal,
} from '@angular/core';

import { ApiService, describeError } from '../../core/api.service';
import { computerNoun, tr } from '../../core/i18n/i18n';
import { TPipe } from '../../core/i18n/t.pipe';
import { SyncChanges, SyncKept, SyncPair } from '../../core/machines';
import { SyncService } from '../../core/sync.service';
import { RelativeTimePipe } from '../../shared/time.pipes';
import {
  syncBadge,
  syncChangesText,
  syncDetail,
  syncLastText,
  syncTone,
  thisComputerTitle,
} from './machines-text';

/**
 * Code sync on the Machines page (docs/PLAN-MACHINES.md §5.15): the
 * projects kept the same on this computer and another, how each stands —
 * in sync, syncing, waiting for the other computer's confirmation, paused
 * and why, off — with *Sync now* and *Turn off*. When both computers
 * changed a project, both sides show and the desktop keeps one: the other
 * side's state is first kept under a ref in its repository, then it takes
 * the kept one's. Nothing is merged. A phone sees it read-only.
 */
@Component({
  selector: 'mc-machines-sync',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [RelativeTimePipe, TPipe],
  template: `
    <section id="code-sync" class="card sync" [attr.aria-label]="'Code sync' | t">
      <div class="chead">
        <h2 class="first">{{ 'Code sync' | t }}</h2>
      </div>
      <p class="muted small">
        {{
          'A project’s code kept the same on two computers: what changes on one goes to the other a few seconds later. Turn it on for a project in its options (Machines › Set up › Options).'
            | t
        }}
      </p>
      @if (error()) {
        <p class="notice error small" role="alert">{{ error() }}</p>
      }
      @if (loadError()) {
        <p class="notice error small" role="alert">{{ loadError() }}</p>
      }
      <ul class="list">
        @for (p of sync.pairs(); track p.id) {
          <li [attr.data-state]="p.status.state">
            <div class="row">
              <strong class="name">{{ p.name }}</strong>
              <span class="muted small">{{ 'with {name}' | t: { name: p.machineName } }}</span>
              <span class="spacer"></span>
              <span
                class="badge"
                [class.ok]="tone(p) === 'ok'"
                [class.warn]="tone(p) === 'warn'"
                [class.danger]="tone(p) === 'danger'"
              >
                {{ badge(p) }}
              </span>
            </div>
            <span class="muted small mono paths">{{ p.folder }} ↔ {{ p.there }}</span>
            @if (p.status.state === 'inSync') {
              <span class="small">
                {{ 'Synced {time}' | t: { time: (p.status.at | relTime) } }}
                @if (lastText(p); as l) {
                  · {{ l }}
                }
              </span>
            }
            @if (detail(p); as d) {
              <p class="small detail">{{ d }}</p>
            }

            @if (conflictOf(p); as c) {
              <div class="conflict" role="group" [attr.aria-label]="'Changed on both' | t">
                <dl class="sides">
                  <dt>{{ thisTitle() }}</dt>
                  <dd>{{ changes(c.here) }}</dd>
                  <dt>{{ p.machineName }}</dt>
                  <dd>{{ changes(c.there) }}</dd>
                </dl>
                @if (!readonly()) {
                  <div class="row acts">
                    <button
                      type="button"
                      class="btn small"
                      [class.danger]="armed(p.id + ':here')"
                      [disabled]="busy() === p.id"
                      (click)="resolve(p, 'here')"
                    >
                      {{
                        armed(p.id + ':here')
                          ? ('Tap again to keep this {computer}’s' | t: { computer: computer() })
                          : ('Keep this {computer}’s' | t: { computer: computer() })
                      }}
                    </button>
                    <button
                      type="button"
                      class="btn small"
                      [class.danger]="armed(p.id + ':there')"
                      [disabled]="busy() === p.id"
                      (click)="resolve(p, 'there')"
                    >
                      {{
                        armed(p.id + ':there')
                          ? ('Tap again to keep {name}’s' | t: { name: p.machineName })
                          : ('Keep {name}’s' | t: { name: p.machineName })
                      }}
                    </button>
                  </div>
                }
                <p class="muted small">
                  {{
                    'Nothing is merged. The side you do not keep is first saved in its own repository, under {ref}, with its staged, unstaged and untracked changes and its commits; then its project folder takes the other side’s.'
                      | t: { ref: keptRefOf(p) }
                  }}
                </p>
              </div>
            }

            @if (p.kept.length) {
              <details class="kept small">
                <summary>
                  {{
                    p.kept.length === 1
                      ? ('1 state kept' | t)
                      : ('{n} states kept' | t: { n: p.kept.length })
                  }}
                </summary>
                <ul>
                  @for (k of keptNewestFirst(p); track k.ref) {
                    <li>
                      <span class="mono">{{ k.ref }}</span>
                      <span class="muted"> · {{ keptWhere(p, k) }} · {{ k.at | relTime }}</span>
                    </li>
                  }
                </ul>
              </details>
            }

            @if (!readonly()) {
              <div class="row acts">
                @if (p.status.state !== 'off' && p.status.state !== 'waiting') {
                  <button
                    type="button"
                    class="btn ghost small"
                    [disabled]="busy() === p.id || p.status.state === 'syncing'"
                    (click)="now(p)"
                  >
                    {{ busy() === p.id ? ('Syncing…' | t) : ('Sync now' | t) }}
                  </button>
                }
                @if (p.status.state === 'off') {
                  <button
                    type="button"
                    class="btn small"
                    [disabled]="busy() === p.id"
                    (click)="turnOn(p)"
                  >
                    {{ 'Turn on again' | t }}
                  </button>
                }
                <button
                  type="button"
                  class="btn ghost small"
                  [disabled]="busy() === p.id"
                  (click)="turnOff(p)"
                >
                  {{ p.status.state === 'off' ? ('Remove' | t) : ('Turn off' | t) }}
                </button>
              </div>
            }
          </li>
        } @empty {
          <li class="muted small">{{ 'No project is kept in sync.' | t }}</li>
        }
      </ul>
    </section>
  `,
  styles: [
    `
      .list {
        list-style: none;
        margin: 0.3rem 0;
        padding: 0;
      }
      .list > li {
        display: flex;
        flex-direction: column;
        gap: 0.2rem;
        padding: 0.5rem 0;
        border-bottom: 1px solid var(--border);
      }
      .list > li:last-child {
        border-bottom: 0;
      }
      .row {
        flex-wrap: wrap;
      }
      .paths {
        overflow-wrap: anywhere;
      }
      .detail {
        margin: 0;
      }
      li[data-state='paused'] .detail {
        color: var(--amber);
      }
      .conflict {
        border: 1px solid var(--red-border);
        border-radius: 0.5rem;
        padding: 0.5rem 0.6rem;
        margin: 0.2rem 0;
      }
      .sides {
        display: grid;
        grid-template-columns: max-content 1fr;
        gap: 0.15rem 0.8rem;
        margin: 0 0 0.4rem;
      }
      .sides dt {
        font-weight: 600;
      }
      .sides dd {
        margin: 0;
      }
      .kept ul {
        margin: 0.2rem 0 0;
        padding-left: 1rem;
      }
      .kept .mono {
        overflow-wrap: anywhere;
      }
      .acts {
        margin-top: 0.2rem;
      }
    `,
  ],
})
export class SyncCardComponent {
  /** A phone sees code sync, never changes it. */
  readonly readonly = input(false);

  private readonly api = inject(ApiService);
  protected readonly sync = inject(SyncService);
  protected readonly computer = computerNoun;
  protected readonly thisTitle = thisComputerTitle;
  protected readonly loadError = this.sync.error;
  protected readonly busy = signal<string | null>(null);
  protected readonly error = signal('');
  /** The choice armed by a first tap. */
  private readonly confirming = signal<string | null>(null);
  private confirmTimer?: ReturnType<typeof setTimeout>;

  constructor() {
    const release = this.sync.watch();
    inject(DestroyRef).onDestroy(() => {
      release();
      clearTimeout(this.confirmTimer);
    });
  }

  protected badge(p: SyncPair): string {
    return syncBadge(p);
  }

  protected tone(p: SyncPair): string {
    return syncTone(p);
  }

  protected detail(p: SyncPair): string {
    return syncDetail(p);
  }

  protected lastText(p: SyncPair): string {
    return syncLastText(p);
  }

  protected changes(c: SyncChanges): string {
    return syncChangesText(c);
  }

  /** The conflict to choose in, while it waits for that choice. */
  protected conflictOf(p: SyncPair): SyncPair['conflict'] {
    const s = p.status;
    return s.state === 'paused' && s.reason === 'conflict' ? p.conflict : null;
  }

  /** The ref the losing side's state goes under (its time is the
   * resolution's). */
  protected keptRefOf(p: SyncPair): string {
    return `refs/mother-claude/sync/${p.id}/kept-…`;
  }

  protected keptNewestFirst(p: SyncPair): SyncKept[] {
    return [...p.kept].reverse();
  }

  protected keptWhere(p: SyncPair, k: SyncKept): string {
    return k.side === 'here'
      ? tr('on this {computer}', { computer: computerNoun() })
      : tr('on {name}', { name: p.machineName });
  }

  protected armed(key: string): boolean {
    return this.confirming() === key;
  }

  protected async resolve(p: SyncPair, keep: 'here' | 'there'): Promise<void> {
    const key = `${p.id}:${keep}`;
    if (!this.armed(key)) {
      this.confirming.set(key);
      clearTimeout(this.confirmTimer);
      this.confirmTimer = setTimeout(() => this.confirming.set(null), 4000);
      return;
    }
    this.confirming.set(null);
    await this.withBusy(p.id, async () => {
      this.sync.applyPair(await this.api.resolveSync(p.id, keep));
    });
  }

  protected async now(p: SyncPair): Promise<void> {
    await this.withBusy(p.id, async () => {
      this.sync.applyPair(await this.api.syncNow(p.id));
    });
  }

  protected async turnOn(p: SyncPair): Promise<void> {
    await this.withBusy(p.id, async () => {
      this.sync.apply(await this.api.setSync(p.folder, p.machine, true));
    });
  }

  protected async turnOff(p: SyncPair): Promise<void> {
    await this.withBusy(p.id, async () => {
      this.sync.apply(await this.api.setSync(p.folder, p.machine, false));
    });
  }

  private async withBusy(key: string, fn: () => Promise<void>): Promise<void> {
    this.busy.set(key);
    this.error.set('');
    try {
      await fn();
    } catch (e) {
      this.error.set(describeError(e).text);
    } finally {
      this.busy.set(null);
    }
  }
}
