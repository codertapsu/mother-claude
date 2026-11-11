import {
  ChangeDetectionStrategy,
  Component,
  DestroyRef,
  computed,
  effect,
  inject,
  input,
  output,
  signal,
  untracked,
} from '@angular/core';
import { Router } from '@angular/router';

import { ApiError } from '../core/api.service';
import { AuthService } from '../core/auth.service';
import { formatBytes } from '../core/data-jobs';
import { computerNoun, tr } from '../core/i18n/i18n';
import { TPipe } from '../core/i18n/t.pipe';
import { MachineScopes } from '../core/machine-scope';
import { grantedByPeer, pathLabel } from '../core/machines';
import { SessionSummary } from '../core/models';
import {
  CodeLanding,
  CodeTransfer,
  MoveCheck,
  MovePreflight,
  SessionMoveBody,
  branchName,
  canCancel,
  canRetry,
  canUndo,
  folderCandidates,
  moveFinal,
  movedSession,
  normalizePreflight,
  sessionInTurn,
} from '../core/moves';
import {
  activeText,
  checkText,
  moveErrorText,
  moveSummary,
  offerLabel,
  passedChecks,
} from '../core/moves-text';
import { MoveEntry, MovesService } from '../core/moves.service';
import { NowService } from '../core/now.service';
import { sessionLink } from '../core/session-links';
import { IconComponent } from './icon.component';
import { MoveProgressComponent } from './move-progress.component';
import { SheetComponent } from './sheet.component';
import { pastClock, sessionName, surfaceLabel } from './util';

/** A computer the session can go on on. */
interface TargetRow {
  id: string;
  name: string;
  online: boolean;
  /** Why it can't be picked now ('' when it can). */
  blocked: string;
  /** The line under its name. */
  detail: string;
}

/** "17 Nov, 14:30" style, for *Possible until …*. */
function untilText(ms: number): string {
  return pastClock(ms, 0);
}

/**
 * *Continue on another computer* (docs/PLAN-MACHINES.md §5.8,
 * spikes/machines/UX.md §7.1–§7.4 and §8.2): the session goes on on the
 * computer picked — the same conversation, with its code — the one it
 * leaves keeping its copy.
 *
 * The other computer is asked first (the preflight, on opening and on every
 * choice that changes the checks): what goes (the conversation's size, the
 * code's branch and changes), where it goes there (*Change…*, or *Choose
 * its folder on …* among that computer's folders when it found none), how
 * the code travels and where it lands, the project's recipe as a request to
 * Claude, *after this turn* or *now* while a turn runs, *Keep driving it
 * from here*, and carrying the folder's trust — a dangerous option, shown
 * only to whoever may take it. Every blocker says what fixes it and what
 * the sheet offers instead (*Use a separate folder (worktree)*, *Continue
 * as a copy* for a session another app runs); the button waits for the
 * checks. Then the move's steps as they happen (*Cancel* until it is being
 * placed, *Close* while it goes on), and how it ended: where it runs now and
 * *Open it there*, *Undo move* until the day it says, or why it stopped and
 * *Try again*.
 *
 * The move is started on the computer the session lives on (`at`, null for
 * this one; another one through its scope). With `to` set to this
 * computer's id it is *Bring back to this {computer}*. `show` opens on a
 * move under way. The desktop and phones alike (one column there).
 */
