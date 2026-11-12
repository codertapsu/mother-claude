import {
  ChangeDetectionStrategy,
  Component,
  DestroyRef,
  computed,
  effect,
  inject,
  signal,
  untracked,
} from '@angular/core';
import { Router, RouterLink } from '@angular/router';

import { ApiError } from '../../core/api.service';
import { copyClock } from '../../core/copies';
import { CopiesService } from '../../core/copies.service';
import { computerNoun, tr } from '../../core/i18n/i18n';
import { TPipe } from '../../core/i18n/t.pipe';
import { MachineContext } from '../../core/machine-context';
import { moveErrorText, moveText } from '../../core/moves-text';
import { MovesService } from '../../core/moves.service';
import { NoticeService } from '../../core/notice.service';
import { sessionLink } from '../../core/session-links';
import { IconComponent } from '../../shared/icon.component';
import { SheetComponent } from '../../shared/sheet.component';
import { pastClock, sessionName } from '../../shared/util';
import { SessionContext } from './session-context';

/**
 * Under the header of a session that lives on another computer now
 * (docs/PLAN-MACHINES.md §5.7, spikes/machines/UX.md §6.3, §7.4, §7.5): where
 * it lives and since when — this copy is the conversation up to then, read
 * only — with *Open there*, *Bring it back here* (a move to this computer,
 * started on the one it lives on, while that one is online: the header's
 * sheet, which outlives this banner once the session lives here again) and
 * *Take over…* (only while that computer is offline: it goes on here from
 * this computer's copy, after a warning). Another computer's copy of a
 * session that moved says where it lives now and opens it there.
 *
 * **Continue here from the copy of 14:32** (§5.9): when the computer a
 * session lives on is offline and it kept a copy of the session current
 * here, taking it over lands that copy — on this computer's page of a
 * session that moved away, and on that computer's own page of it (it lives
 * there: no *Lives on* line, only the offer).
 */
