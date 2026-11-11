import {
  ChangeDetectionStrategy,
  Component,
  computed,
  effect,
  inject,
  signal,
  untracked,
} from '@angular/core';

import { DEFAULT_ACCOUNT } from '../../core/accounts';
import { AccountsService } from '../../core/accounts.service';
import { ApiService } from '../../core/api.service';
import {
  deleteErrorText,
  desktopOs,
  normalizeSessionFiles,
  revealLabel,
} from '../../core/delete-data';
import { computerNoun, tr } from '../../core/i18n/i18n';
import { TPipe } from '../../core/i18n/t.pipe';
import { MachineContext } from '../../core/machine-context';
import { KeyName, LaunchOverrides, McpServerStatus } from '../../core/models';
import { anyTarget } from '../../core/move-data';
import { NowService } from '../../core/now.service';
import { PlanUsageService } from '../../core/plan-usage.service';
import { StorageService } from '../../core/storage.service';
import { AccountChipComponent } from '../../shared/account-chip.component';
import { accountKindLabel } from '../../shared/accounts';
import { CapLimitsComponent } from '../../shared/cap-limits.component';
import { capKeys, capRename, capTerminal, capThinking } from '../../shared/caps';
import { ChannelBadgeComponent } from '../../shared/channel-badge.component';
import { ContextMeterComponent } from '../../shared/context-meter.component';
import { CopyButtonComponent } from '../../shared/copy-button.component';
import { CostComponent } from '../../shared/cost.component';
import { IconComponent } from '../../shared/icon.component';
import { LaunchOptionsComponent } from '../../shared/launch-options.component';
import { PlanUsageComponent } from '../../shared/plan-usage.component';
import {
  fastText,
  thinkingEvidence,
  thinkingOffContradicted,
  thinkingSettingText,
  ultracodeUnknownText,
  ultracodeWhen,
} from '../../shared/session-settings';
import { StatusPillComponent } from '../../shared/status-pill.component';
import { TimeAgoComponent } from '../../shared/time-ago.component';
import {
  channelLabel,
  effortLabel,
  effortSourceText,
  formatTokens,
  modeLabel,
  shortPath,
  surfaceLabel,
  ultracodeLabel,
} from '../../shared/util';
import { SessionContext } from './session-context';

const KEYS: { key: KeyName; label: string; aria: string }[] = [
  { key: 'Escape', label: 'Esc', aria: 'Escape' },
  { key: 'Enter', label: '⏎', aria: 'Enter' },
  { key: 'Up', label: '↑', aria: 'Up arrow' },
  { key: 'Down', label: '↓', aria: 'Down arrow' },
  { key: 'Tab', label: 'Tab', aria: 'Tab' },
  { key: '1', label: '1', aria: '1' },
  { key: '2', label: '2', aria: '2' },
  { key: '3', label: '3', aria: '3' },
];

/**
 * Everything known about the session: ids, process, where it runs, model,
 * mode, effort, ultracode, thinking and fast mode (each with how it is
 * known), usage and cost, context, timestamps; the Claude plan's usage;
 * what can be done with it and why not (limits with their fixes); lifecycle
 * (stop / respawn / remove, remove confirmed); continue or fork as an owned
 * session; rename; the terminal view and keys for tmux; the resume command;
 * MCP servers of owned sessions.
 */
