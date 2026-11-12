import {
  Component,
  ElementRef,
  computed,
  effect,
  inject,
  signal,
  untracked,
  viewChild,
} from '@angular/core';
import { FormsModule } from '@angular/forms';
import { ActivatedRoute } from '@angular/router';

import { DEFAULT_ACCOUNT } from '../../core/accounts';
import { AccountsService } from '../../core/accounts.service';
import {
  MAIN_SERVER,
  MAIN_SERVER_NAME,
  ServerAddress,
  ServerDefaults,
  absorbReply,
  lanBaseOf,
  normalizeServer,
  serverAccount,
  serverAddresses,
  serverErrorText,
  serverPanel,
  serversOf,
  withoutServer,
} from '../../core/api-servers';
import { ApiService } from '../../core/api.service';
import { AuthService } from '../../core/auth.service';
import { plural } from '../../core/data-jobs';
import { computerNoun, tr } from '../../core/i18n/i18n';
import { TPipe } from '../../core/i18n/t.pipe';
import {
  BridgeStatus,
  BridgeThread,
  LaunchDefaults,
  RemoteInfo,
  SessionSummary,
} from '../../core/models';
import { signInLine } from '../../shared/accounts';
import { IconComponent } from '../../shared/icon.component';
import { EFFORT_LABELS, effortLabel, relativeTime, sessionName } from '../../shared/util';
import { API_PERMISSION_MODES } from './permission-modes';
import { ApiServerAddressesComponent } from './server-addresses.component';
import { ApiServerListComponent } from './server-list.component';
import { ApiServerSheetComponent } from './server-sheet.component';
import { ConversationFilesComponent } from './conversation-files.component';

/** New conversation on the first message, rather than joining an existing one. */
const FRESH = '';

/** The panel's "Claude account" line. */
interface ClaudeLine {
  text: string;
  ok: boolean;
  /** The account the server names (absent: the Default's sign-in). */
  account?: string;
  /** That account was removed. */
  gone?: boolean;
  /** That account's name, when it is signed out. */
  signedOut?: string;
}

/**
 * Start, configure and stop the HTTP API.
 *
 * Opening Mother Claude does not publish the API — starting it is a deliberate
 * act, because the port can drive Claude with your account. What you pick here
 * becomes the *defaults* for conversations the API creates; any request can
 * override them.
 *
 * Besides the main server there may be others, each on its own port and,
 * when it names one, running as one Claude account (docs/ACCOUNTS.md §13.5).
 * While there are, the screen lists them and the panel shows the one picked,
 * with its addresses; with only the main one it is the screen it always was,
 * plus a quiet *Add API server*. Phones may do all of it but open a port to
 * tunnels (the server enforces that, and a mode that skips approvals).
 */