@Component({
  selector: 'mc-move-session-sheet',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [IconComponent, MoveProgressComponent, SheetComponent, TPipe],
  template: `
    <mc-sheet [open]="open()" [title]="sheetTitle()" (closed)="close()">
      @if (open()) {
        @if (entry(); as e) {
          @let m = e.move;
          <mc-move-progress [move]="m" [words]="words()" />
          @if (finished()) {
            <p
              class="notice small outcome"
              [class.ok]="m.state === 'resumed' || m.state === 'undone'"
              [class.error]="m.state === 'failed'"
              role="status"
            >
              {{ summary() }}
            </p>
          } @else {
            <p
              class="notice small outcome"
              [class.warn]="retryable()"
              role="status"
              aria-live="polite"
            >
              {{ retryable() ? summary() : nowText() }}
            </p>
          }
          @if (error()) {
            <p class="notice error small" role="alert">{{ error() }}</p>
          }
          <div class="row actions">
            @if (cancellable()) {
              <button class="btn" type="button" [disabled]="busy()" (click)="cancel()">
                {{ 'Cancel the move' | t }}
              </button>
            }
            @if (retryable()) {
              <button class="btn primary" type="button" [disabled]="busy()" (click)="retry()">
                <mc-icon name="refresh" [size]="14" /> {{ 'Try again' | t }}
              </button>
            }
            @if (m.state === 'failed') {
              @if (failOffer(); as offer) {
                <button class="btn" type="button" (click)="again(offer)">
                  {{ offerText(offer) }}
                </button>
              }
              <button class="btn primary" type="button" (click)="again(null)">
                {{ 'Try again' | t }}
              </button>
            }
            @if (openLink()) {
              <button
                class="btn"
                type="button"
                [class.primary]="m.keepDriving || !runsHere()"
                (click)="openThere()"
              >
                {{ runsHere() ? ('Open it' | t) : ('Open it there' | t) }}
              </button>
            }
            @if (undoable()) {
              <button
                class="btn"
                type="button"
                [class.danger]="confirmingUndo()"
                [disabled]="busy()"
                (click)="undo()"
              >
                <mc-icon name="undo" [size]="14" />
                {{
                  busy()
                    ? ('Undoing…' | t)
                    : confirmingUndo()
                      ? ('Tap again to undo' | t)
                      : ('Undo move' | t)
                }}
              </button>
            }
            <span class="spacer"></span>
            <button class="btn ghost" type="button" (click)="close()">
              {{ finished() ? ('Close' | t) : ('Close (it goes on)' | t) }}
            </button>
          </div>
          @if (undoable() && m.undoUntil) {
            <p class="muted small until">
              {{ 'Possible until {time}.' | t: { time: until(m.undoUntil) } }}
            </p>
          }
        } @else if (session(); as s) {
          @if (lede()) {
            <p class="lede small">{{ lede() }}</p>
          }

          @if (!fixedTarget()) {
            <h3>{{ 'Computer' | t }}</h3>
            <div class="opts" role="radiogroup" [attr.aria-label]="'Computer' | t">
              @for (t of targets(); track t.id) {
                <button
                  type="button"
                  role="radio"
                  class="opt"
                  [class.on]="t.id === targetId()"
                  [attr.aria-checked]="t.id === targetId()"
                  [disabled]="!!t.blocked || busy()"
                  (click)="pickTarget(t.id)"
                >
                  <span class="odot" [class.online]="t.online" aria-hidden="true"></span>
                  <span class="ot">
                    <strong class="oname">{{ t.name }}</strong>
                    <span class="small" [class.why]="!!t.blocked" [class.muted]="!t.blocked">{{
                      t.blocked || t.detail
                    }}</span>
                  </span>
                  @if (t.id === targetId()) {
                    <mc-icon name="check" [size]="16" />
                  }
                </button>
              } @empty {
                <p class="muted small">{{ 'No other computer is paired.' | t }}</p>
              }
            </div>
          }

          @if (preError()) {
            <p class="notice error small" role="alert">{{ preError() }}</p>
          }
          @if (!pre() && checking()) {
            <p class="muted small checking" role="status">
              <span class="spin" aria-hidden="true"></span>
              {{ 'Checking with {name}…' | t: { name: targetName() } }}
            </p>
          }

          @if (pre(); as p) {
            <h3>{{ bringBack() ? ('What comes back' | t) : ('What goes' | t) }}</h3>
            <dl class="kv what">
              <dt>{{ 'Conversation' | t }}</dt>
              <dd>{{ conversationLine() }}</dd>
              <dt>{{ 'Code||source' | t }}</dt>
              <dd>{{ codeLine() }}</dd>
              @if (ignoredLine(); as files) {
                <dt>{{ 'Not in git' | t }}</dt>
                <dd class="ignored">
                  <span class="mono break">{{ files }}</span>
                  @if (mayDangerous()) {
                    <label class="check">
                      <input
                        type="checkbox"
                        [checked]="carryIgnored()"
                        [disabled]="busy()"
                        (change)="carryIgnored.set($any($event.target).checked)"
                      />
                      <span>
                        {{ 'Carry them' | t }}
                        <span class="muted small hint">{{
                          'Files git ignores that the project lists. They usually hold secrets.' | t
                        }}</span>
                      </span>
                    </label>
                  } @else {
                    <span class="muted small">{{
                      'They stay where they are: carrying them needs the dangerous permission.' | t
                    }}</span>
                  }
                </dd>
              }
            </dl>
            @if (bringBack()) {
              <p class="muted small">
                {{ 'Nothing here is lost: only what happened there is added.' | t }}
              </p>
            }

            <dl class="kv how">
              @if (folderThere(); as f) {
                <dt>{{ 'Where it goes' | t }}</dt>
                <dd class="where">
                  <span class="mono break">{{ f }}</span>
                  @if (applyValue() === 'worktree' && p.code.repo && codeValue() !== 'none') {
                    <span class="muted small">{{
                      "(a worktree of it, in Mother Claude's data folder)" | t
                    }}</span>
                  }
                  @if (!choosing()) {
                    <button
                      class="btn ghost small"
                      type="button"
                      [disabled]="busy()"
                      (click)="startChoosing(null)"
                    >
                      {{ 'Change…' | t }}
                    </button>
                  }
                </dd>
              }
              @if (p.code.repo) {
                <dt>
                  <label for="mv-code">{{ 'Code travels' | t }}</label>
                </dt>
                <dd>
                  <select
                    id="mv-code"
                    [value]="codeValue()"
                    [disabled]="busy()"
                    (change)="setCode($any($event.target).value)"
                  >
                    <option value="bundle">{{ 'Machine to machine' | t }}</option>
                    <option value="remote">{{ 'Through your git remote' | t }}</option>
                    <option value="none">{{ 'Only the conversation' | t }}</option>
                  </select>
                </dd>
                @if (codeValue() !== 'none') {
                  <dt>
                    <label for="mv-apply">{{ 'Code lands' | t }}</label>
                  </dt>
                  <dd class="lands">
                    <select
                      id="mv-apply"
                      [value]="applyValue()"
                      [disabled]="busy()"
                      (change)="setApply($any($event.target).value)"
                    >
                      <option value="checkout">{{ 'In the project folder' | t }}</option>
                      <option value="worktree">{{ 'In a separate folder (worktree)' | t }}</option>
                    </select>
                    @if (cleanThere()) {
                      <span class="ok small"
                        ><mc-icon name="check" [size]="13" />
                        {{
                          bringBack()
                            ? ('it has no changes of its own here' | t)
                            : ('it has no changes there' | t)
                        }}</span
                      >
                    }
                  </dd>
                }
              }
              @if (p.recipe.length) {
                <dt>{{ 'After the move' | t }}</dt>
                <dd>
                  <label class="check">
                    <input
                      type="checkbox"
                      [checked]="recipe()"
                      [disabled]="busy()"
                      (change)="recipe.set($any($event.target).checked)"
                    />
                    <span>
                      {{ 'On {name}, ask Claude to run:' | t: { name: targetName() } }}
                      <span class="mono">{{ p.recipe.join(' · ') }}</span>
                      <span class="muted small hint">{{
                        'Claude runs them there with its own approvals.' | t
                      }}</span>
                    </span>
                  </label>
                </dd>
              }
              @if (showWhen()) {
                <dt>{{ 'When' | t }}</dt>
                <dd>
                  <div class="whens" role="radiogroup" [attr.aria-label]="'When' | t">
                    <label class="when">
                      <input
                        type="radio"
                        name="mv-when"
                        [checked]="when() === 'idle'"
                        [disabled]="busy()"
                        (change)="when.set('idle')"
                      />
                      <span
                        >{{ 'After this turn' | t }}
                        <span class="muted small">{{ '— it finishes first' | t }}</span></span
                      >
                    </label>
                    <label class="when">
                      <input
                        type="radio"
                        name="mv-when"
                        [checked]="when() === 'now'"
                        [disabled]="busy()"
                        (change)="when.set('now')"
                      />
                      <span
                        >{{ 'Now||switch' | t }}
                        <span class="muted small">{{ '— interrupts the turn' | t }}</span></span
                      >
                    </label>
                  </div>
                </dd>
              }
            </dl>

            @if (choosing()) {
              <div
                class="chooser"
                role="group"
                [attr.aria-label]="'Its folder on {name}' | t: { name: targetName() }"
              >
                <strong class="small">{{
                  'Its folder on {name}' | t: { name: targetName() }
                }}</strong>
                @for (c of candidates(); track c) {
                  <label class="cand">
                    <input
                      type="radio"
                      name="mv-folder"
                      [checked]="folderDraft() === c"
                      (change)="folderDraft.set(c)"
                    />
                    <span class="mono break">{{ c }}</span>
                  </label>
                }
                <label class="field">
                  <span class="field-label">{{
                    'Another folder on {name}' | t: { name: targetName() }
                  }}</span>
                  <input
                    class="mono"
                    autocomplete="off"
                    spellcheck="false"
                    [value]="typedFolder()"
                    (input)="folderDraft.set($any($event.target).value)"
                  />
                </label>
                <div class="row">
                  <button
                    class="btn primary small"
                    type="button"
                    [disabled]="!folderDraft().trim()"
                    (click)="useFolder()"
                  >
                    {{ 'Use this folder' | t }}
                  </button>
                  <button class="btn ghost small" type="button" (click)="choosing.set(false)">
                    {{ 'Cancel' | t }}
                  </button>
                </div>
              </div>
            }

            @if (showKeepDriving()) {
              <label class="check">
                <input
                  type="checkbox"
                  [checked]="keepDriving()"
                  [disabled]="busy()"
                  (change)="keepDriving.set($any($event.target).checked)"
                />
                <span>{{ 'Keep driving it from here' | t }}</span>
              </label>
            }
            @if (showTrust()) {
              <label class="check trust">
                <input
                  type="checkbox"
                  [checked]="trust()"
                  [disabled]="busy()"
                  (change)="trust.set($any($event.target).checked)"
                />
                <span>
                  {{ "Carry this folder's trust to {name}" | t: { name: targetName() } }}
                  <span class="muted small hint">{{
                    'A trusted folder runs its project hooks and MCP servers without asking.' | t
                  }}</span>
                </span>
              </label>
            }

            <h3>{{ 'Checks' | t }}</h3>
            <ul class="checks">
              @for (ok of passed(); track ok) {
                <li class="ok">
                  <mc-icon name="check" [size]="15" />
                  <span>{{ ok }}</span>
                </li>
              }
              @for (b of p.blockers; track $index) {
                <li class="blocker">
                  <mc-icon name="xCircle" [size]="15" />
                  <span class="cbody">
                    <span class="visually-hidden">{{ 'Required' | t }}: </span>
                    {{ checkWords(b).message }}
                    @if (checkWords(b).fix) {
                      <span class="muted small fix">{{ checkWords(b).fix }}</span>
                    }
                    @if (offerText(b.offer); as label) {
                      <button
                        class="btn small offer"
                        type="button"
                        [disabled]="busy() || offerTaken(b)"
                        (click)="takeOffer(b)"
                      >
                        {{ label }}
                      </button>
                    }
                  </span>
                </li>
              }
              @for (w of p.warnings; track $index) {
                <li class="warning">
                  <mc-icon name="alert" [size]="15" />
                  <span class="cbody">
                    <span class="visually-hidden">{{ 'Recommended' | t }}: </span>
                    {{ checkWords(w).message }}
                    @if (checkWords(w).fix) {
                      <span class="muted small fix">{{ checkWords(w).fix }}</span>
                    }
                  </span>
                </li>
              }
            </ul>
            @if (checking()) {
              <p class="muted small checking" role="status">
                <span class="spin" aria-hidden="true"></span>
                {{ 'Checking with {name}…' | t: { name: targetName() } }}
              </p>
            }
          }

          @if (error()) {
            <p class="notice error small" role="alert">{{ error() }}</p>
          }
          <div class="row actions">
            <button class="btn primary" type="button" [disabled]="!canGo()" (click)="go()">
              <mc-icon name="monitor" [size]="14" /> {{ actionLabel() }}
            </button>
            <button class="btn ghost" type="button" (click)="close()">{{ 'Cancel' | t }}</button>
          </div>
        }
      }
    </mc-sheet>
  `,
  styles: [
    `
      h3 {
        margin: 0.9rem 0 0.4rem;
        font-size: 0.9rem;
      }
      .lede {
        margin: 0 0 0.25rem;
        color: var(--muted);
        line-height: 1.6;
      }
      .opts {
        display: flex;
        flex-direction: column;
        gap: 0.4rem;
      }
      .opt {
        appearance: none;
        display: flex;
        align-items: center;
        gap: 0.65rem;
        width: 100%;
        min-height: var(--tap);
        padding: 0.5rem 0.7rem;
        text-align: left;
        font: inherit;
        color: var(--text);
        background: var(--panel-2);
        border: 1px solid var(--border);
        border-radius: var(--radius-sm);
        cursor: pointer;
      }
      .opt.on {
        border-color: var(--accent);
        background: var(--accent-soft);
      }
      .opt:disabled {
        cursor: not-allowed;
        opacity: 0.7;
      }
      .opt mc-icon {
        color: var(--accent-text);
      }
      .odot {
        flex: 0 0 auto;
        width: 0.6rem;
        height: 0.6rem;
        border-radius: 50%;
        border: 1.5px solid var(--faint);
      }
      .odot.online {
        border-color: var(--green);
        background: var(--green);
      }
      .ot {
        flex: 1;
        min-width: 0;
        display: flex;
        flex-direction: column;
        gap: 0.05rem;
      }
      .oname {
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
      }
      .why {
        color: var(--amber);
      }
      .kv {
        margin-top: 0.25rem;
      }
      .kv.how {
        margin-top: 0.75rem;
        align-items: center;
      }
      .where,
      .lands {
        display: flex;
        align-items: center;
        flex-wrap: wrap;
        gap: 0.3rem 0.5rem;
      }
      .ignored {
        display: flex;
        flex-direction: column;
        align-items: flex-start;
        gap: 0.3rem;
      }
      .ok {
        color: var(--green);
      }
      select {
        max-width: 100%;
      }
      .check {
        display: flex;
        align-items: flex-start;
        gap: 0.5rem;
        margin-top: 0.6rem;
        cursor: pointer;
        line-height: 1.45;
      }
      .kv .check {
        margin-top: 0;
      }
      .check input {
        margin-top: 0.2rem;
      }
      .hint {
        display: block;
      }
      .whens {
        display: flex;
        flex-direction: column;
        gap: 0.2rem;
      }
      .when {
        display: flex;
        align-items: center;
        gap: 0.5rem;
        min-height: var(--tap);
        cursor: pointer;
      }
      .chooser {
        display: flex;
        flex-direction: column;
        gap: 0.45rem;
        margin-top: 0.6rem;
        padding: 0.6rem 0.7rem;
        border: 1px solid var(--border);
        border-radius: var(--radius-sm);
        background: var(--panel-2);
      }
      .cand {
        display: flex;
        align-items: center;
        gap: 0.5rem;
        cursor: pointer;
      }
      .checks {
        list-style: none;
        margin: 0;
        padding: 0;
        display: flex;
        flex-direction: column;
        gap: 0.45rem;
        font-size: 0.9rem;
      }
      .checks li {
        display: flex;
        align-items: flex-start;
        gap: 0.5rem;
      }
      .checks li > mc-icon {
        flex: 0 0 auto;
        margin-top: 0.15rem;
      }
      .checks li.ok > mc-icon {
        color: var(--green);
      }
      .checks li.blocker > mc-icon {
        color: var(--red);
      }
      .checks li.warning > mc-icon {
        color: var(--amber);
      }
      .cbody {
        display: flex;
        flex-direction: column;
        align-items: flex-start;
        gap: 0.25rem;
        min-width: 0;
        overflow-wrap: anywhere;
      }
      .checking {
        display: flex;
        align-items: center;
        gap: 0.4rem;
        margin: 0.6rem 0 0;
      }
      .spin {
        display: inline-block;
        width: 0.8rem;
        height: 0.8rem;
        border: 2px solid var(--border-strong);
        border-top-color: var(--accent);
        border-radius: 50%;
        animation: mc-spin 0.8s linear infinite;
      }
      .notice {
        margin: 0.75rem 0 0;
      }
      .outcome {
        margin-top: 0.9rem;
      }
      .actions {
        margin-top: 0.9rem;
      }
      .until {
        margin: 0.4rem 0 0;
      }
      @media (max-width: 480px) {
        .kv.how,
        .kv.what {
          grid-template-columns: 1fr;
          gap: 0.15rem;
        }
        .kv dd {
          margin-bottom: 0.4rem;
        }
      }
    `,
  ],
})
export class MoveSessionSheetComponent {
  private readonly moves = inject(MovesService);
  private readonly auth = inject(AuthService);
  private readonly scopes = inject(MachineScopes);
  private readonly router = inject(Router);
  private readonly clock = inject(NowService);