@Component({
  selector: 'mc-info-panel',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [
    AccountChipComponent,
    CapLimitsComponent,
    ChannelBadgeComponent,
    ContextMeterComponent,
    CopyButtonComponent,
    CostComponent,
    IconComponent,
    LaunchOptionsComponent,
    PlanUsageComponent,
    StatusPillComponent,
    TimeAgoComponent,
    TPipe,
  ],
  template: `
    @if (ctx.session(); as s) {
      <section class="card">
        <h2 class="first">{{ 'Session' | t }}</h2>
        <dl class="kv">
          <dt>{{ 'Name' | t }}</dt>
          <dd>
            {{ s.title || s.name || '—' }}
            <button class="btn ghost small" type="button" (click)="ctx.openPicker.set('rename')">
              <mc-icon name="edit" [size]="13" /> {{ 'Rename' | t }}
            </button>
            @if (renameMode() === 'alias') {
              <span class="muted small">{{ '(only in Mother Claude)' | t }}</span>
            }
          </dd>
          <dt>{{ 'Status' | t }}</dt>
          <dd>
            <mc-status-pill [status]="s.status" [live]="s.live" [activity]="s.activity" />
            @if (s.waitingFor) {
              <span class="muted small"> · {{ s.waitingFor }}</span>
            }
          </dd>
          <dt>{{ 'Session id' | t }}</dt>
          <dd class="mono">{{ s.id }} <mc-copy-button [text]="s.id" [label]="'Copy' | t" /></dd>
          <dt>{{ 'Project' | t }}</dt>
          <dd>{{ s.projectName }}</dd>
          <dt>{{ 'Folder' | t }}</dt>
          @if (moveOffered()) {
            <dd>
              <span class="mono">{{ short(s.cwd) }}</span>
              <button class="btn ghost small" type="button" (click)="ctx.moveCopy.set('project')">
                <mc-icon name="move" [size]="13" /> {{ 'Move or copy…' | t }}
              </button>
            </dd>
          } @else {
            <dd class="mono">{{ short(s.cwd) }}</dd>
          }
          @if (files(); as f) {
            <dt>{{ 'Transcript' | t }}</dt>
            <dd class="path-dd">
              <span class="mono break" [title]="f.transcript">{{ short(f.transcript) }}</span>
              <span class="path-acts">
                <mc-copy-button
                  [text]="f.transcript"
                  [label]="'Copy path' | t"
                  [ariaLabel]="copyTranscriptLabel()"
                />
                @if (machine; as m) {
                  <span class="muted small">{{
                    'Its files are on {name}: show them there.' | t: { name: m.name() }
                  }}</span>
                } @else if (ctx.isDesktop()) {
                  <button
                    class="btn small"
                    type="button"
                    [disabled]="revealing()"
                    (click)="reveal()"
                  >
                    <mc-icon name="folder" [size]="14" /> {{ revealText() }}
                  </button>
                }
              </span>
              @if (revealError()) {
                <span class="error-text small" role="alert">{{ revealError() }}</span>
              }
            </dd>
            @if (f.folder) {
              <dt>{{ 'Session folder' | t }}</dt>
              <dd class="path-dd">
                <span class="mono break" [title]="f.folder">{{ short(f.folder) }}</span>
                <span class="path-acts">
                  <mc-copy-button
                    [text]="f.folder"
                    [label]="'Copy path' | t"
                    [ariaLabel]="copyFolderLabel()"
                  />
                </span>
              </dd>
            }
          }
          <dt>{{ 'Runs in' | t }}</dt>
          <dd>
            {{ surface(s.surface) | t }}
            <mc-channel-badge
              [surface]="s.surface"
              [control]="s.control"
              [session]="s"
              [compact]="true"
            />
          </dd>
          @if (s.kind) {
            <dt>{{ 'Kind' | t }}</dt>
            <dd>{{ s.kind }}</dd>
          }
          @if (s.entrypoint) {
            <dt>{{ 'Entry point' | t }}</dt>
            <dd class="mono">{{ s.entrypoint }}</dd>
          }
          @if (s.account; as a) {
            <dt>{{ 'Claude account' | t }}</dt>
            <dd>
              <mc-account-chip [account]="a" />
              <span class="muted small">{{ kindLabel(a) | t }}</span>
            </dd>
          }
          <dt>{{ 'Owned' | t }}</dt>
          <dd>
            {{ s.owned ? ('Yes: started or taken over by Mother Claude' | t) : ('No' | t) }}
          </dd>
          @if (s.pid) {
            <dt>{{ 'Process' | t }}</dt>
            <dd class="mono">pid {{ s.pid }}{{ s.live ? '' : ' ' + ('(exited)' | t) }}</dd>
          }
          @if (s.jobId) {
            <dt>{{ 'Background job' | t }}</dt>
            <dd class="mono">{{ s.jobId }}</dd>
          }
          @if (s.tmuxPane) {
            <dt>{{ 'tmux pane' | t }}</dt>
            <dd class="mono">{{ s.tmuxPane }}</dd>
          }
          @if (s.version) {
            <dt>Claude Code</dt>
            <dd>{{ s.version }}</dd>
          }
          @if (s.cliName && s.cliName !== s.name) {
            <dt>{{ 'CLI name' | t }}</dt>
            <dd class="mono">{{ s.cliName }}</dd>
          }
          @if (s.gitBranch) {
            <dt>{{ 'Branch' | t }}</dt>
            <dd class="mono">{{ s.gitBranch }}</dd>
          }
          @if (s.prLink) {
            <dt>{{ 'Pull request' | t }}</dt>
            <dd>
              <a [href]="s.prLink.url" target="_blank" rel="noopener noreferrer"
                >{{ s.prLink.repo ? s.prLink.repo + ' ' : '' }}#{{ s.prLink.number }}</a
              >
            </dd>
          }
          @if (s.remoteControl) {
            <dt>Remote Control</dt>
            <dd>{{ 'On (also reachable from the Claude app)' | t }}</dd>
          }
          <dt>{{ 'Started' | t }}</dt>
          <dd><mc-time [ms]="s.startedAt" />{{ s.startedAt ? '' : '—' }}</dd>
          <dt>{{ 'Last activity' | t }}</dt>
          <dd><mc-time [ms]="s.lastActivityAt" />{{ s.lastActivityAt ? '' : '—' }}</dd>
          <dt>{{ 'Messages' | t }}</dt>
          <dd>{{ s.messageCount }}</dd>
          @if (s.lastError; as err) {
            <dt>{{ 'Last error' | t }}</dt>
            <dd class="error-text">{{ err.kind }}: {{ err.text }} · <mc-time [ms]="err.at" /></dd>
          }
        </dl>
      </section>

      <section class="card">
        <h2 class="first">{{ 'Model and usage' | t }}</h2>
        <dl class="kv">
          <dt>{{ 'Model' | t }}</dt>
          <dd>
            {{ s.model?.label || '—' }} <span class="mono muted small">{{ s.model?.id }}</span>
          </dd>
          <dt>{{ 'Permission mode' | t }}</dt>
          <dd>{{ mode(s.permissionMode) | t }}</dd>
          <dt>{{ 'Effort' | t }}</dt>
          <dd class="how-dd">
            @if (s.effort) {
              {{ effort(s.effort) | t }}
              @if (effortHow()) {
                <span class="muted small"><span class="dot">· </span>{{ effortHow() }}</span>
              }
            } @else if (s.effortSource === 'applied') {
              {{ 'None' | t }}
              <span class="muted small"
                ><span class="dot">· </span>{{ 'this model has no effort levels' | t }}</span
              >
            } @else {
              <span class="muted">{{ 'Not reported' | t }}</span>
            }
          </dd>
          <dt>Ultracode</dt>
          <dd class="how-dd">
            @if (s.ultracode) {
              {{ ultracodeLabel(s.ultracode) | t }}
              @if (ultracodeHow()) {
                <span class="muted small"><span class="dot">· </span>{{ ultracodeHow() }}</span>
              }
            } @else {
              <span class="muted">{{ s.live ? ('Unknown' | t) : ('Not recorded' | t) }}</span>
              @if (ultracodeUnknownHow()) {
                <span class="muted small"
                  ><span class="dot">· </span>{{ ultracodeUnknownHow() }}</span
                >
              }
            }
          </dd>
          <dt>{{ 'Thinking' | t }}</dt>
          <dd class="stack-dd">
            <span>
              @if (s.thinking) {
                {{ thinkingSetting() }}
              } @else if (thinkingSwitchable()) {
                {{ "Claude Code's default" | t }}
                <span class="muted">{{ '(not reported)' | t }}</span>
              } @else {
                <span class="muted">{{ 'Not reported' | t }}</span>
              }
            </span>
            <span class="muted small">{{ thinkingUsed() }}</span>
          </dd>
          <dt>{{ 'Fast mode' | t }}</dt>
          <dd class="how-dd">
            @if (s.fast) {
              {{ 'On' | t }}
              <span class="muted small"><span class="dot">· </span>{{ fastHow() }}</span>
            } @else {
              <span class="muted">{{ 'Not seen' | t }}</span>
            }
          </dd>
          <dt>{{ 'Cost' | t }}</dt>
          <dd>
            <mc-cost [usd]="s.costUsd" [final]="!!s.costFinal" />
            <span class="muted small">{{
              ' ' +
                (s.costFinal
                  ? ('(reported by Claude)' | t)
                  : s.costEstimated
                    ? ('(estimated: some requests ran in another speed tier or had no known price)'
                      | t)
                    : ('(estimated)' | t))
            }}</span>
          </dd>
          <dt>{{ 'Input' | t }}</dt>
          <dd>{{ '{n} tokens' | t: { n: tokens(s.usage.input) } }}</dd>
          <dt>{{ 'Output' | t }}</dt>
          <dd>{{ '{n} tokens' | t: { n: tokens(s.usage.output) } }}</dd>
          <dt>{{ 'Cache reads' | t }}</dt>
          <dd>{{ '{n} tokens' | t: { n: tokens(s.usage.cacheRead) } }}</dd>
          <dt>{{ 'Cache writes' | t }}</dt>
          <dd>{{ '{n} tokens' | t: { n: tokens(s.usage.cacheWrite) } }}</dd>
          @if (s.usagePartial) {
            <dt>{{ 'Totals' | t }}</dt>
            <dd class="muted small">
              {{ 'Still counting: older parts of the transcript are being read.' | t }}
            </dd>
          }
          @if (s.context; as c) {
            <dt>{{ 'Context' | t }}</dt>
            <dd><mc-context-meter class="wide" [context]="c" [compact]="false" /></dd>
          }
        </dl>
      </section>

      @if (!machine && plan.availableOf(planAccount())) {
        <section class="card">
          <h2 class="first">{{ 'Plan usage' | t }}</h2>
          <p class="muted small plan-lede">
            @if (s.account; as a) {
              {{
                'The plan of {name}: every session on this account shares it.' | t: { name: a.name }
              }}
            } @else {
              {{ 'Your Claude plan: every session on this account shares it.' | t }}
            }
          </p>
          <mc-plan-usage [account]="planAccount()" />
        </section>
      }

      <section class="card">
        <h2 class="first">{{ 'From here you can' | t }}</h2>
        <p class="muted small">{{ channel(s.control.channel) }}.</p>
        <mc-cap-limits [session]="s" />
      </section>

      <section class="card">
        <h2 class="first">{{ 'Actions' | t }}</h2>
        <div class="actions">
          @if (terminal()) {
            <button class="btn" type="button" (click)="ctx.terminalOpen.set(true)">
              <mc-icon name="terminal" [size]="15" /> {{ 'Terminal view' | t }}
            </button>
          }
          @if (vscodeUrl(); as url) {
            <a class="btn" [href]="url"
              ><mc-icon name="external" [size]="15" /> {{ 'Open in VS Code' | t }}</a
            >
          }
          @if (s.control.lifecycle.stop) {
            <button class="btn" type="button" (click)="ctx.lifecycle('stop')">
              <mc-icon name="stop" [size]="14" /> {{ 'Stop' | t }}
            </button>
          }
          @if (s.control.lifecycle.respawn) {
            <button class="btn" type="button" (click)="ctx.lifecycle('respawn')">
              <mc-icon name="refresh" [size]="14" /> {{ 'Respawn' | t }}
            </button>
          }
          @if (rmOffered()) {
            <button
              class="btn danger"
              type="button"
              [disabled]="!ctx.canDangerous()"
              (click)="remove()"
            >
              <mc-icon name="trash" [size]="14" />
              {{ confirmRm() ? ('Tap again to remove' | t) : ('Remove' | t) }}
            </button>
          }
        </div>
        @if (rmOffered() && !ctx.canDangerous()) {
          <p class="muted small">
            {{
              'Removing needs the {computer}, or a device allowed to take dangerous actions.'
                | t: { computer: computer() }
            }}
          </p>
        }
        @if (keys() && s.live) {
          <h3>{{ 'Send a key' | t }}</h3>
          <div class="keys" role="group" [attr.aria-label]="'Send a key to the terminal' | t">
            @for (k of keyList; track k.key) {
              <button
                class="btn small"
                type="button"
                [attr.aria-label]="'Send {key}' | t: { key: (k.aria | t) }"
                [disabled]="!ctx.canAct()"
                (click)="ctx.sendKey(k.key)"
              >
                {{ k.label }}
              </button>
            }
          </div>
          <p class="muted small">
            {{ 'For a dialog open in the terminal; the terminal view shows the result.' | t }}
          </p>
        }

        <h3>{{ 'Resume elsewhere' | t }}</h3>
        <div class="copyrow">
          <code class="grow">claude --resume {{ s.id }}</code>
          <mc-copy-button [text]="'claude --resume ' + s.id" [label]="'Copy' | t" />
        </div>
        <div class="copyrow">
          <code class="grow">cd {{ s.cwd }}</code>
          <mc-copy-button [text]="'cd ' + quote(s.cwd)" [label]="'Copy' | t" />
        </div>

        @if (!s.owned && (s.control.fork || s.control.continueOwned)) {
          <h3>{{ s.live ? ('Continue here (fork)' | t) : ('Continue here' | t) }}</h3>
          <p class="muted small">
            {{
              s.live
                ? ('It keeps running where it is; you drive a copy from here, with full control.'
                  | t)
                : ('Resume this conversation as a session Mother Claude controls fully.' | t)
            }}
          </p>
          <details class="copts">
            <summary>{{ 'Launch options (model · effort · mode)' | t }}</summary>
            <mc-launch-options (changed)="overrides = $event" />
          </details>
          <button class="btn primary" type="button" (click)="ctx.continueHere(overrides)">
            {{ s.live ? ('Continue here (fork)' | t) : ('Continue here' | t) }}
          </button>
        }
      </section>

      @if (s.owned) {
        <section class="card">
          <h2 class="first">{{ 'MCP servers' | t }}</h2>
          @if (mcpError()) {
            <p class="muted small">{{ mcpError() }}</p>
          } @else if (mcp(); as list) {
            <ul class="mcp">
              @for (m of list; track m.name) {
                <li>
                  <mc-icon name="plug" [size]="14" />
                  <span class="grow">{{ m.name }}</span>
                  <span class="mstatus" [attr.data-status]="m.status">{{ m.status | t }}</span>
                </li>
              } @empty {
                <li class="muted">{{ 'No MCP servers.' | t }}</li>
              }
            </ul>
          } @else {
            <p class="muted small">{{ 'Loading…' | t }}</p>
          }
        </section>
      }
    }
  `,
  styles: [
    `
      :host {
        display: flex;
        flex-direction: column;
        gap: 0.75rem;
        padding: 0.5rem 0 1rem;
      }
      .card + .card {
        margin-top: 0;
      }
      h2.first {
        margin-top: 0;
      }
      h3 {
        margin: 1rem 0 0.4rem;
        font-size: 0.9rem;
      }
      .kv dd {
        display: flex;
        align-items: center;
        flex-wrap: wrap;
        gap: 0.35rem;
      }
      .kv dd.path-dd {
        display: flex;
        flex-direction: column;
        align-items: flex-start;
        gap: 0.3rem;
        min-width: 0;
      }
      .path-acts {
        display: flex;
        flex-wrap: wrap;
        gap: 0.3rem;
      }
      .kv dd.stack-dd {
        flex-direction: column;
        align-items: flex-start;
        gap: 0.1rem;
      }
      /* Phones: how a value is known goes under it, not after a dot. */
      @media (max-width: 767px) {
        .kv dd.how-dd {
          flex-direction: column;
          align-items: flex-start;
          gap: 0.1rem;
        }
        .how-dd .dot {
          display: none;
        }
      }
      .plan-lede {
        margin: -0.25rem 0 0.85rem;
      }
      .actions,
      .keys {
        display: flex;
        flex-wrap: wrap;
        gap: 0.5rem;
      }
      .keys .btn {
        min-width: 2.75rem;
      }
      .copyrow {
        display: flex;
        align-items: center;
        gap: 0.5rem;
        margin-bottom: 0.35rem;
      }
      .grow {
        flex: 1;
        min-width: 0;
        overflow-wrap: anywhere;
      }
      code.grow {
        padding: 0.35rem 0.5rem;
      }
      .copts {
        margin: 0.4rem 0 0.6rem;
      }
      .copts summary {
        cursor: pointer;
        font-size: 0.85rem;
        color: var(--muted);
        min-height: 32px;
      }
      .mcp {
        list-style: none;
        margin: 0;
        padding: 0;
      }
      .mcp li {
        display: flex;
        align-items: center;
        gap: 0.5rem;
        padding: 0.4rem 0;
        border-top: 1px solid var(--border);
      }
      .mcp li:first-child {
        border-top: 0;
      }
      .mstatus {
        font-size: 0.75rem;
        color: var(--muted);
      }
      .mstatus[data-status='connected'] {
        color: var(--green);
      }
      .mstatus[data-status='failed'] {
        color: var(--red);
      }
    `,
  ],
})
export class InfoPanelComponent {
  protected readonly ctx = inject(SessionContext);
  /** Another computer's session (none: this computer's). */
  protected readonly machine = inject(MachineContext, { optional: true });
  private api = inject(ApiService);
  private clock = inject(NowService);
  private accounts = inject(AccountsService);
  private storage = inject(StorageService);
  protected readonly plan = inject(PlanUsageService);