@Component({
  selector: 'mc-bridge',
  imports: [
    FormsModule,
    ApiServerAddressesComponent,
    ApiServerListComponent,
    ApiServerSheetComponent,
    ConversationFilesComponent,
    IconComponent,
    TPipe,
  ],
  template: `
    <div class="page-head"><h1>API</h1></div>
    <p class="lede">
      {{
        'A local HTTP API for Claude on this machine — conversations, turns, live streams, tool approvals, images. It is off until you start it.'
          | t
      }}
    </p>

    @if (error(); as e) {
      <div class="card err" role="alert">{{ e }}</div>
    }

    @if (panel(); as s) {
      @if (multi()) {
        <mc-api-server-list
          [servers]="servers()"
          [selected]="panelId()"
          (picked)="pick($event)"
          (add)="adding.set(true)"
        />
      }
      @if (notice(); as n) {
        @if (n.at === 'top') {
          <p class="notice ok small said-top" role="status">{{ n.text }}</p>
        }
      }

      <!-- ── Status ─────────────────────────────────────────────── -->
      <div #panelTop class="card status" [class.picked]="multi()">
        <div class="row head">
          @if (multi()) {
            <strong class="sname">{{ panelName() }}</strong>
          }
          <span class="badge" [class.working]="s.running" [class.stopped]="!s.running">
            {{ s.running ? ('running' | t) : ('stopped' | t) }}
          </span>
          @if (s.running && s.url) {
            <a class="mono url" [href]="s.url + '/docs/'" target="_blank" rel="noreferrer">{{
              s.url
            }}</a>
          } @else if (s.port) {
            <span class="mono muted">{{ 'port {port}' | t: { port: s.port } }}</span>
          }
          <span class="spacer"></span>
          @if (!isMain()) {
            <button class="btn" type="button" [disabled]="busy()" (click)="managing.set(true)">
              <mc-icon name="settings" [size]="15" /> {{ 'Manage' | t }}
            </button>
          }
          @if (s.running) {
            <button class="btn" type="button" [disabled]="busy()" (click)="stop()">
              {{ 'Stop' | t }}
            </button>
          }
        </div>

        <dl class="facts">
          <dt>{{ 'Claude account' | t }}</dt>
          <dd [class.warn]="!claudeLine().ok">{{ claudeLine().text }}</dd>
          <dt>{{ 'Runtime' | t }}</dt>
          <dd>
            {{
              s.hostBuilt ? (s.hostRunning ? ('running' | t) : ('ready' | t)) : ('not built' | t)
            }}
            @if (!s.hostBuilt) {
              <span class="muted small">{{ '— run' | t }} <code>npm run sidecar:build</code></span>
            }
          </dd>
          <dt>{{ 'Authentication' | t }}</dt>
          <dd [class.warn]="s.tokenPolicy === 'never'">{{ authLabel(s) }}</dd>
          @if (s.running) {
            <dt>{{ 'Started' | t }}</dt>
            <dd>{{ rel(s.startedAt) }}</dd>
            @if (isMain()) {
              <dt>{{ 'In flight' | t }}</dt>
              <dd>{{ inFlight(s.activeOperations) }}</dd>
            }
          }
        </dl>

        @if (s.tokenPolicy === 'never') {
          <p class="warn">
            <strong>{{ 'No token is required from anywhere.' | t }}</strong>
            {{ openWarning(s) }}
            {{
              'Dangerous actions stay limited to callers on this {computer}.'
                | t: { computer: computer() }
            }}
            {{ 'Turn off' | t }}
            <em>{{ 'Open to tunnels' | t }}</em>
            {{ 'below and restart to close it.' | t }}
          </p>
        }

        @if (claudeLine().gone) {
          <p class="warn">
            {{
              'Its account was removed, so it cannot start. Choose another account under Defaults (or Automatic), then start it.'
                | t
            }}
          </p>
        } @else if (claudeLine().signedOut; as name) {
          <p class="warn">
            {{
              '{name} is not signed in, so every message would fail. Sign it in on the Accounts page of the desktop app.'
                | t: { name }
            }}
          </p>
        } @else if (!s.claudeAuthenticated && !claudeLine().account) {
          <p class="warn">
            {{ 'Claude itself is not signed in, so every message would fail. Run' | t }}
            <code>claude auth login</code>
            {{ 'in a terminal, then reload this page.' | t }}
          </p>
        }

        @if (s.running && s.url) {
          <div class="links">
            <a class="btn" [href]="s.docsUrl" target="_blank" rel="noreferrer">{{
              'API reference' | t
            }}</a>
            <a class="btn" [href]="s.consoleUrl" target="_blank" rel="noreferrer">{{
              'Console' | t
            }}</a>
          </div>
          <pre class="mono snippet">{{ snippet(s) }}</pre>
        }
      </div>

      @if (multi() && addresses().length) {
        <!-- ── Addresses ──────────────────────────────────────────── -->
        <h2>{{ 'Addresses' | t }}</h2>
        <mc-api-server-addresses [addresses]="addresses()" />
      }

      <!-- ── Defaults ───────────────────────────────────────────── -->
      <h2>{{ s.running ? ('Defaults in use' | t) : ('Defaults' | t) }}</h2>
      <p class="muted">
        {{
          'Applied to conversations the API creates. Every one can be overridden in a request payload. Leave a field on'
            | t
        }}
        <em>{{ 'Default' | t }}</em>
        {{ 'to inherit your own Claude settings.' | t }}
      </p>

      <div class="card form">
        <label class="wide">
          <span class="lbl">{{ 'Working directory' | t }}</span>
          <input
            [ngModel]="cwd()"
            (ngModelChange)="cwd.set($event)"
            [placeholder]="'your home directory' | t"
            spellcheck="false"
          />
          <span class="hint">{{
            'Where conversations run, and which files Claude can reach.' | t
          }}</span>
        </label>

        <div class="row">
          <label>
            <span class="lbl">{{ 'Model' | t }}</span>
            <select [ngModel]="model()" (ngModelChange)="model.set($event)">
              <option value="">
                {{
                  launch()?.model
                    ? ('Default — {value}' | t: { value: launch()!.model })
                    : ('Default' | t)
                }}
              </option>
              @for (m of launch()?.models ?? []; track m.value) {
                <option [value]="m.value" [title]="m.description || ''">{{ m.label }}</option>
              }
            </select>
          </label>

          <label>
            <span class="lbl">{{ 'Effort' | t }}</span>
            <select [ngModel]="effort()" (ngModelChange)="effort.set($event)">
              <option value="">
                {{
                  launch()?.effort
                    ? ('Default — {value}' | t: { value: effortLabel(launch()!.effort!) })
                    : ('Default' | t)
                }}
              </option>
              @for (e of efforts(); track e) {
                <option [value]="e">{{ effortLabel(e) }}</option>
              }
            </select>
          </label>

          <label>
            <span class="lbl">{{ 'Thinking' | t }}</span>
            <select [ngModel]="thinking()" (ngModelChange)="thinking.set($event)">
              <option value="">{{ 'Default' | t }}</option>
              <option value="on">{{ 'On' | t }}</option>
              <option value="off">{{ 'Off' | t }}</option>
            </select>
          </label>
        </div>

        @if (accounts.multi() || (!isMain() && account())) {
          <label class="wide">
            <span class="lbl">{{ 'Account' | t }}</span>
            <select [ngModel]="account()" (ngModelChange)="pickAccount($event)">
              <option value="">
                {{
                  "Automatic — the project's account, else {name}"
                    | t: { name: bridgeDefaultName() }
                }}
              </option>
              @for (a of accounts.list(); track a.id) {
                <option [value]="a.id">{{ a.name }}{{ a.email ? ' — ' + a.email : '' }}</option>
              }
              @if (accountGone()) {
                <option [value]="account()">{{ 'A removed account' | t }}</option>
              }
            </select>
            <span class="hint">
              {{
                "The Claude account conversations run under. Automatic takes the project's account, else the one Accounts sets for the API. A request can still name another."
                  | t
              }}
            </span>
          </label>
          @if (account() && (multi() || accountOnly())) {
            <label class="check">
              <input
                type="checkbox"
                [ngModel]="accountOnly()"
                (ngModelChange)="accountOnly.set($event)"
              />
              <span>{{
                "Only this account — every conversation runs as {name}; a request naming another account is refused, and other accounts' conversations stay out of reach"
                  | t: { name: accountName() }
              }}</span>
            </label>
          }
        }

        <label class="wide">
          <span class="lbl">{{ 'Permissions' | t }}</span>
          <select [ngModel]="permissionMode()" (ngModelChange)="permissionMode.set($event)">
            @for (p of permissionModes; track p.value) {
              <option [value]="p.value">{{ p.label | t }} — {{ p.hint | t }}</option>
            }
          </select>
          <span class="hint">
            {{ 'Approvals reach you as prompt cards here and on your phone, and over' | t }}
            <code>GET /requests</code>.
          </span>
        </label>

        <label class="check">
          <input type="checkbox" [ngModel]="isolate()" (ngModelChange)="isolate.set($event)" />
          <span
            >{{ "Ignore project settings — don't load" | t }} <code>CLAUDE.md</code
            >{{ ', hooks or MCP servers from the working directory' | t }}</span
          >
        </label>
      </div>

      <!-- ── Conversation ───────────────────────────────────────── -->
      <h2>{{ 'Conversation' | t }}</h2>
      <p class="muted">
        {{ 'Where a message with no' | t }} <code>thread_id</code>
        {{ 'goes.||where a message with no thread_id goes' | t }}
        {{ 'Pass||a field, in any request' | t }} <code>createNewChat: true</code>
        {{
          'in any request to branch off a new one — it is false by default so a client that only sends'
            | t
        }}
        <code>{{ '{' }} "message": "…" {{ '}' }}</code>
        {{ 'keeps one continuous conversation.' | t }}
      </p>

      <div class="card form">
        <label class="wide">
          <span class="lbl">{{ 'Messages join' | t }}</span>
          <select [ngModel]="thread()" (ngModelChange)="thread.set($event)">
            <option [value]="FRESH">
              {{ 'A new conversation, created by the first message' | t }}
            </option>
            @if (liveThreads(s).length) {
              <optgroup [attr.label]="'Running now' | t">
                @for (t of liveThreads(s); track t.threadId) {
                  <option [value]="t.threadId">
                    {{ t.title || shortId(t.threadId) }} — {{ t.cwd
                    }}{{ accounts.multi() ? ' · ' + threadAccount(t) : '' }}
                  </option>
                }
              </optgroup>
            }
            @if (pastSessions(s).length) {
              <optgroup [attr.label]="'Earlier conversations' | t">
                @for (p of pastSessions(s); track p.id) {
                  <option [value]="p.id">{{ label(p) }} — {{ rel(p.lastActivityAt) }}</option>
                }
              </optgroup>
            }
          </select>
          <span class="hint">
            {{
              'Picking an earlier conversation resumes it — same history, same working directory.'
                | t
            }}
          </span>
        </label>
        @if (picked(); as p) {
          <mc-conversation-files
            [id]="p.id"
            [title]="p.title"
            [account]="p.account"
            [running]="p.running"
            (ran)="onDeleteRan($event)"
          />
        }

        @if (s.running && thread() !== (s.activeThread ?? FRESH)) {
          <button class="btn" type="button" [disabled]="busy()" (click)="applyThread()">
            {{ 'Switch conversation' | t }}
          </button>
        }
      </div>

      <!-- ── Access ─────────────────────────────────────────────── -->
      @if (s.port) {
        <h2>{{ 'Access' | t }}</h2>
        <div class="card form">
          <label class="check">
            <input
              type="checkbox"
              [ngModel]="openToTunnels()"
              (ngModelChange)="openToTunnels.set($event)"
              [disabled]="!isDesktop()"
            />
            <span>{{
              'Open to tunnels — accept calls without a token from anyone who can reach port {port}'
                | t: { port: s.port }
            }}</span>
          </label>
          <span class="hint">
            @if (openToTunnels()) {
              {{
                "On: through ngrok, cloudflared or any tunnel to this port, whoever has its URL can start conversations here and approve Claude's ordinary tool calls on this {computer}. Dangerous actions stay limited to this {computer}."
                  | t: { computer: computer() }
              }}
            } @else {
              {{ closedLabel(s) }}
            }
            @if (!isDesktop()) {
              {{ 'Only the desktop app can change this.' | t }}
              <!-- A start of a server besides the main one closes it unless the
                   desktop opens it (a change under Manage keeps its opening). -->
              @if (!isMain() && openToTunnels()) {
                <em>{{ 'Restart with these settings' | t }}</em>
                {{ 'from here closes it to tunnels.' | t }}
              }
            }
          </span>
          @if (openToTunnels() && s.exposedByTunnel) {
            <p class="warn">
              {{
                s.tunnelUrl
                  ? ('A tunnel points at this port now: {url}.' | t: { url: s.tunnelUrl })
                  : ('A tunnel points at this port now.' | t)
              }}
            </p>
          }
        </div>
      }

      <!-- ── Action ─────────────────────────────────────────────── -->
      <div class="actions">
        <button
          class="btn primary"
          type="button"
          [disabled]="busy() || accountGone()"
          (click)="start()"
        >
          {{
            busy()
              ? ('Working…' | t)
              : s.running
                ? ('Restart with these settings' | t)
                : startLabel()
          }}
        </button>
        @if (!isMain() && !s.running) {
          <button
            class="btn"
            type="button"
            [disabled]="busy() || accountGone()"
            (click)="saveDefaults()"
          >
            {{ 'Save without starting' | t }}
          </button>
        }
        @if (!s.running) {
          <span class="muted small">
            {{ 'Starting publishes' | t }}
            <span class="mono">{{ s.url || ('the /v1 routes' | t) }}</span>
            {{ 'to local tools.' | t }}
          </span>
        }
      </div>

      @if (notice(); as n) {
        @if (n.at === 'actions') {
          <p class="notice ok small said" role="status">{{ n.text }}</p>
        }
      }

      @if (!multi() && s.enabled) {
        <p class="more muted small">
          <button class="btn ghost small" type="button" (click)="adding.set(true)">
            <mc-icon name="plus" [size]="14" /> {{ 'Add API server' | t }}
          </button>
          <span>{{ 'Another API on a port of its own — for one Claude account, say.' | t }}</span>
        </p>
      }

      @if (!s.enabled) {
        <div class="card muted">
          {{ 'The API is disabled for this process' | t }} (<code>MOTHER_CLAUDE_BRIDGE=0</code>).
        </div>
      }
    } @else if (!error()) {
      <p class="muted">{{ 'Loading…' | t }}</p>
    }

    <mc-api-server-sheet
      [open]="adding() || managing()"
      [server]="managing() ? selected() : null"
      [servers]="servers()"
      [mainPort]="status()?.port"
      [mainDefaults]="status()?.defaults ?? {}"
      [launch]="launch()"
      (closed)="closeSheet()"
      (saved)="onSaved($event)"
      (removed)="onRemoved($event)"
    />
  `,
  styles: [
    `
      .lede {
        margin: -4px 0 18px;
        max-width: 46rem;
      }
      h2 {
        margin: 26px 0 4px;
        font-size: 0.85rem;
        text-transform: uppercase;
        letter-spacing: 0.06em;
        opacity: 0.7;
      }
      h2 + .muted {
        margin: 0 0 10px;
        max-width: 46rem;
      }
      h2 + mc-api-server-addresses {
        display: block;
        margin-top: 10px;
      }
      .card.err {
        border-color: var(--error, #a63232);
        color: var(--error, #a63232);
      }
      .card.status.picked {
        margin-top: 12px;
      }
      .status .head {
        align-items: center;
        gap: 10px;
        margin-bottom: 12px;
      }
      .sname {
        font-size: 1rem;
        overflow-wrap: anywhere;
      }
      .url {
        text-decoration: none;
        overflow-wrap: anywhere;
      }
      .facts {
        display: grid;
        grid-template-columns: max-content 1fr;
        gap: 4px 16px;
        margin: 0;
        font-size: 0.9rem;
      }
      .facts dt {
        opacity: 0.6;
      }
      .facts dd {
        margin: 0;
        min-width: 0;
        overflow-wrap: anywhere;
      }
      .warn {
        color: var(--warn, #b8860b);
      }
      .links {
        display: flex;
        gap: 8px;
        margin-top: 14px;
      }
      .snippet {
        margin: 12px 0 0;
        padding: 10px 12px;
        border-radius: 8px;
        background: rgba(127, 127, 127, 0.1);
        font-size: 12px;
        line-height: 1.5;
        overflow-x: auto;
        white-space: pre;
      }
      .form {
        display: flex;
        flex-direction: column;
        gap: 14px;
      }
      .form .row {
        gap: 12px;
        flex-wrap: wrap;
      }
      label {
        display: flex;
        flex-direction: column;
        gap: 4px;
        min-width: 0;
      }
      label.wide {
        width: 100%;
      }
      label.check {
        flex-direction: row;
        align-items: flex-start;
        gap: 8px;
        font-size: 0.9rem;
      }
      label.check input {
        margin-top: 2px;
      }
      .lbl {
        font-size: 0.78rem;
        text-transform: uppercase;
        letter-spacing: 0.04em;
        opacity: 0.6;
      }
      .hint {
        font-size: 0.8rem;
        opacity: 0.6;
      }
      input,
      select {
        font: inherit;
        padding: 7px 9px;
        border-radius: 7px;
        border: 1px solid rgba(127, 127, 127, 0.35);
        background: transparent;
        color: inherit;
        min-width: 0;
      }
      .actions {
        display: flex;
        align-items: center;
        gap: 12px;
        margin: 18px 0 40px;
        flex-wrap: wrap;
      }
      .said {
        margin: -24px 0 40px;
      }
      .said-top {
        margin: 12px 0 0;
      }
      .said-top + .card.status {
        margin-top: 12px;
      }
      .more {
        display: flex;
        align-items: center;
        flex-wrap: wrap;
        gap: 4px 10px;
        margin: -24px 0 40px;
      }
      .more .btn {
        margin-left: -0.6rem;
      }
      @media (max-width: 560px) {
        .form .row {
          flex-direction: column;
          align-items: stretch;
        }
        .actions .btn {
          width: 100%;
        }
      }
    `,
  ],
})
export class BridgeComponent {
  private api = inject(ApiService);
  /** Only the desktop app may open the port to tunnels (the server enforces it). */
  readonly isDesktop = inject(AuthService).isDesktop;
  readonly accounts = inject(AccountsService);

