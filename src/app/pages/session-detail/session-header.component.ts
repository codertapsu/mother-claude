import {
  ChangeDetectionStrategy,
  Component,
  computed,
  effect,
  inject,
  signal,
  untracked,
} from '@angular/core';
import { FormsModule } from '@angular/forms';
import { RouterLink } from '@angular/router';

import { transferTargets } from '../../core/accounts';
import { AccountsService } from '../../core/accounts.service';
import { NoticeService } from '../../core/notice.service';
import { PushService } from '../../core/push.service';
import { RealtimeService, connectionDetail, connectionLabel } from '../../core/realtime.service';
import { SessionPushPref, TransferWhat } from '../../core/models';
import { AccountChipComponent } from '../../shared/account-chip.component';
import { ActivityLineComponent } from '../../shared/activity-line.component';
import { capRename, capTerminal } from '../../shared/caps';
import { ContinueOnSheetComponent, canContinue } from '../../shared/continue-on-sheet.component';
import { CopyButtonComponent } from '../../shared/copy-button.component';
import { IconComponent } from '../../shared/icon.component';
import { MoveCopySheetComponent } from '../../shared/move-copy-sheet.component';
import { SheetComponent } from '../../shared/sheet.component';
import { StatusPillComponent } from '../../shared/status-pill.component';
import { TimeAgoComponent } from '../../shared/time-ago.component';
import { sessionName } from '../../shared/util';
import { SessionContext } from './session-context';

/**
 * The session page's sticky top row: back, name, status, the Claude
 * account (while there are several) and what it is doing right now (with
 * elapsed time), the connection state when it isn't Live, and a ⋯ menu
 * (rename, continue/fork, continue on another account, on the desktop
 * moving or copying it or its project to another account's directory,
 * terminal, notifications, resume command, stop/respawn/remove). 48 px tall
 * on phones.
 */