  protected overrides: LaunchOverrides = {};
  protected readonly keyList = KEYS;
  protected readonly confirmRm = signal(false);
  protected readonly mcp = signal<McpServerStatus[] | null>(null);
  protected readonly mcpError = signal('');
  private rmTimer?: ReturnType<typeof setTimeout>;

  protected readonly renameMode = computed(() => {
    const s = this.ctx.session();
    return capRename(s?.control, s?.owned);
  });
  /** Whose plan this session uses (the Default's when it names none). */
  protected readonly planAccount = computed(
    () => this.ctx.session()?.account?.id ?? DEFAULT_ACCOUNT,
  );
  /** Where its files are on the Mac (docs/ACCOUNTS.md §14.1). */
  protected readonly files = computed(() => normalizeSessionFiles(this.ctx.detail()?.files));
  /** "Show in Finder", "Show in File Explorer", "Show in folder" (the
   * desktop's own window: its user agent names its system). */
  protected readonly revealText = computed(() => tr(revealLabel(desktopOs(navigator.userAgent))));
  /** The desktop's computer, in sentences. */
  protected readonly computer = computerNoun;
  /** What the path copy buttons say to a screen reader. */
  protected readonly copyTranscriptLabel = computed(() => tr("Copy the transcript's path"));
  protected readonly copyFolderLabel = computed(() => tr("Copy the session folder's path"));
  protected readonly revealing = signal(false);
  protected readonly revealError = signal('');
  /** The folder can be moved or copied (the header hosts the sheet): this
   * computer's data only. */
  protected readonly moveOffered = computed(
    () =>
      !!this.ctx.session() &&
      !this.machine &&
      anyTarget(this.accounts.list(), this.storage.list() ?? []),
  );
  /** Removing a background job: this computer's only. */
  protected readonly rmOffered = computed(
    () => !!this.ctx.session()?.control.lifecycle.rm && !this.machine,
  );
  protected readonly kindLabel = accountKindLabel;
  protected readonly terminal = computed(() => {
    const s = this.ctx.session();
    return capTerminal(s?.control, s);
  });
  protected readonly keys = computed(() => {
    const s = this.ctx.session();
    return capKeys(s?.control, s);
  });
  protected readonly vscodeUrl = computed(() => {
    const s = this.ctx.session();
    return s?.surface === 'vscode' && this.ctx.isDesktop() && !this.machine
      ? `vscode://anthropic.claude-code/open?session=${encodeURIComponent(s.id)}`
      : '';
  });