  readonly FRESH = FRESH;
  readonly permissionModes = API_PERMISSION_MODES;

  /** `GET /api/bridge`: the main server's status, with `servers` while
   * another exists. */
  readonly status = signal<BridgeStatus | null>(null);
  readonly launch = signal<LaunchDefaults | null>(null);
  readonly sessions = signal<SessionSummary[]>([]);
  readonly error = signal('');
  /** What was just done, said where the eye is: at the top for a server
   * added or removed, under the buttons for one saved there. */
  readonly notice = signal<{ text: string; at: 'top' | 'actions' } | null>(null);
  readonly busy = signal(false);

  // Form state, seeded from whatever the API is already configured with.
  readonly cwd = signal('');
  readonly model = signal('');
  readonly effort = signal('');
  readonly thinking = signal('');
  readonly permissionMode = signal('default');
  readonly isolate = signal(false);
  readonly thread = signal(FRESH);
  /** Access: no token from anyone who can reach the port, tunnels included. */
  readonly openToTunnels = signal(false);
  /** The account conversations run under ('' = the project's, else the
   * accounts' API default). */
  readonly account = signal('');
  /** Run only as that account (a request naming another is refused). */
  readonly accountOnly = signal(false);

  // ── Servers (docs/ACCOUNTS.md §13.5)
  /** Every server, main first; none while the main one is the only one. */
  readonly servers = computed(() => serversOf(this.status()));
  readonly multi = computed(() => this.servers().length > 0);
  /** The server picked (`?server=` on opening). */
  readonly selectedId = signal(MAIN_SERVER);
  readonly selected = computed(
    () => this.servers().find((s) => s.id === this.selectedId()) ?? null,
  );
  /** The server the panel shows: the main one unless another is picked. */
  readonly panelId = computed(() => this.selected()?.id ?? MAIN_SERVER);
  readonly isMain = computed(() => this.panelId() === MAIN_SERVER);
  readonly panelName = computed(() => this.selected()?.name ?? MAIN_SERVER_NAME);
  /** What the panel shows: the main payload, or the picked server's. */
  readonly panel = computed(() => {
    const s = this.status();
    return s ? serverPanel(s, this.selected()) : null;
  });
  readonly adding = signal(false);
  readonly managing = signal(false);
  /** The desktop's `GET /api/remote`, for the LAN address. */
  private readonly remote = signal<RemoteInfo | null>(null);
  /** The server the form was seeded for. */
  private seededFor = '';
  private readonly panelTop = viewChild<ElementRef<HTMLElement>>('panelTop');