  readonly open = input(false);
  /** The session, as the computer it lives on shows it. */
  readonly session = input<SessionSummary | null>(null);
  /** The computer the session lives on, where the move starts (null: this
   * one). */
  readonly at = input<string | null>(null);
  /** The one computer it goes to (this computer's id: *Bring back*). */
  readonly to = input<string | null>(null);
  /** A move under way to show. */
  readonly show = input<MoveEntry | null>(null);
  /** *Bring back*: when it left this computer (ms), when known. */
  readonly since = input<number | null>(null);
  readonly closed = output<void>();

  // ── The choices ──
  private readonly picked = signal('');
  protected readonly codeMode = signal<CodeTransfer | null>(null);
  protected readonly apply = signal<CodeLanding | null>(null);
  protected readonly folder = signal<string | null>(null);
  protected readonly asCopy = signal(false);
  protected readonly recipe = signal(false);
  protected readonly trust = signal(false);
  /** Carry the files git ignores that the project lists (with the
   * dangerous right; on by default then). */
  protected readonly carryIgnored = signal(true);
  protected readonly when = signal<'idle' | 'now'>('idle');
  protected readonly keepDriving = signal(true);
  protected readonly choosing = signal(false);
  private readonly chooserCheck = signal<MoveCheck | null>(null);
  protected readonly folderDraft = signal('');