@Component({
  selector: 'mc-lives-on-banner',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [RouterLink, IconComponent, SheetComponent, TPipe],
  template: `
    @if (!lives() && remoteCopy(); as h) {
      <div class="box">
        <p class="line" role="status">
          <mc-icon name="copy" [size]="15" />
          <span>{{
            'This {computer} keeps a copy of this session, copied at {time}.'
              | t: { computer: computer(), time: clock(h.copiedAt) }
          }}</span>
        </p>
        <div class="acts">
          <button class="btn small primary" type="button" (click)="openTakeOver()">
            {{ 'Continue here from the copy of {time}' | t: { time: clock(h.copiedAt) } }}
          </button>
        </div>
      </div>
    }
    @if (lives()) {
      <div class="box">
        <p class="line" role="status">
          <mc-icon name="monitor" [size]="15" />
          <span>{{ text() }}</span>
        </p>
        @if (offlineText()) {
          <p class="muted small off">{{ offlineText() }}</p>
        }
        @if (openLink() || canBringBack() || canTakeOver()) {
          <div class="acts">
            @if (openLink(); as link) {
              <a class="btn small" [class.primary]="!canTakeOver()" [routerLink]="link">{{
                openLabel()
              }}</a>
            }
            @if (canBringBack()) {
              <!-- The header hosts the sheet: it outlives this banner. -->
              <button class="btn small" type="button" (click)="ctx.computerMove.set('back')">
                {{ 'Bring it back here' | t }}
              </button>
            }
            @if (canTakeOver()) {
              <button class="btn small" type="button" (click)="openTakeOver()">
                @if (heldCopy(); as h) {
                  {{ 'Continue here from the copy of {time}' | t: { time: clock(h.copiedAt) } }}
                } @else {
                  {{ 'Take over…' | t }}
                }
              </button>
            }
          </div>
        }
      </div>
    }

    @if (canTakeOver() || remoteCopy()) {
      <mc-sheet
        [open]="takingOver()"
        [title]="'Take over {title}?' | t: { title: title() }"
        (closed)="takingOver.set(false)"
      >
        @if (heldCopy(); as h) {
          <p>
            {{
              '{name} is offline since {offline}. The session goes on here from the copy this {computer} keeps of it, as of {time}.'
                | t
                  : {
                      name: homeName(),
                      offline: offlineAt(),
                      computer: computer(),
                      time: clock(h.copiedAt),
                    }
            }}
          </p>
          @if (!h.complete) {
            <p class="muted small">
              {{ 'Its last copy did not finish: part of the last turn may be missing.' | t }}
            </p>
          }
          <p>
            {{
              "The code is this {computer}'s project folder as it is: what {name} changed since isn't here."
                | t: { name: homeName(), computer: computer() }
            }}
          </p>
        } @else {
          <p>
            {{
              "{name} is offline since {offline}. The session goes on here from this {computer}'s copy: it has the conversation only up to {since}, when it moved."
                | t
                  : {
                      name: homeName(),
                      offline: offlineAt(),
                      computer: computer(),
                      since: sinceAt(),
                    }
            }}
          </p>
          <p>
            {{
              "The code is this {computer}'s project folder as it is: what {name} changed after {since} isn't here."
                | t: { name: homeName(), computer: computer(), since: sinceAt() }
            }}
          </p>
        }
        <p>
          {{
            'If {name} went on with it meanwhile, both versions are kept side by side when it next moves there.'
              | t: { name: homeName() }
          }}
        </p>
        @if (takeOverError()) {
          <p class="notice error small" role="alert">{{ takeOverError() }}</p>
        }
        <div class="row">
          <button class="btn primary" type="button" [disabled]="busy()" (click)="takeOver()">
            @if (busy()) {
              {{ 'Taking over…' | t }}
            } @else if (heldCopy(); as h) {
              {{ 'Continue here from the copy of {time}' | t: { time: clock(h.copiedAt) } }}
            } @else {
              {{ 'Take over from {time}' | t: { time: sinceAt() } }}
            }
          </button>
          <button class="btn ghost" type="button" (click)="takingOver.set(false)">
            {{ 'Cancel' | t }}
          </button>
        </div>
      </mc-sheet>
    }
  `,
  styles: [
    `
      :host {
        display: block;
      }
      .box {
        padding: 0.45rem max(0.75rem, env(safe-area-inset-right)) 0.55rem
          max(0.75rem, env(safe-area-inset-left));
        background: var(--amber-soft);
        border-bottom: 1px solid var(--amber-border);
        font-size: 0.875rem;
      }
      .line,
      .off,
      .acts {
        max-width: 960px;
        margin: 0 auto;
      }
      .line {
        display: flex;
        align-items: center;
        gap: 0.45rem;
      }
      .line mc-icon {
        flex: 0 0 auto;
        color: var(--amber);
      }
      .off {
        margin-top: 0.15rem;
      }
      .acts {
        display: flex;
        flex-wrap: wrap;
        gap: 0.4rem;
        margin-top: 0.4rem;
      }
      p {
        line-height: 1.5;
      }
    `,
  ],
})
export class LivesOnBannerComponent {
  protected readonly ctx = inject(SessionContext);
  /** Another computer's copy (none: this computer's). */
  protected readonly machine = inject(MachineContext, { optional: true });
  protected readonly moves = inject(MovesService);
  private readonly notices = inject(NoticeService);
  private readonly copies = inject(CopiesService);
  private readonly router = inject(Router);
  /** The copies' release, while the offer may show (its home offline). */
  private release: (() => void) | null = null;

  protected readonly takingOver = signal(false);
  protected readonly busy = signal(false);
  protected readonly takeOverError = signal('');
  protected readonly computer = computerNoun;

  protected readonly lives = computed(() => this.ctx.session()?.livesOn ?? null);
  protected readonly title = computed(() => {
    const s = this.ctx.session();
    return s ? sessionName(s) : '';
  });
  /** It lives on this computer (another computer's copy says so). */
  private readonly livesHere = computed(() => {
    const l = this.lives();
    return !!l && !!this.moves.selfId() && l.machine === this.moves.selfId();
  });
  /** One of this computer's paired computers. */
  private readonly homeIsPeer = computed(() => {
    const l = this.lives();
    return !!l && this.moves.peers().some((p) => p.id === l.machine);
  });
  /** Where it lives, as this computer names it (another computer's page of
   * its own session: that computer). */
  protected readonly homeName = computed(() => {
    const l = this.lives();
    if (!l) return this.machine?.name() ?? '';
    return this.homeIsPeer() ? this.moves.nameOf(l.machine) : l.name;
  });
  private readonly homeOnline = computed(() => {
    const l = this.lives();
    return !!l && this.moves.online(l.machine);
  });
  protected readonly sinceAt = computed(() => pastClock(this.lives()?.since));
  protected readonly offlineAt = computed(() => {
    const l = this.lives();
    const home = l?.machine ?? this.machine?.id ?? null;
    const at = home ? this.moves.offlineSince(home) : null;
    return at ? pastClock(at) : tr('a while');
  });
  /** Another computer's page of its own session, that computer offline:
   * the copy of it this computer keeps (§5.9). */
  protected readonly remoteCopy = computed(() => {
    const m = this.machine;
    if (!m || m.online() || this.lives() || !this.moves.enabled()) return null;
    return this.copies.heldOf(this.ctx.id(), m.id) ?? null;
  });
  /** The copy a take over goes on from, when this computer keeps one. */
  protected readonly heldCopy = computed(() => {
    if (this.remoteCopy()) return this.remoteCopy();
    const l = this.lives();
    return l && this.canTakeOver() ? (this.copies.heldOf(this.ctx.id(), l.machine) ?? null) : null;
  });
  /** The copies are read while an offer may show: its home is offline. */
  private readonly copyMayShow = computed(() => {
    if (!this.moves.enabled()) return false;
    if (this.machine) return !this.machine.online() && !this.lives();
    return this.canTakeOver();
  });