  readonly efforts = computed(() => this.launch()?.efforts ?? Object.keys(EFFORT_LABELS));
  /** The desktop's computer in words ("Mac", "PC", "computer"). */
  readonly computer = computed(() => computerNoun());
  readonly bridgeDefaultName = computed(() => {
    const id = this.accounts.view()?.defaults.bridge;
    return this.accounts.byId(id)?.name ?? tr('Default');
  });
  /** The account picked for a server besides the main one is gone (it keeps
   * a removed one until another is chosen; the main one forgets it). */
  readonly accountGone = computed(() => {
    const id = this.account();
    const server = this.selected();
    if (!id || !server || this.isMain()) return false;
    return (server.accountMissing && server.defaults.account === id) || this.accountUnknown(id);
  });
  readonly accountName = computed(
    () => this.accounts.byId(this.account())?.name ?? tr('it||an account'),
  );
  readonly startLabel = computed(() =>
    this.isMain() ? tr('Start the API') : tr('Start {name}', { name: this.panelName() }),
  );

  readonly claudeLine = computed<ClaudeLine>(() => {
    const s = this.panel();
    if (!s) return { text: '', ok: true };
    const server = this.multi() ? this.selected() : null;
    const id = server?.defaults.account;
    if (!server || !id) return { text: s.claudeAccount, ok: s.claudeAuthenticated };
    const acc = serverAccount(server, (x) => this.accounts.byId(x)?.name);
    const only = acc.only ? ' · ' + tr('only this account') : '';
    if (acc.missing) return { text: tr('A removed account'), ok: false, account: id, gone: true };
    const view = this.accounts.byId(id);
    if (!view) return { text: acc.name + only, ok: true, account: id };
    return {
      text: `${view.name} — ${signInLine(view).text}${only}`,
      ok: view.signedIn,
      account: id,
      ...(view.signedIn ? {} : { signedOut: view.name }),
    };
  });