  // ── The checks ──
  protected readonly pre = signal<MovePreflight | null>(null);
  protected readonly checking = signal(false);
  protected readonly preError = signal('');
  private seq = 0;

  // ── The move ──
  private readonly moveKey = signal<{ at: string | null; id: string } | null>(null);
  /** This sheet started the move it shows. */
  private startedHere = false;
  protected readonly busy = signal(false);
  protected readonly error = signal('');
  protected readonly confirmingUndo = signal(false);
  private undoTimer?: ReturnType<typeof setTimeout>;

  protected readonly entry = computed(() => {
    const k = this.moveKey();
    return k ? (this.moves.get(k.at, k.id) ?? null) : null;
  });
  private readonly sessionId = computed(() => this.session()?.id ?? '');
  /** The move's computers in this computer's words (and whether the
   * session is in a turn the move waits for). */
  protected readonly words = computed(() => this.moves.words(sessionInTurn(this.session())));

  /** It comes to this computer. */
  protected readonly bringBack = computed(() => {
    const to = this.to();
    return !!to && to === this.moves.selfId();
  });
  protected readonly fixedTarget = computed(() => !!this.to());

  protected readonly targets = computed<TargetRow[]>(() => {
    const at = this.at();
    const self = this.moves.selfId();
    const to = this.to();
    if (to && to === self) {
      return [
        {
          id: self,
          name: tr('This {computer}', { computer: computerNoun() }),
          online: true,
          blocked: '',
          detail: '',
        },
      ];
    }
    const rows: TargetRow[] = [];
    for (const p of this.moves.peers()) {
      if (p.id === at || p.id === self) continue;
      const online = this.moves.online(p.id);
      const since = online ? null : this.moves.offlineSince(p.id);
      let blocked = '';
      if (!online) {
        blocked = since
          ? tr("Offline since {time} — can't now", { time: pastClock(since) })
          : tr("Offline — can't now");
      } else if (!at && grantedByPeer(p, 'move') === false) {
        blocked = tr("Doesn't let this {computer} move sessions there", {
          computer: computerNoun(),
        });
      }
      // `pathLabel` is English: its words are the Machines page's.
      const path = pathLabel(p.path);
      rows.push({
        id: p.id,
        name: p.name,
        online,
        blocked,
        detail: path ? tr('Online · {path}', { path: tr(path) }) : tr('Online'),
      });
    }
    return to ? rows.filter((r) => r.id === to) : rows;
  });
  /** The pick, else the one computer it goes to, else the first that can
   * take it. */
  protected readonly targetId = computed(() => {
    const rows = this.targets();
    const picked = this.picked();
    if (picked && rows.some((r) => r.id === picked)) return picked;
    const to = this.to();
    if (to) return to;
    return (rows.find((r) => !r.blocked) ?? rows[0])?.id ?? '';
  });
  protected readonly targetName = computed(() => {
    const id = this.targetId();
    if (!id) return tr('the other computer');
    if (id === this.moves.selfId()) return tr('this {computer}', { computer: computerNoun() });
    return this.targets().find((r) => r.id === id)?.name ?? this.moves.nameOf(id);
  });
  /** Where the session lives, in words. */
  private readonly sourceName = computed(() => {
    const at = this.at();
    return at ? this.moves.nameOf(at) : '';
  });