@Component({
  selector: 'mc-session-header',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [
    FormsModule,
    RouterLink,
    AccountChipComponent,
    ActivityLineComponent,
    ContinueOnSheetComponent,
    CopyButtonComponent,
    IconComponent,
    MoveCopySheetComponent,
    SheetComponent,
    StatusPillComponent,
    TimeAgoComponent,
  ],
  template: `
    @if (ctx.session(); as s) {
      <div class="bar">
        <a class="btn ghost icon-only back" routerLink="/" aria-label="Back to sessions">
          <mc-icon name="back" />
        </a>
        <div class="titles">
          <div class="nameline">
            <h1 class="name">{{ name() }}</h1>
            <mc-status-pill class="pill" [status]="s.status" [live]="s.live" />
            @if (s.account; as a) {
              <mc-account-chip class="acct" [account]="a" />
            }
          </div>
          <mc-activity-line class="act" [activity]="s.activity" />
        </div>
        @if (connection() !== 'open') {
          <button
            type="button"
            class="conn"
            [attr.data-state]="connection()"
            [attr.aria-label]="'Connection: ' + connText()"
            (click)="connSheet.set(true)"
          >
            <span class="conn-dot" aria-hidden="true"></span>
            <span class="conn-text">{{ connText() }}</span>
          </button>
        } @else {
          <span class="conn live" data-state="open" title="Live">
            <span class="conn-dot" aria-hidden="true"></span>
            <span class="visually-hidden">Connection: Live</span>
          </span>
        }
        <button
          class="btn ghost icon-only"
          type="button"
          aria-label="Session menu"
          aria-haspopup="dialog"
          (click)="openMenu()"
        >
          <mc-icon name="moreV" />
        </button>
      </div>

      <mc-sheet [open]="menu()" [title]="name()" (closed)="menu.set(false)">
        <div class="menu">
          <button class="item" type="button" (click)="startRename()">
            <mc-icon name="edit" [size]="18" />
            <span>Rename…</span>
            @if (renameMode() === 'alias') {
              <span class="note">only in Mother Claude</span>
            }
          </button>
          @if (!s.owned && (s.control.fork || s.control.continueOwned)) {
            <button class="item" type="button" (click)="continueHere()">
              <mc-icon name="play" [size]="18" />
              <span>{{ s.live ? 'Continue here (fork)' : 'Continue here' }}</span>
            </button>
          }
          @if (continueOnOffered()) {
            <button class="item" type="button" (click)="openContinueOn()">
              <mc-icon name="user" [size]="18" />
              <span>Continue on another account…</span>
            </button>
          }
          @if (moveOffered()) {
            <button class="item" type="button" (click)="openMoveCopy('session')">
              <mc-icon name="move" [size]="18" />
              <span>Move or copy to another account…</span>
            </button>
            <button class="item" type="button" (click)="openMoveCopy('project')">
              <mc-icon name="folder" [size]="18" />
              <span>Move or copy this project…</span>
            </button>
          }
          @if (terminal()) {
            <button class="item" type="button" (click)="openTerminal()">
              <mc-icon name="terminal" [size]="18" />
              <span>Terminal view</span>
            </button>
          }
          @if (pushAvailable()) {
            <div class="item static">
              <mc-icon [name]="pushPref() === 'mute' ? 'bellOff' : 'bell'" [size]="18" />
              <span>Notifications</span>
              <div class="segmented" role="group" aria-label="Notifications for this session">
                <button
                  type="button"
                  [attr.aria-pressed]="pushPref() === 'follow'"
                  (click)="setPush('follow')"
                >
                  Follow
                </button>
                <button
                  type="button"
                  [attr.aria-pressed]="pushPref() === ''"
                  (click)="setPush(null)"
                >
                  Default
                </button>
                <button
                  type="button"
                  [attr.aria-pressed]="pushPref() === 'mute'"
                  (click)="setPush('mute')"
                >
                  Mute
                </button>
              </div>
            </div>
            @if (s.live && s.status !== 'idle' && s.status !== 'ended') {
              <button class="item" type="button" (click)="toggleTurnFinished()">
                <mc-icon name="bell" [size]="18" />
                <span>{{
                  turnArmed()
                    ? "Don't notify when this turn finishes"
                    : 'Notify me when this turn finishes'
                }}</span>
              </button>
            }
          }
          <div class="item static">
            <mc-icon name="terminal" [size]="18" />
            <span class="mono ellipsis">claude --resume {{ s.id.slice(0, 8) }}…</span>
            <mc-copy-button [text]="'claude --resume ' + s.id" label="Copy" />
          </div>
          @if (vscodeUrl(); as url) {
            <a class="item" [href]="url">
              <mc-icon name="external" [size]="18" />
              <span>Open in VS Code</span>
            </a>
          }
          <button class="item" type="button" (click)="showInfo()">
            <mc-icon name="info" [size]="18" />
            <span>Session info</span>
          </button>
          @if (s.control.lifecycle.stop || s.control.lifecycle.respawn || s.control.lifecycle.rm) {
            <div class="lifecycle">
              @if (s.control.lifecycle.stop) {
                <button class="btn" type="button" (click)="lifecycle('stop')">
                  <mc-icon name="stop" [size]="14" /> Stop
                </button>
              }
              @if (s.control.lifecycle.respawn) {
                <button class="btn" type="button" (click)="lifecycle('respawn')">
                  <mc-icon name="refresh" [size]="14" /> Respawn
                </button>
              }
              @if (s.control.lifecycle.rm) {
                <button
                  class="btn danger"
                  type="button"
                  [disabled]="!ctx.canDangerous()"
                  (click)="remove()"
                >
                  <mc-icon name="trash" [size]="14" />
                  {{ confirmRm() ? 'Tap again to remove' : 'Remove' }}
                </button>
              }
            </div>
          }
        </div>
      </mc-sheet>

      <mc-sheet [open]="renaming()" title="Rename session" (closed)="renaming.set(false)">
        @if (renameMode() === 'none') {
          <p class="muted">This session can't be renamed from here.</p>
        } @else {
          <form class="rename" (submit)="saveRename($event)">
            <label class="field">
              <span class="field-label">Name</span>
              <input
                name="name"
                [ngModel]="renameText()"
                (ngModelChange)="renameText.set($event)"
                maxlength="120"
                autocomplete="off"
              />
            </label>
            @if (renameMode() === 'alias') {
              <p class="muted small">
                Only in Mother Claude: this session belongs to
                {{ s.surface === 'vscode' ? 'VS Code' : 'another program' }}, which keeps its own
                name.
              </p>
            }
            <div class="row">
              <button class="btn primary" type="submit" [disabled]="!renameText().trim()">
                Save
              </button>
              <button class="btn ghost" type="button" (click)="renaming.set(false)">Cancel</button>
            </div>
          </form>
        }
      </mc-sheet>

      <mc-sheet [open]="connSheet()" title="Connection" (closed)="connSheet.set(false)">
        <p>
          <strong>{{ connText() }}</strong> — {{ connDetail() }}
        </p>
        @if (lastData()) {
          <p class="muted small">Updated <mc-time [ms]="lastData()" /></p>
        }
        <button class="btn" type="button" (click)="reconnect()">
          <mc-icon name="refresh" [size]="15" /> Reconnect now
        </button>
      </mc-sheet>

      <mc-continue-on-sheet
        [open]="continueOn()"
        [sessions]="continueOnSessions()"
        [suggested]="continueOnSuggestion()"
        (closed)="continueOn.set(false)"
      />

      @if (moveOffered()) {
        <mc-move-copy-sheet
          [open]="!!ctx.moveCopy()"
          [what]="moveWhat()"
          [source]="s.account"
          [live]="s.live"
          (closed)="ctx.moveCopy.set(null)"
        />
      }
    }
  `,
  styles: [
    `
      :host {
        display: block;
      }
      .bar {
        display: flex;
        align-items: center;
        gap: 0.35rem;
        min-height: 48px;
        padding: 0 0.25rem;
      }
      .titles {
        flex: 1;
        min-width: 0;
        display: flex;
        flex-direction: column;
        justify-content: center;
      }
      .nameline {
        display: flex;
        align-items: center;
        gap: 0.5rem;
        min-width: 0;
      }
      .name {
        margin: 0;
        font-size: 1rem;
        line-height: 1.25;
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
        min-width: 0;
      }
      /* The title gives way first; a long account name ellipses at 45 %. */
      .acct {
        flex: 0 0 auto;
        max-width: 45%;
      }
      .act {
        font-size: 0.75rem;
      }
      .conn {
        appearance: none;
        display: inline-flex;
        align-items: center;
        gap: 0.35rem;
        min-height: var(--tap);
        padding: 0 0.35rem;
        border: 0;
        background: none;
        font: inherit;
        font-size: 0.72rem;
        color: var(--muted);
        white-space: nowrap;
        cursor: pointer;
      }
      .conn.live {
        min-width: 1rem;
        justify-content: center;
        cursor: default;
      }
      .conn-dot {
        width: 0.55rem;
        height: 0.55rem;
        border-radius: 50%;
        background: var(--faint);
      }
      [data-state='open'] .conn-dot {
        background: var(--green);
      }
      [data-state='limited'] .conn-dot {
        background: var(--blue);
      }
      [data-state='connecting'] .conn-dot,
      [data-state='reconnecting'] .conn-dot {
        background: var(--amber);
        color: var(--amber);
        animation: mc-pulse 1.4s ease-out infinite;
      }
      [data-state='offline'] .conn-dot,
      [data-state='unreachable'] .conn-dot,
      [data-state='unpaired'] .conn-dot {
        background: var(--red);
      }
      .menu {
        display: flex;
        flex-direction: column;
      }
      .item {
        appearance: none;
        display: flex;
        align-items: center;
        gap: 0.75rem;
        min-height: var(--tap);
        padding: 0.4rem 0.25rem;
        border: 0;
        border-top: 1px solid var(--border);
        background: none;
        color: var(--text);
        font: inherit;
        text-align: left;
        text-decoration: none;
        cursor: pointer;
        flex-wrap: wrap;
      }
      .item:first-child {
        border-top: 0;
      }
      .item.static {
        cursor: default;
      }
      .item > span {
        flex: 1;
        min-width: 0;
      }
      .item mc-icon {
        color: var(--muted);
      }
      .note {
        flex: 0 0 auto !important;
        font-size: 0.75rem;
        color: var(--faint);
      }
      .lifecycle {
        display: flex;
        flex-wrap: wrap;
        gap: 0.5rem;
        padding-top: 0.75rem;
        border-top: 1px solid var(--border);
      }
      .rename {
        display: flex;
        flex-direction: column;
        gap: 0.75rem;
      }
      @media (max-width: 767px) {
        .pill {
          display: none;
        }
      }
      @media (min-width: 768px) {
        .bar {
          min-height: 60px;
        }
        .name {
          font-size: 1.15rem;
        }
        .act {
          font-size: 0.85rem;
        }
      }
    `,
  ],
})
export class SessionHeaderComponent {
  protected readonly ctx = inject(SessionContext);
  private realtime = inject(RealtimeService);
  private push = inject(PushService);
  private notices = inject(NoticeService);
  private accounts = inject(AccountsService);