  readonly addresses = computed<ServerAddress[]>(() => {
    const server = this.selected();
    const main = this.servers()[0];
    if (!server || !main) return [];
    const remote = this.remote();
    return serverAddresses(server, {
      desktop: this.isDesktop(),
      main: { port: main.port, running: main.running, tunnelUrl: main.tunnelUrl },
      lanBase: lanBaseOf(remote, location.origin),
      dashboardPort: remote?.lan.port,
    });
  });

  constructor() {
    const wanted = inject(ActivatedRoute).snapshot.queryParamMap.get('server');
    if (wanted) this.selectedId.set(wanted);
    void this.load();
    // The picker needs the accounts: with only the Default nothing sent them.
    void this.accounts.ensureLoaded();
    // The desktop knows its LAN address (a phone on the LAN uses its own
    // origin): asked once servers besides the main one exist.
    let remoteAsked = false;
    effect(() => {
      if (remoteAsked || !this.multi() || !this.isDesktop()) return;
      remoteAsked = true;
      untracked(() => {
        this.api.getRemote().then(
          (r) => this.remote.set(r),
          () => undefined,
        );
      });
    });
  }

  private async load(): Promise<void> {
    try {
      const [status, launch, sessions] = await Promise.all([
        this.api.getBridge(),
        this.api.getDefaults().catch(() => null),
        this.api
          .listSessions({ scope: 'all', limit: 60 })
          .then((page) => page.sessions ?? [])
          .catch(() => [] as SessionSummary[]),
      ]);
      this.launch.set(launch);
      this.sessions.set(sessions);
      this.apply(status, true);
      this.error.set('');
    } catch (e) {
      this.error.set(e instanceof Error ? e.message : String(e));
    }
  }