  /** Who may carry a folder's trust or the files git ignores: this device's
   * dangerous right, and — a session on another computer — that computer's
   * Dangerous permission for this one. */
  protected readonly mayDangerous = computed(() => {
    const at = this.at();
    return this.auth.canDangerous() && (!at || this.scopes.context(at).mayDangerous());
  });

  /** What the checks depend on (keep driving, the recipe and when do not). */
  private readonly body = computed<SessionMoveBody | null>(() => {
    const to = this.targetId();
    if (!to) return null;
    const code = this.codeMode();
    const apply = this.apply();
    const folder = this.folder();
    return {
      to,
      ...(code ? { code } : {}),
      ...(apply ? { apply } : {}),
      ...(folder ? { folder } : {}),
      ...(this.asCopy() ? { asCopy: true } : {}),
      ...(this.trust() && this.mayDangerous() ? { trust: true } : {}),
      // The files git ignores travel with the right (the server's default
      // then), unless unticked; without it they stay where they are.
      ...(this.mayDangerous() && !this.carryIgnored() ? { carryIgnored: false } : {}),
    };
  });

  /** The transfer in force: the pick, else the preflight's (the setup's
   * default). */
  protected readonly codeValue = computed<CodeTransfer>(() => {
    const picked = this.codeMode();
    if (picked) return picked;
    const mode = this.pre()?.code.mode;
    return mode === 'remote' || mode === 'none' ? mode : 'bundle';
  });
  protected readonly applyValue = computed<CodeLanding>(() => {
    const picked = this.apply();
    if (picked) return picked;
    return this.pre()?.code.apply === 'worktree' ? 'worktree' : 'checkout';
  });