  protected readonly menu = signal(false);
  /** The "Continue on another account" choice is open. */
  protected readonly continueOn = signal(false);
  protected readonly renaming = signal(false);
  protected readonly renameText = signal('');
  protected readonly confirmRm = signal(false);
  protected readonly connSheet = signal(false);
  private rmTimer?: ReturnType<typeof setTimeout>;

  protected readonly name = computed(() => {
    const s = this.ctx.session();
    return s ? sessionName(s) : '';
  });
  protected readonly connection = this.realtime.connection;
  protected readonly connText = computed(() => connectionLabel(this.connection()));
  protected readonly connDetail = computed(() => connectionDetail(this.connection()));
  protected readonly lastData = this.realtime.lastDataAt;
  protected readonly renameMode = computed(() => {
    const s = this.ctx.session();
    return capRename(s?.control, s?.owned);
  });
  protected readonly terminal = computed(() => {
    const s = this.ctx.session();
    return capTerminal(s?.control, s);
  });
  protected readonly pushPref = computed<SessionPushPref | ''>(() =>
    this.push.sessionPref(this.ctx.id()),
  );
  /** Push is per device: the desktop app shows everything live instead. */
  protected readonly pushAvailable = computed(() => !this.ctx.isDesktop());
  protected readonly turnArmed = computed(() => this.push.turnFinishedArmed(this.ctx.id()));
  protected readonly vscodeUrl = computed(() => {
    const s = this.ctx.session();
    return s?.surface === 'vscode' && this.ctx.isDesktop()
      ? `vscode://anthropic.claude-code/open?session=${encodeURIComponent(s.id)}`
      : '';
  });
  /** Sessions name an account only while more than the Default exists. */
  protected readonly continueOnOffered = computed(() => {
    const s = this.ctx.session();
    return !!s?.account && canContinue(s);
  });
  protected readonly continueOnSessions = computed(() => {
    const s = this.ctx.session();
    return s ? [s] : [];
  });
  protected readonly continueOnSuggestion = computed(() => {
    const a = this.ctx.session()?.account;
    return a ? (this.accounts.suggestionFor(a)?.id ?? null) : null;
  });
  /** Desktop, several accounts, and one of them in another directory. */
  protected readonly moveOffered = computed(() => {
    const s = this.ctx.session();
    return (
      !!s?.account &&
      this.ctx.isDesktop() &&
      transferTargets(this.accounts.list(), s.account).length > 0
    );
  });
  protected readonly moveWhat = computed<TransferWhat | null>(() => {
    const s = this.ctx.session();
    const kind = this.ctx.moveCopy();
    if (!s || !kind) return null;
    return kind === 'project' ? { what: 'project', cwd: s.cwd } : { what: 'session', id: s.id };
  });