  /** Adopt a status payload. The form is seeded from the shown server's
   * defaults when asked to, or when the server it was seeded for is no
   * longer the one shown (it was removed, say). */
  private apply(status: BridgeStatus | null, reseed: boolean): void {
    this.status.set(status);
    if (reseed || this.panelId() !== this.seededFor) this.seed();
  }

  /** Seed the form from what the shown server is configured with. */
  private seed(): void {
    const status = this.panel();
    if (!status) return;
    this.seededFor = this.panelId();
    const d: ServerDefaults = status.defaults ?? {};
    this.cwd.set(d.cwd ?? '');
    this.model.set(d.model ?? '');
    this.effort.set(d.effort ?? '');
    this.thinking.set(d.thinking ?? '');
    this.permissionMode.set(d.permissionMode ?? 'default');
    this.isolate.set(Array.isArray(d.settingSources) && d.settingSources.length === 0);
    this.thread.set(status.activeThread ?? FRESH);
    this.openToTunnels.set(status.tokenPolicy === 'never');
    // Absent: none was chosen at start (the project's, else the accounts'
    // API default).
    this.account.set(d.account ?? '');
    this.accountOnly.set(d.accountOnly === true);
  }

  private defaults(): ServerDefaults {
    // Only while several accounts exist for the main server: the request is
    // unchanged otherwise. Another server keeps the account it names.
    const account =
      (this.isMain() ? this.accounts.multi() && this.account() : this.account()) || undefined;
    return {
      cwd: this.cwd().trim() || undefined,
      model: this.model() || undefined,
      effort: this.effort() || undefined,
      thinking: (this.thinking() || undefined) as 'on' | 'off' | undefined,
      permissionMode: this.permissionMode() || undefined,
      // `[]` is meaningful — it isolates the conversation — so it must not be
      // collapsed to "unset" the way an empty string is.
      settingSources: this.isolate() ? [] : undefined,
      defaultThread: this.thread() || undefined,
      account,
      // Only with an account: the server refuses it alone.
      accountOnly: account && this.accountOnly() ? true : undefined,
    };
  }