  protected readonly lede = computed(() => {
    const s = this.session();
    if (!s || this.bringBack()) return '';
    if (this.asCopy()) {
      const app = surfaceLabel(s.surface);
      return tr(
        "{title} runs in {app} — Mother Claude can't stop it. A copy goes on there as a new session; this one goes on here. Or end it in {app} first.",
        { title: sessionName(s), app },
      );
    }
    return this.at()
      ? tr(
          'The session goes on there — the same conversation, with its code. {name} keeps its copy.',
          { name: this.sourceName() },
        )
      : tr(
          'The session goes on there — the same conversation, with its code. This {computer} keeps its copy, and you can keep driving it from here.',
          { computer: computerNoun() },
        );
  });

  protected readonly conversationLine = computed(() => {
    const p = this.pre();
    if (!p) return '';
    const size = formatBytes(p.session.bytes);
    const since = this.since() ?? this.session()?.livesOn?.since;
    if (this.bringBack() && since && this.at()) {
      return tr('{size}: what happened on {name} since {time}', {
        size,
        name: this.sourceName(),
        time: pastClock(since),
      });
    }
    const n = p.session.subagents;
    if (!n) return size;
    return n === 1
      ? tr('{size}, with 1 subagent', { size })
      : tr('{size}, with {n} subagents', { size, n });
  });

  protected readonly codeLine = computed(() => {
    const p = this.pre();
    if (!p) return '';
    if (!p.code.repo) return tr('Not in a git repository: only the conversation goes');
    if (this.codeValue() === 'none') return tr('Not moved: only the conversation goes');
    const branch = branchName(p.code.branch) || tr('no branch (detached)');
    const untracked = p.code.untracked ?? 0;
    const tracked = Math.max(0, (p.code.changed ?? 0) - untracked);
    const parts: string[] = [];
    if (tracked) {
      parts.push(tracked === 1 ? tr('1 changed file') : tr('{n} changed files', { n: tracked }));
    }
    if (untracked) parts.push(untracked === 1 ? tr('1 new') : tr('{n} new', { n: untracked }));
    return `${branch} · ${parts.length ? parts.join(', ') : tr('no changes')}`;
  });

  /** The files git ignores that the project lists (*Not in git*), while the
   * code moves. */
  protected readonly ignoredLine = computed(() => {
    const p = this.pre();
    if (!p || !p.code.repo || this.codeValue() === 'none') return '';
    return (p.code.ignored ?? []).join(' · ');
  });

  /** The project folder there: the one chosen, else the one it found. */
  protected readonly folderThere = computed(
    () => this.folder() ?? this.pre()?.target?.folder?.path ?? '',
  );
  protected readonly cleanThere = computed(() => {
    const c = this.pre()?.target?.code;
    return this.applyValue() === 'checkout' && !!c && c.repo && (c.clean || c.parked);
  });
  protected readonly candidates = computed(() => folderCandidates(this.chooserCheck(), this.pre()));
  /** The folder field holds a typed path, not one of the candidates. */
  protected readonly typedFolder = computed(() => {
    const d = this.folderDraft();
    return this.candidates().includes(d) ? '' : d;
  });

  protected readonly showWhen = computed(() => !this.asCopy() && sessionInTurn(this.session()));
  protected readonly showKeepDriving = computed(() => !this.bringBack() && !this.asCopy());
  /** Carrying the trust: only to whoever may, and while the folder there
   * is not trusted yet. */
  protected readonly showTrust = computed(() => {
    const f = this.pre()?.target?.folder;
    return this.mayDangerous() && (this.trust() || (!!f && !f.trusted));
  });
  protected readonly passed = computed(() => {
    const p = this.pre();
    return p ? passedChecks(p, this.targetName(), this.bringBack()) : [];
  });

