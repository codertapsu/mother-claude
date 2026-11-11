import { ChangeDetectionStrategy, Component, computed, inject, signal } from '@angular/core';
import { RouterLink } from '@angular/router';

import { ApiError } from '../../core/api.service';
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
 */
@Component({
  selector: 'mc-lives-on-banner',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [RouterLink, IconComponent, SheetComponent, TPipe],
  template: `
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
                {{ 'Take over…' | t }}
              </button>
            }
          </div>
        }
      </div>

      @if (!machine) {
        <mc-sheet
          [open]="takingOver()"
          [title]="'Take over {title}?' | t: { title: title() }"
          (closed)="takingOver.set(false)"
        >
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
              {{
                busy() ? ('Taking over…' | t) : ('Take over from {time}' | t: { time: sinceAt() })
              }}
            </button>
            <button class="btn ghost" type="button" (click)="takingOver.set(false)">
              {{ 'Cancel' | t }}
            </button>
          </div>
        </mc-sheet>
      }
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
  /** Where it lives, as this computer names it. */
  protected readonly homeName = computed(() => {
    const l = this.lives();
    if (!l) return '';
    return this.homeIsPeer() ? this.moves.nameOf(l.machine) : l.name;
  });
  private readonly homeOnline = computed(() => {
    const l = this.lives();
    return !!l && this.moves.online(l.machine);
  });
  protected readonly sinceAt = computed(() => pastClock(this.lives()?.since));
  protected readonly offlineAt = computed(() => {
    const l = this.lives();
    const at = l ? this.moves.offlineSince(l.machine) : null;
    return at ? pastClock(at) : tr('a while');
  });

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
      const reply = await this.moves.takeOver(id);
      this.takingOver.set(false);
      this.notices.success(moveText(reply.warning), {
        title: tr('{title} lives here again', { title: this.title() }),
      });
      void this.ctx.refreshDetail();
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