  /** Run an action and adopt its answer: the main server's routes answer
   * the whole payload, a server's routes that server's status. */
  private async run(action: () => Promise<unknown>): Promise<void> {
    this.busy.set(true);
    this.error.set('');
    this.notice.set(null);
    try {
      this.apply(absorbReply(this.status(), await action()), true);
    } catch (e) {
      this.error.set(this.explain(e));
    } finally {
      this.busy.set(false);
    }
  }

  private explain(e: unknown): string {
    const main = this.isMain();
    return serverErrorText(e, {
      server: main && !this.multi() ? tr('the API') : this.panelName(),
      main,
      account: this.accounts.byId(this.account())?.name,
      servers: this.servers(),
    });
  }

  start(): Promise<void> {
    // A phone cannot change access: it starts with whatever is configured.
    const open = this.isDesktop() ? this.openToTunnels() : undefined;
    const id = this.panelId();
    return this.run(() =>
      id === MAIN_SERVER
        ? this.api.startBridge(this.defaults(), open)
        : this.api.startBridgeServer(id, this.defaults(), open),
    );
  }

  stop(): Promise<void> {
    const id = this.panelId();
    return this.run(() =>
      id === MAIN_SERVER ? this.api.stopBridge() : this.api.stopBridgeServer(id),
    );
  }

  applyThread(): Promise<void> {
    const id = this.panelId();
    const thread = this.thread() || null;
    return this.run(() =>
      id === MAIN_SERVER
        ? this.api.setBridgeThread(thread)
        : this.api.setBridgeServerThread(id, thread),
    );
  }

  /** A stopped server's defaults, kept for its next start. */
  async saveDefaults(): Promise<void> {
    const id = this.panelId();
    if (id === MAIN_SERVER) return;
    await this.run(() => this.api.updateBridgeServer(id, { defaults: this.defaults() }));
    if (!this.error()) {
      this.notice.set({
        text: tr('Saved: {name} starts with these settings.', { name: this.panelName() }),
        at: 'actions',
      });
    }
  }

  /** Show another server's panel. */
  pick(id: string): void {
    if (id === this.panelId()) return;
    this.selectedId.set(id);
    this.error.set('');
    this.notice.set(null);
    this.seed();
    // On a phone the panel is below the list: bring it into view.
    const top = this.panelTop()?.nativeElement;
    if (top && top.getBoundingClientRect().top > window.innerHeight * 0.6) {
      top.scrollIntoView({ block: 'start' });
    }
  }

  /** Picking an account for a server besides the main one runs it only as
   * that account, until unticked (as when adding one). */
  pickAccount(id: string): void {
    if (!id) this.accountOnly.set(false);
    else if (!this.account() && !this.isMain()) this.accountOnly.set(true);
    this.account.set(id);
  }

  closeSheet(): void {
    this.adding.set(false);
    this.managing.set(false);
  }

  /** A server was added (its panel shows next) or changed. */
  onSaved(reply: unknown): void {
    const added = this.adding();
    this.closeSheet();
    const server = normalizeServer(reply);
    const next = absorbReply(this.status(), reply);
    if (added && server) {
      this.selectedId.set(server.id);
      this.apply(next, true);
      this.notice.set({
        text: tr('Added {name}. It is stopped: check its defaults below, then start it.', {
          name: server.name,
        }),
        at: 'top',
      });
    } else {
      // Its name or port: what is being edited in the form stays.
      this.apply(next, false);
      // An answer that isn't a server's status: read the list again.
      if (!server) void this.refresh();
    }
  }

  private async refresh(): Promise<void> {
    try {
      this.apply(await this.api.getBridge(), false);
    } catch {
      // The next action says what is wrong.
    }
  }

  onRemoved(id: string): void {
    const name = this.servers().find((s) => s.id === id)?.name;
    this.closeSheet();
    this.selectedId.set(MAIN_SERVER);
    this.apply(withoutServer(this.status(), id), true);
    this.notice.set({
      text: name ? tr('Removed {name}.', { name }) : tr('Removed the server.'),
      at: 'top',
    });
  }