  constructor() {
    effect(() => {
      const need = this.copyMayShow();
      untracked(() => {
        if (need && !this.release) this.release = this.copies.watch();
        if (!need && this.release) {
          this.release();
          this.release = null;
        }
      });
    });
    inject(DestroyRef).onDestroy(() => this.release?.());
  }

  protected clock(ms: number | null | undefined): string {
    return copyClock(ms) || tr('a while ago');
  }

  protected readonly text = computed(() => {
    const l = this.lives();
    if (!l) return '';
    const time = this.sinceAt();
    if (this.machine) {
      const from = this.machine.name();
      return this.livesHere()
        ? tr('Lives on this {computer} since {time} — moved from {from}.', {
            computer: computerNoun(),
            time,
            from,
          })
        : tr('Lives on {name} since {time} — moved from {from}.', {
            name: this.homeName(),
            time,
            from,
          });
    }
    return tr('Lives on {name} since {time} — moved from this {computer}.', {
      name: this.homeName(),
      time,
      computer: computerNoun(),
    });
  });
  protected readonly offlineText = computed(() => {
    const l = this.lives();
    if (!l || this.livesHere() || !this.homeIsPeer() || this.homeOnline()) return '';
    const at = this.moves.offlineSince(l.machine);
    return at
      ? tr('{name} is offline since {time}.', { name: this.homeName(), time: pastClock(at) })
      : tr('{name} is offline.', { name: this.homeName() });
  });

  protected readonly openLink = computed(() => {
    const l = this.lives();
    const id = this.ctx.id();
    if (!l || !id) return null;
    if (this.livesHere()) return sessionLink(id, null);
    return this.homeIsPeer() ? sessionLink(id, l.machine) : null;
  });
  protected readonly openLabel = computed(() =>
    this.livesHere() ? tr('Open it here') : tr('Open there'),
  );
  /** A move back here, started on the computer it lives on. */
  protected readonly canBringBack = computed(
    () =>
      !this.machine &&
      this.moves.enabled() &&
      this.homeIsPeer() &&
      this.homeOnline() &&
      !this.moves.current(this.ctx.id()),
  );
  /** Only while the computer it lives on is offline. */
  protected readonly canTakeOver = computed(
    () => !this.machine && !this.livesHere() && this.homeIsPeer() && !this.homeOnline(),
  );

  protected openTakeOver(): void {
    this.takeOverError.set('');
    this.takingOver.set(true);
  }

  protected async takeOver(): Promise<void> {
    const id = this.ctx.id();
    if (!id || this.busy()) return;
    this.busy.set(true);
    this.takeOverError.set('');
    try {
      // From the copy this computer keeps, when it keeps one.
      const reply = await this.moves.takeOver(id, this.heldCopy() ? { fromCopy: true } : {});
      this.takingOver.set(false);
      if (reply.fromCopy && reply.resumeError) {
        this.notices.show(
          'warning',
          tr('It is on this {computer} now but did not start: {why}', {
            computer: computerNoun(),
            why: reply.resumeError,
          }),
        );
      } else {
        this.notices.success(moveText(reply.warning), {
          title: reply.fromCopy
            ? tr('{title} goes on here', { title: this.title() })
            : tr('{title} lives here again', { title: this.title() }),
        });
      }
      if (this.machine) {
        // Another computer's page: the session lives on this one now.
        void this.router.navigate(sessionLink(id, null));
      } else {
        void this.ctx.refreshDetail();
      }
    } catch (e) {
      this.takeOverError.set(
        e instanceof ApiError && e.code === 'home_online'
          ? tr('{name} is online again: bring it back here instead.', { name: this.homeName() })
          : moveErrorText(e, { source: this.homeName() }),
      );
    } finally {
      this.busy.set(false);
    }
  }
}