  protected readonly effortHow = computed(() =>
    tr(effortSourceText(this.ctx.session()?.effortSource)),
  );

  /** Desktop: the transcript, selected in the file manager. */
  protected async reveal(): Promise<void> {
    const id = this.ctx.id();
    if (!id || this.revealing()) return;
    this.revealing.set(true);
    this.revealError.set('');
    try {
      await this.api.revealSession(id);
    } catch (e) {
      this.revealError.set(deleteErrorText(e));
    } finally {
      this.revealing.set(false);
    }
  }
  protected readonly ultracodeHow = computed(() => {
    const s = this.ctx.session();
    if (!s) return '';
    if (s.ultracode === 'unavailable') {
      return tr(
        "requested, but this model or workspace can't run it; it comes back on once it can",
      );
    }
    return ultracodeWhen(s, this.clock.now());
  });
  /** Why ultracode isn't known, and when it will be. */
  protected readonly ultracodeUnknownHow = computed(() => {
    const s = this.ctx.session();
    return s ? ultracodeUnknownText(s, this.clock.now()) : '';
  });
  protected readonly thinkingSetting = computed(() => {
    const s = this.ctx.session();
    if (!s) return '';
    // "Off (at launch)" when the session has thought since is stale.
    const setting = thinkingSettingText(s);
    return thinkingOffContradicted(s) ? tr('{setting}, but used since', { setting }) : setting;
  });
  /** An owned session whose thinking nobody set yet: its header offers on
   * or off. */
  protected readonly thinkingSwitchable = computed(() => {
    const s = this.ctx.session();
    return !!s?.live && capThinking(s.control);
  });
  protected readonly fastHow = computed(() => {
    const s = this.ctx.session();
    return s ? fastText(s) : '';
  });
  protected readonly thinkingUsed = computed(() => {
    const s = this.ctx.session();
    return s ? thinkingEvidence(s, this.clock.now()) : '';
  });