  liveThreads(s: BridgeStatus) {
    return s.threads ?? [];
  }

  /** The conversation picked in "Messages join", for its files and
   * Delete… (docs/ACCOUNTS.md §14.4). */
  readonly picked = computed(() => {
    const id = this.thread();
    if (id === FRESH) return null;
    const t = this.panel()?.threads?.find((x) => x.threadId === id);
    const s = this.sessions().find((x) => x.id === id);
    const title = t?.title || (s ? sessionName(s) : '');
    return {
      id,
      title,
      account: t ? (t.account?.id ?? DEFAULT_ACCOUNT) : s?.account?.id,
      running: !!t,
    };
  });

  /** A delete ran: what it deleted leaves the picker, and the status is
   * read again — the server cleared a deleted conversation from every
   * server's active conversation and defaults. */
  onDeleteRan(gone: ReadonlySet<string>): void {
    this.sessions.update((list) => list.filter((s) => !gone.has(s.id)));
    if (gone.has(this.thread())) this.thread.set(FRESH);
    this.api.getBridge().then(
      (status) => this.apply(status, false),
      () => undefined,
    );
  }

  /** The account a running conversation is under (absent: the Default's). */
  threadAccount(t: BridgeThread): string {
    return t.account?.name ?? this.accounts.byId(DEFAULT_ACCOUNT)?.name ?? tr('Default');
  }

  /** Recent sessions that are not already listed as running conversations —
   * only the account's own, for a server that runs only as it. */
  pastSessions(s: BridgeStatus): SessionSummary[] {
    const live = new Set((s.threads ?? []).map((t) => t.threadId));
    const only = this.accountOnly() ? this.account() : '';
    return this.sessions()
      .filter((session) => !live.has(session.id) && session.messageCount > 0)
      .filter((session) => !only || (session.account?.id ?? DEFAULT_ACCOUNT) === only)
      .sort((a, b) => (b.lastActivityAt ?? 0) - (a.lastActivityAt ?? 0))
      .slice(0, 25);
  }

  label(session: SessionSummary): string {
    return sessionName(session);
  }

  /** What the port does with the switch off, given the configured policy. */
  closedLabel(s: BridgeStatus): string {
    return s.configuredTokenPolicy === 'required'
      ? tr('Off: every call needs a token.')
      : tr(
          'Off: no token for programs on this {computer}; through a tunnel a token is needed, and while a tunnel points at this port, every call needs one.',
          { computer: computerNoun() },
        );
  }

  /** Why no token from anywhere matters now: a tunnel points at the port,
   * or one started later would. */
  openWarning(s: BridgeStatus): string {
    const computer = computerNoun();
    if (!s.exposedByTunnel) {
      return tr(
        'Anyone who can reach this port — through a tunnel you start later, too — can drive Claude on this {computer}.',
        { computer },
      );
    }
    return s.tunnelUrl
      ? tr(
          "A tunnel points at this port ({url}): anyone who finds that URL can start conversations here and approve Claude's ordinary tool calls on this {computer}.",
          { url: s.tunnelUrl, computer },
        )
      : tr(
          "A tunnel points at this port: anyone who finds that URL can start conversations here and approve Claude's ordinary tool calls on this {computer}.",
          { computer },
        );
  }

  /** The Authentication line: who may call this port without a token. */
  authLabel(s: BridgeStatus): string {
    switch (s.tokenPolicy) {
      case 'required':
        return tr('bearer token required');
      case 'never':
        return tr('none — anyone who can reach the port, through a tunnel too');
      default:
        return s.exposedByTunnel
          ? tr('token required while a tunnel points at this port')
          : tr('none — any local tool can call it');
    }
  }

  /** "3 operations" in flight. */
  inFlight(n: number): string {
    return plural(n, 'operation');
  }

  /** The call to paste into a terminal, matching this listener's auth policy. */
  snippet(s: BridgeStatus): string {
    const url = s.url ?? '/v1';
    // The default policy wants a token too while a tunnel points at the port.
    const needsToken = s.requireToken || (s.tokenPolicy === 'local' && s.exposedByTunnel);
    const auth = needsToken ? ' \\\n  -H "Authorization: Bearer <token>"' : '';
    return (
      `curl -sS ${url}/chat${auth} \\\n` +
      `  -H 'Content-Type: application/json' \\\n` +
      `  -d '{"message":"What changed in this repo today?"}'`
    );
  }

  effortLabel(value: string): string {
    return effortLabel(value);
  }

  shortId(id: string): string {
    return id.slice(0, 8);
  }

  rel(at?: number): string {
    return relativeTime(at);
  }

  /** An account id the loaded registry doesn't have. */
  private accountUnknown(id: string): boolean {
    return !!this.accounts.view() && !this.accounts.byId(id);
  }
}
