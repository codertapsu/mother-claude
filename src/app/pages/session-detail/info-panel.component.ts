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
  ],
  template: `
    @if (ctx.session(); as s) {
      <section class="card">
        <h2 class="first">Session</h2>
        <dl class="kv">
          <dt>Name</dt>
          <dd>
            {{ s.title || s.name || '—' }}
            <button class="btn ghost small" type="button" (click)="ctx.openPicker.set('rename')">
              <mc-icon name="edit" [size]="13" /> Rename
            </button>
            @if (renameMode() === 'alias') {
              <span class="muted small">(only in Mother Claude)</span>
            }
          </dd>
          <dt>Status</dt>
          <dd>
            <mc-status-pill [status]="s.status" [live]="s.live" />
            @if (s.waitingFor) {
              <span class="muted small"> · {{ s.waitingFor }}</span>
            }
          </dd>
          <dt>Session id</dt>
          <dd class="mono">{{ s.id }} <mc-copy-button [text]="s.id" label="Copy" /></dd>
          <dt>Project</dt>
          <dd>{{ s.projectName }}</dd>
          <dt>Folder</dt>
          @if (moveOffered()) {
            <dd>
              <span class="mono">{{ short(s.cwd) }}</span>
              <button class="btn ghost small" type="button" (click)="ctx.moveCopy.set('project')">
                <mc-icon name="move" [size]="13" /> Move or copy…
              </button>
            </dd>
          } @else {
            <dd class="mono">{{ short(s.cwd) }}</dd>
          }
          <dt>Runs in</dt>
          <dd>
            {{ surface(s.surface) }}
            <mc-channel-badge
              [surface]="s.surface"
              [control]="s.control"
              [session]="s"
              [compact]="true"
            />
          </dd>
          @if (s.kind) {
            <dt>Kind</dt>
            <dd>{{ s.kind }}</dd>
          }
          @if (s.entrypoint) {
            <dt>Entry point</dt>
            <dd class="mono">{{ s.entrypoint }}</dd>
          }
          @if (s.account; as a) {
            <dt>Claude account</dt>
            <dd>
              <mc-account-chip [account]="a" />
              <span class="muted small">{{ kindLabel(a) }}</span>
            </dd>
          }
          <dt>Owned</dt>
          <dd>{{ s.owned ? 'Yes: started or taken over by Mother Claude' : 'No' }}</dd>
          @if (s.pid) {
            <dt>Process</dt>
            <dd class="mono">pid {{ s.pid }}{{ s.live ? '' : ' (exited)' }}</dd>
          }
          @if (s.jobId) {
            <dt>Background job</dt>
            <dd class="mono">{{ s.jobId }}</dd>
          }
          @if (s.tmuxPane) {
            <dt>tmux pane</dt>
            <dd class="mono">{{ s.tmuxPane }}</dd>
          }
          @if (s.version) {
            <dt>Claude Code</dt>
            <dd>{{ s.version }}</dd>
          }
          @if (s.cliName && s.cliName !== s.name) {
            <dt>CLI name</dt>
            <dd class="mono">{{ s.cliName }}</dd>
          }
          @if (s.gitBranch) {
            <dt>Branch</dt>
            <dd class="mono">{{ s.gitBranch }}</dd>
          }
          @if (s.prLink) {
            <dt>Pull request</dt>
            <dd>
              <a [href]="s.prLink.url" target="_blank" rel="noopener noreferrer"
                >{{ s.prLink.repo ? s.prLink.repo + ' ' : '' }}#{{ s.prLink.number }}</a
              >
            </dd>
          }
          @if (s.remoteControl) {
            <dt>Remote Control</dt>
            <dd>On (also reachable from the Claude app)</dd>
          }
          <dt>Started</dt>
          <dd><mc-time [ms]="s.startedAt" />{{ s.startedAt ? '' : '—' }}</dd>
          <dt>Last activity</dt>
          <dd><mc-time [ms]="s.lastActivityAt" />{{ s.lastActivityAt ? '' : '—' }}</dd>
          <dt>Messages</dt>
          <dd>{{ s.messageCount }}</dd>
          @if (s.lastError; as err) {
            <dt>Last error</dt>
            <dd class="error-text">{{ err.kind }}: {{ err.text }} · <mc-time [ms]="err.at" /></dd>
          }
        </dl>
      </section>

      <section class="card">
        <h2 class="first">Model and usage</h2>
        <dl class="kv">
          <dt>Model</dt>
          <dd>
            {{ s.model?.label || '—' }} <span class="mono muted small">{{ s.model?.id }}</span>
          </dd>
          <dt>Permission mode</dt>
          <dd>{{ mode(s.permissionMode) }}</dd>
          <dt>Effort</dt>
          <dd class="how-dd">
            @if (s.effort) {
              {{ effort(s.effort) }}
              @if (effortHow()) {
                <span class="muted small"><span class="dot">· </span>{{ effortHow() }}</span>
              }
            } @else if (s.effortSource === 'applied') {
              None
              <span class="muted small"
                ><span class="dot">· </span>this model has no effort levels</span
              >
            } @else {
              <span class="muted">Not reported</span>
            }
          </dd>
          <dt>Ultracode</dt>
          <dd class="how-dd">
            @if (s.ultracode) {
              {{ ultracodeLabel(s.ultracode) }}
              @if (ultracodeHow()) {
                <span class="muted small"><span class="dot">· </span>{{ ultracodeHow() }}</span>
              }
            } @else {
              <span class="muted">{{ s.live ? 'Unknown' : 'Not recorded' }}</span>
              @if (ultracodeUnknownHow()) {
                <span class="muted small"
                  ><span class="dot">· </span>{{ ultracodeUnknownHow() }}</span
                >
              }
            }
          </dd>
          <dt>Thinking</dt>
          <dd class="stack-dd">
            <span>
              @if (s.thinking) {
                {{ thinkingSetting() }}
              } @else if (thinkingSwitchable()) {
                Claude Code's default <span class="muted">(not reported)</span>
              } @else {
                <span class="muted">Not reported</span>
              }
            </span>
            <span class="muted small">{{ thinkingUsed() }}</span>
          </dd>
          <dt>Fast mode</dt>
          <dd class="how-dd">
            @if (s.fast) {
              On
              <span class="muted small"><span class="dot">· </span>{{ fastHow() }}</span>
            } @else {
              <span class="muted">Not seen</span>
            }
          </dd>
          <dt>Cost</dt>
          <dd>
            <mc-cost [usd]="s.costUsd" [final]="!!s.costFinal" />
            <span class="muted small">{{
              s.costFinal
                ? ' (reported by Claude)'
                : s.costEstimated
                  ? ' (estimated: some requests ran in another speed tier or had no known price)'
                  : ' (estimated)'
            }}</span>
          </dd>
          <dt>Input</dt>
          <dd>{{ tokens(s.usage.input) }} tokens</dd>
          <dt>Output</dt>
          <dd>{{ tokens(s.usage.output) }} tokens</dd>
          <dt>Cache reads</dt>
          <dd>{{ tokens(s.usage.cacheRead) }} tokens</dd>
          <dt>Cache writes</dt>
          <dd>{{ tokens(s.usage.cacheWrite) }} tokens</dd>
          @if (s.usagePartial) {
            <dt>Totals</dt>
            <dd class="muted small">
              Still counting: older parts of the transcript are being read.
            </dd>
          }
          @if (s.context; as c) {
            <dt>Context</dt>
            <dd><mc-context-meter class="wide" [context]="c" [compact]="false" /></dd>
          }
        </dl>
      </section>

      @if (plan.availableOf(planAccount())) {
        <section class="card">
          <h2 class="first">Plan usage</h2>
          <p class="muted small plan-lede">
            @if (s.account; as a) {
              The plan of {{ a.name }}: every session on this account shares it.
            } @else {
              Your Claude plan: every session on this account shares it.
            }
          </p>
          <mc-plan-usage [account]="planAccount()" />
        </section>
      }

      <section class="card">
        <h2 class="first">From here you can</h2>
        <p class="muted small">{{ channel(s.control.channel) }}.</p>
        <mc-cap-limits [session]="s" />
      </section>

      <section class="card">
        <h2 class="first">Actions</h2>
        <div class="actions">
          @if (terminal()) {
            <button class="btn" type="button" (click)="ctx.terminalOpen.set(true)">
              <mc-icon name="terminal" [size]="15" /> Terminal view
            </button>
          }
          @if (vscodeUrl(); as url) {
            <a class="btn" [href]="url"><mc-icon name="external" [size]="15" /> Open in VS Code</a>
          }
          @if (s.control.lifecycle.stop) {
            <button class="btn" type="button" (click)="ctx.lifecycle('stop')">
              <mc-icon name="stop" [size]="14" /> Stop
            </button>
          }
          @if (s.control.lifecycle.respawn) {
            <button class="btn" type="button" (click)="ctx.lifecycle('respawn')">
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
        @if (s.control.lifecycle.rm && !ctx.canDangerous()) {
          <p class="muted small">
            Removing needs the Mac, or a device allowed to take dangerous actions.
          </p>
        }
        @if (keys() && s.live) {
          <h3>Send a key</h3>
          <div class="keys" role="group" aria-label="Send a key to the terminal">
            @for (k of keyList; track k.key) {
              <button
                class="btn small"
                type="button"
                [attr.aria-label]="'Send ' + k.aria"
                [disabled]="!ctx.canAct()"
                (click)="ctx.sendKey(k.key)"
              >
                {{ k.label }}
              </button>
            }
          </div>
          <p class="muted small">
            For a dialog open in the terminal; the terminal view shows the result.
          </p>
        }

        <h3>Resume elsewhere</h3>
        <div class="copyrow">
          <code class="grow">claude --resume {{ s.id }}</code>
          <mc-copy-button [text]="'claude --resume ' + s.id" label="Copy" />
        </div>
        <div class="copyrow">
          <code class="grow">cd {{ s.cwd }}</code>
          <mc-copy-button [text]="'cd ' + quote(s.cwd)" label="Copy" />
        </div>

        @if (!s.owned && (s.control.fork || s.control.continueOwned)) {
          <h3>{{ s.live ? 'Continue here (fork)' : 'Continue here' }}</h3>
          <p class="muted small">
            {{
              s.live
                ? 'It keeps running where it is; you drive a copy from here, with full control.'
                : 'Resume this conversation as a session Mother Claude controls fully.'
            }}
          </p>
          <details class="copts">
            <summary>Launch options (model · effort · mode)</summary>
            <mc-launch-options (changed)="overrides = $event" />
          </details>
          <button class="btn primary" type="button" (click)="ctx.continueHere(overrides)">
            {{ s.live ? 'Continue here (fork)' : 'Continue here' }}
          </button>
        }
      </section>

      @if (s.owned) {
        <section class="card">
          <h2 class="first">MCP servers</h2>
          @if (mcpError()) {
            <p class="muted small">{{ mcpError() }}</p>
          } @else if (mcp(); as list) {
            <ul class="mcp">
              @for (m of list; track m.name) {
                <li>
                  <mc-icon name="plug" [size]="14" />
                  <span class="grow">{{ m.name }}</span>
                  <span class="mstatus" [attr.data-status]="m.status">{{ m.status }}</span>
                </li>
              } @empty {
                <li class="muted">No MCP servers.</li>
              }
            </ul>
          } @else {
            <p class="muted small">Loading…</p>
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
  /** The desktop may move or copy the project to another account's
   * directory (the header's dialog). */
  /** The folder can be moved or copied (the header hosts the sheet). */
  protected readonly moveOffered = computed(
    () => !!this.ctx.session() && anyTarget(this.accounts.list(), this.storage.list() ?? []),
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
    return s?.surface === 'vscode' && this.ctx.isDesktop()
      ? `vscode://anthropic.claude-code/open?session=${encodeURIComponent(s.id)}`
      : '';
  });

  protected readonly effortHow = computed(() => effortSourceText(this.ctx.session()?.effortSource));
  protected readonly ultracodeHow = computed(() => {
    const s = this.ctx.session();
    if (!s) return '';
    if (s.ultracode === 'unavailable') {
      return "requested, but this model or workspace can't run it; it comes back on once it can";
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
    const since = thinkingOffContradicted(s) ? ', but used since' : '';
    return `${thinkingSettingText(s)}${since}`;
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
  protected readonly channel = (c: string) => {
    const label = channelLabel(c);
    return label === 'Full control' ? 'Full control: everything works from here' : label;
  };

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
      this.mcpError.set('MCP status is not available for this session.');
    }
  }
}