  protected readonly canGo = computed(() => {
    const p = this.pre();
    return (
      !!p &&
      p.ok &&
      !this.checking() &&
      !this.busy() &&
      !this.preError() &&
      !this.choosing() &&
      !!this.targetId()
    );
  });
  protected readonly actionLabel = computed(() => {
    const name = this.targetName();
    if (this.busy()) return tr('Starting…');
    if (!this.pre() && this.checking()) return tr('Checking with {name}…', { name });
    if (this.pre() && !this.pre()!.ok) return tr('Fix the checks first');
    if (this.bringBack()) return tr('Bring back here');
    if (this.asCopy()) return tr('Continue as a copy on {name}', { name });
    return tr('Continue on {name}', { name });
  });

  // ── The move under way ──
  protected readonly finished = computed(() => {
    const e = this.entry();
    return !!e && moveFinal(e.move);
  });
  protected readonly summary = computed(() => {
    const e = this.entry();
    return e ? moveSummary(e.move, this.words()) : '';
  });
  protected readonly nowText = computed(() => {
    const e = this.entry();
    return e ? activeText(e.move, this.words()) : '';
  });
  protected readonly cancellable = computed(() => {
    const e = this.entry();
    return !!e && canCancel(e.move);
  });
  protected readonly retryable = computed(() => {
    const e = this.entry();
    return !!e && canRetry(e.move);
  });
  protected readonly undoable = computed(() => {
    const e = this.entry();
    return !!e && canUndo(e.move, this.clock.now());
  });
  protected readonly failOffer = computed(() => {
    const offer = this.entry()?.move.error?.offer;
    return offer && offerLabel(offer, '') ? offer : null;
  });
  /** The computer it runs on now (null: this one), once it runs there. */
  private readonly runsOn = computed<{ machine: string | null } | null>(() => {
    const e = this.entry();
    if (!e || e.move.state !== 'resumed') return null;
    const self = this.moves.selfId();
    const where = e.move.role === 'out' ? e.move.to.id : (e.at ?? self);
    if (where === self) return { machine: null };
    return this.moves.peers().some((p) => p.id === where) ? { machine: where } : null;
  });
  protected readonly runsHere = computed(() => this.runsOn()?.machine === null);
  protected readonly openLink = computed(() => {
    const r = this.runsOn();
    const e = this.entry();
    return r && e ? sessionLink(movedSession(e.move), r.machine) : null;
  });

  protected readonly sheetTitle = computed(() => {
    const e = this.entry();
    if (this.bringBack() || (e && e.move.to.id === this.moves.selfId())) {
      return tr('Bring back to this {computer}', { computer: computerNoun() });
    }
    if (e) return tr('Continue on {name}', { name: e.move.to.name });
    return tr('Continue on another computer');
  });

  constructor() {
    // Every opening starts over (or on the move it was asked to show, as it
    // was when it opened).
    effect(() => {
      if (!this.open()) return;
      untracked(() => {
        const show = this.show();
        this.moves.ensureMachines();
        this.reset();
        this.moveKey.set(show ? { at: show.at, id: show.move.id } : null);
        if (show?.at && !moveFinal(show.move)) this.moves.follow(show.at, show.move.id);
      });
    });
    // The checks, on opening and on every choice they depend on (never on
    // the session's own updates).
    effect(() => {
      const open = this.open();
      const body = this.body();
      const id = this.sessionId();
      const asking = !this.moveKey();
      if (!open || !body || !id || !asking) return;
      untracked(() => void this.check(id, body));
    });
    inject(DestroyRef).onDestroy(() => clearTimeout(this.undoTimer));
  }

  private reset(): void {
    this.seq += 1;
    this.picked.set('');
    this.codeMode.set(null);
    this.apply.set(null);
    this.folder.set(null);
    this.asCopy.set(false);
    this.recipe.set(false);
    this.trust.set(false);
    this.carryIgnored.set(true);
    this.when.set('idle');
    this.keepDriving.set(true);
    this.choosing.set(false);
    this.chooserCheck.set(null);
    this.folderDraft.set('');
    this.pre.set(null);
    this.preError.set('');
    this.checking.set(false);
    this.error.set('');
    this.busy.set(false);
    this.confirmingUndo.set(false);
    this.startedHere = false;
  }

  private names(): { source: string; target: string } {
    return { source: this.sourceName(), target: this.targetName() };
  }

  private async check(session: string, body: SessionMoveBody): Promise<void> {
    const seq = ++this.seq;
    this.checking.set(true);
    try {
      const p = await this.moves.preflight(this.at(), session, body);
      if (seq !== this.seq) return;
      this.pre.set(p);
      this.preError.set('');
      // A session another app runs moves only as a copy: say so at once.
      if (p.session.foreign && !this.asCopy() && p.blockers.some((b) => b.offer === 'as-copy')) {
        this.asCopy.set(true);
      }
    } catch (e) {
      if (seq !== this.seq) return;
      this.pre.set(null);
      this.preError.set(moveErrorText(e, this.names()));
    } finally {
      if (seq === this.seq) this.checking.set(false);
    }
  }