  constructor() {
    // The composer's `/rename` lands here.
    effect(() => {
      if (this.ctx.openPicker() !== 'rename') return;
      untracked(() => {
        this.ctx.openPicker.set(null);
        this.startRename();
      });
    });
  }

  protected openMenu(): void {
    this.menu.set(true);
    if (!this.push.prefs()) void this.push.loadPrefs();
  }

  startRename(): void {
    this.menu.set(false);
    this.renameText.set(this.name());
    this.renaming.set(true);
  }

  protected async saveRename(e: Event): Promise<void> {
    e.preventDefault();
    const name = this.renameText().trim();
    if (!name) return;
    if (await this.ctx.rename(name)) this.renaming.set(false);
  }

  protected continueHere(): void {
    this.menu.set(false);
    void this.ctx.continueHere();
  }

  protected openContinueOn(): void {
    this.menu.set(false);
    this.continueOn.set(true);
  }

  protected openMoveCopy(kind: 'session' | 'project'): void {
    this.menu.set(false);
    this.ctx.moveCopy.set(kind);
  }

  protected openTerminal(): void {
    this.menu.set(false);
    this.ctx.terminalOpen.set(true);
  }

  protected showInfo(): void {
    this.menu.set(false);
    this.ctx.tab.set('info');
  }

  protected async setPush(pref: SessionPushPref | null): Promise<void> {
    try {
      await this.push.setSessionPref(this.ctx.id(), pref);
      this.notices.success(
        pref === 'follow'
          ? 'You will be notified about everything in this session.'
          : pref === 'mute'
            ? 'Notifications from this session are muted.'
            : 'This session uses your notification settings.',
      );
    } catch (e) {
      this.notices.error(e, undefined, 'Notifications');
    }
  }

  protected async toggleTurnFinished(): Promise<void> {
    const on = !this.turnArmed();
    this.menu.set(false);
    try {
      await this.push.setTurnFinished(this.ctx.id(), on);
      this.notices.success(
        on
          ? "You'll get a notification when this turn finishes (even if the session is muted)."
          : 'No notification for this turn.',
      );
    } catch (e) {
      this.notices.error(e, undefined, 'Notifications');
    }
  }

  protected lifecycle(action: 'stop' | 'respawn'): void {
    this.menu.set(false);
    void this.ctx.lifecycle(action);
  }

  protected remove(): void {
    if (!this.confirmRm()) {
      this.confirmRm.set(true);
      clearTimeout(this.rmTimer);
      this.rmTimer = setTimeout(() => this.confirmRm.set(false), 4000);
      return;
    }
    this.confirmRm.set(false);
    this.menu.set(false);
    void this.ctx.lifecycle('rm');
  }

  protected reconnect(): void {
    this.connSheet.set(false);
    this.realtime.reconnectNow();
  }
}