  protected readonly short = shortPath;
  protected readonly surface = surfaceLabel;
  protected readonly mode = modeLabel;
  protected readonly effort = effortLabel;
  protected readonly ultracodeLabel = ultracodeLabel;
  protected readonly tokens = formatTokens;
  protected readonly channel = (c: string) =>
    c === 'owned' ? tr('Full control: everything works from here') : tr(channelLabel(c));

  constructor() {
    effect(() => {
      const s = this.ctx.session();
      const owned = !!s?.owned && !!s.live;
      const id = s?.id;
      untracked(() => {
        if (owned && id && this.mcp() === null && !this.mcpError()) void this.loadMcp(id);
      });
    });
  }

  protected quote(path: string): string {
    return /[^\w/.-]/.test(path) ? `'${path.replace(/'/g, `'\\''`)}'` : path;
  }

  protected remove(): void {
    if (!this.confirmRm()) {
      this.confirmRm.set(true);
      clearTimeout(this.rmTimer);
      this.rmTimer = setTimeout(() => this.confirmRm.set(false), 4000);
      return;
    }
    this.confirmRm.set(false);
    void this.ctx.lifecycle('rm');
  }

  private async loadMcp(id: string): Promise<void> {
    try {
      const res = await this.api.getMcp(id);
      this.mcp.set(res.servers ?? []);
    } catch {
      this.mcpError.set(tr('MCP status is not available for this session.'));
    }
  }
}