  // ─────────────────────────────────────────────────────────── Choices

  protected pickTarget(id: string): void {
    if (id === this.targetId()) return;
    this.picked.set(id);
    // Another computer: its own folders, its own answer.
    this.folder.set(null);
    this.choosing.set(false);
    this.trust.set(false);
    this.pre.set(null);
    this.error.set('');
  }

  protected setCode(v: string): void {
    this.codeMode.set(v === 'remote' || v === 'none' ? v : 'bundle');
  }

  protected setApply(v: string): void {
    this.apply.set(v === 'worktree' ? 'worktree' : 'checkout');
  }

  protected checkWords(c: MoveCheck): { message: string; fix: string } {
    return checkText(c);
  }

  protected offerText(offer: string | null | undefined): string {
    return offerLabel(offer, this.targetName());
  }

  /** The offer was taken already (the next answer is on its way). */
  protected offerTaken(c: MoveCheck): boolean {
    switch (c.offer) {
      case 'worktree':
      case 'worktree-or-reset':
        return this.applyValue() === 'worktree';
      case 'as-copy':
        return this.asCopy();
      default:
        return false;
    }
  }

  protected takeOffer(c: MoveCheck): void {
    switch (c.offer) {
      case 'worktree':
      case 'worktree-or-reset':
        this.apply.set('worktree');
        break;
      case 'as-copy':
        this.asCopy.set(true);
        break;
      case 'choose-folder':
        this.startChoosing(c);
        break;
    }
  }

  protected startChoosing(c: MoveCheck | null): void {
    this.chooserCheck.set(c);
    this.folderDraft.set(this.folderThere() || (folderCandidates(c, this.pre())[0] ?? ''));
    this.choosing.set(true);
  }

  protected useFolder(): void {
    const f = this.folderDraft().trim();
    if (!f) return;
    this.folder.set(f);
    this.choosing.set(false);
  }

  // ─────────────────────────────────────────────────────────── Moving

  protected async go(): Promise<void> {
    const s = this.session();
    const p = this.pre();
    const base = this.body();
    if (!s || !p || !base || !this.canGo()) return;
    const repo = p.code.repo;
    const code = repo ? this.codeValue() : undefined;
    const body: SessionMoveBody = {
      ...base,
      ...(code ? { code } : {}),
      ...(code && code !== 'none' ? { apply: this.applyValue() } : {}),
      keepDriving: this.showKeepDriving() ? this.keepDriving() : false,
      ...(this.recipe() && p.recipe.length ? { recipe: true } : {}),
      ...(this.showWhen() ? { when: this.when() } : {}),
    };
    this.busy.set(true);
    this.error.set('');
    try {
      const m = await this.moves.start(this.at(), s.id, body);
      this.startedHere = true;
      this.moveKey.set({ at: this.at(), id: m.id });
    } catch (e) {
      if (e instanceof ApiError && e.code === 'move_blocked') {
        const again = normalizePreflight(e.extra['preflight']);
        if (again) this.pre.set(again);
      }
      this.error.set(moveErrorText(e, this.names()));
    } finally {
      this.busy.set(false);
    }
  }

  protected async cancel(): Promise<void> {
    await this.act('cancel');
  }

  protected async retry(): Promise<void> {
    await this.act('retry');
  }

  protected undo(): void {
    if (this.busy()) return;
    if (!this.confirmingUndo()) {
      this.confirmingUndo.set(true);
      clearTimeout(this.undoTimer);
      this.undoTimer = setTimeout(() => this.confirmingUndo.set(false), 4000);
      return;
    }
    clearTimeout(this.undoTimer);
    this.confirmingUndo.set(false);
    void this.act('undo');
  }

  private async act(what: 'cancel' | 'retry' | 'undo'): Promise<void> {
    const e = this.entry();
    if (!e || this.busy()) return;
    this.busy.set(true);
    this.error.set('');
    try {
      await this.moves.act(e, what);
    } catch (err) {
      this.error.set(
        moveErrorText(err, {
          source: e.at ? this.moves.nameOf(e.at) : '',
          target: e.move.to.name,
        }),
      );
    } finally {
      this.busy.set(false);
    }
  }

  /** Back to the choices, after a move that stopped (with what it offered
   * taken). */
  protected again(offer: string | null): void {
    this.moveKey.set(null);
    this.startedHere = false;
    this.error.set('');
    this.pre.set(null);
    if (offer === 'worktree' || offer === 'worktree-or-reset') this.apply.set('worktree');
  }

  protected until(ms: number): string {
    return untilText(ms);
  }

  protected openThere(): void {
    const link = this.openLink();
    this.closed.emit();
    if (link) void this.router.navigate(link);
  }

  /** Closing after the move this sheet started went on there: with *Keep
   * driving it from here* (or brought back here) the page becomes the
   * session where it runs now. */
  protected close(): void {
    const e = this.entry();
    const link = this.openLink();
    const follow =
      this.startedHere &&
      !!e &&
      e.move.state === 'resumed' &&
      !e.move.asCopy &&
      !!link &&
      (e.move.keepDriving || this.runsHere());
    this.closed.emit();
    if (follow && link) void this.router.navigate(link, { replaceUrl: true });
  }
}
