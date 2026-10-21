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
  serverAccount,
  serverAddresses,
  serverErrorText,
  serverPanel,
  serversOf,
} from '../../core/api-servers';
import { ApiService } from '../../core/api.service';
import { AuthService } from '../../core/auth.service';
import {
  BridgeStatus,
  BridgeThread,
  LaunchDefaults,
  RemoteInfo,
  SessionSummary,
} from '../../core/models';
import { signInLine } from '../../shared/accounts';
import { EFFORT_LABELS, effortLabel, relativeTime, sessionName } from '../../shared/util';
import { API_PERMISSION_MODES } from './permission-modes';
import { ApiServerAddressesComponent } from './server-addresses.component';
import { ApiServerListComponent } from './server-list.component';

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
 * with its addresses; with only the main one it is the screen it always was.
 * Phones may do all of it but open a port to tunnels (the server enforces
 * that, and a mode that skips approvals).
 */
@Component({
  selector: 'mc-bridge',
  imports: [FormsModule, ApiServerAddressesComponent, ApiServerListComponent],
  template: `
    <div class="page-head"><h1>API</h1></div>
    <p class="lede">
      A local HTTP API for Claude on this machine — conversations, turns, live streams, tool
      approvals, images. It is off until you start it.
    </p>

    @if (error(); as e) {
      <div class="card err" role="alert">{{ e }}</div>
    }

    @if (panel(); as s) {
      @if (multi()) {
        <mc-api-server-list [servers]="servers()" [selected]="panelId()" (picked)="pick($event)" />
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
            {{ s.running ? 'running' : 'stopped' }}
          </span>
          @if (s.running && s.url) {
            <a class="mono url" [href]="s.url + '/docs/'" target="_blank" rel="noreferrer">{{
              s.url
            }}</a>
          } @else if (s.port) {
            <span class="mono muted">port {{ s.port }}</span>
          }
          <span class="spacer"></span>
          @if (s.running) {
            <button class="btn" type="button" [disabled]="busy()" (click)="stop()">Stop</button>
          }
        </div>

        <dl class="facts">
          <dt>Claude account</dt>
          <dd [class.warn]="!claudeLine().ok">{{ claudeLine().text }}</dd>
          <dt>Runtime</dt>
          <dd>
            {{ s.hostBuilt ? (s.hostRunning ? 'running' : 'ready') : 'not built' }}
            @if (!s.hostBuilt) {
              <span class="muted small">— run <code>npm run sidecar:build</code></span>
            }
          </dd>
          <dt>Authentication</dt>
          <dd [class.warn]="s.tokenPolicy === 'never'">{{ authLabel(s) }}</dd>
          @if (s.running) {
            <dt>Started</dt>
            <dd>{{ rel(s.startedAt) }}</dd>
            @if (isMain()) {
              <dt>In flight</dt>
              <dd>{{ s.activeOperations }} operation{{ s.activeOperations === 1 ? '' : 's' }}</dd>
            }
          }
        </dl>

        @if (s.tokenPolicy === 'never') {
          <p class="warn">
            <strong>No token is required from anywhere.</strong>
            @if (s.exposedByTunnel) {
              A tunnel points at this port{{ s.tunnelUrl ? ' (' + s.tunnelUrl + ')' : '' }}: anyone
              who finds that URL can start conversations here and approve Claude's ordinary tool
              calls on this Mac.
            } @else {
              Anyone who can reach this port — through a tunnel you start later, too — can drive
              Claude on this Mac.
            }
            Dangerous actions stay limited to callers on this Mac. Turn off
            <em>Open to tunnels</em> below and restart to close it.
          </p>
        }

        @if (claudeLine().gone) {
          <p class="warn">
            Its account was removed, so it cannot start. Choose another account under Defaults (or
            Automatic), then start it.
          </p>
        } @else if (claudeLine().signedOut; as name) {
          <p class="warn">
            {{ name }} is not signed in, so every message would fail. Sign it in on the Accounts
            page of the desktop app.
          </p>
        } @else if (!s.claudeAuthenticated && !claudeLine().account) {
          <p class="warn">
            Claude itself is not signed in, so every message would fail. Run
            <code>claude auth login</code> in a terminal, then reload this page.
          </p>
        }

        @if (s.running && s.url) {
          <div class="links">
            <a class="btn" [href]="s.docsUrl" target="_blank" rel="noreferrer">API reference</a>
            <a class="btn" [href]="s.consoleUrl" target="_blank" rel="noreferrer">Console</a>
          </div>
          <pre class="mono snippet">{{ snippet(s) }}</pre>
        }
      </div>

      @if (multi() && addresses().length) {
        <!-- ── Addresses ──────────────────────────────────────────── -->
        <h2>Addresses</h2>
        <mc-api-server-addresses [addresses]="addresses()" />
      }

      <!-- ── Defaults ───────────────────────────────────────────── -->
      <h2>{{ s.running ? 'Defaults in use' : 'Defaults' }}</h2>
      <p class="muted">
        Applied to conversations the API creates. Every one can be overridden in a request payload.
        Leave a field on <em>Default</em> to inherit your own Claude settings.
      </p>

      <div class="card form">
        <label class="wide">
          <span class="lbl">Working directory</span>
          <input
            [ngModel]="cwd()"
            (ngModelChange)="cwd.set($event)"
            placeholder="your home directory"
            spellcheck="false"
          />
          <span class="hint">Where conversations run, and which files Claude can reach.</span>
        </label>

        <div class="row">
          <label>
            <span class="lbl">Model</span>
            <select [ngModel]="model()" (ngModelChange)="model.set($event)">
              <option value="">Default{{ launch()?.model ? ' — ' + launch()!.model : '' }}</option>
              @for (m of launch()?.models ?? []; track m.value) {
                <option [value]="m.value" [title]="m.description || ''">{{ m.label }}</option>
              }
            </select>
          </label>

          <label>
            <span class="lbl">Effort</span>
            <select [ngModel]="effort()" (ngModelChange)="effort.set($event)">
              <option value="">
                Default{{ launch()?.effort ? ' — ' + effortLabel(launch()!.effort!) : '' }}
              </option>
              @for (e of efforts(); track e) {
                <option [value]="e">{{ effortLabel(e) }}</option>
              }
            </select>
          </label>

          <label>
            <span class="lbl">Thinking</span>
            <select [ngModel]="thinking()" (ngModelChange)="thinking.set($event)">
              <option value="">Default</option>
              <option value="on">On</option>
              <option value="off">Off</option>
            </select>
          </label>
        </div>

        @if (accounts.multi() || (!isMain() && account())) {
          <label class="wide">
            <span class="lbl">Account</span>
            <select [ngModel]="account()" (ngModelChange)="pickAccount($event)">
              <option value="">
                Automatic — the project's account, else {{ bridgeDefaultName() }}
              </option>
              @for (a of accounts.list(); track a.id) {
                <option [value]="a.id">{{ a.name }}{{ a.email ? ' — ' + a.email : '' }}</option>
              }
              @if (accountGone()) {
                <option [value]="account()">A removed account</option>
              }
            </select>
            <span class="hint">
              The Claude account conversations run under. Automatic takes the project's account,
              else the one Settings › Accounts sets for the API. A request can still name another.
            </span>
          </label>
          @if (account() && (multi() || accountOnly())) {
            <label class="check">
              <input
                type="checkbox"
                [ngModel]="accountOnly()"
                (ngModelChange)="accountOnly.set($event)"
              />
              <span
                >Only this account — every conversation runs as {{ accountName() }}; a request
                naming another account is refused, and other accounts' conversations stay out of
                reach</span
              >
            </label>
          }
        }

        <label class="wide">
          <span class="lbl">Permissions</span>
          <select [ngModel]="permissionMode()" (ngModelChange)="permissionMode.set($event)">
            @for (p of permissionModes; track p.value) {
              <option [value]="p.value">{{ p.label }} — {{ p.hint }}</option>
            }
          </select>
          <span class="hint">
            Approvals reach you as prompt cards here and on your phone, and over
            <code>GET /requests</code>.
          </span>
        </label>

        <label class="check">
          <input type="checkbox" [ngModel]="isolate()" (ngModelChange)="isolate.set($event)" />
          <span
            >Ignore project settings — don't load <code>CLAUDE.md</code>, hooks or MCP servers from
            the working directory</span
          >
        </label>
      </div>

      <!-- ── Conversation ───────────────────────────────────────── -->
      <h2>Conversation</h2>
      <p class="muted">
        Where a message with no <code>thread_id</code> goes. Pass
        <code>createNewChat: true</code> in any request to branch off a new one — it is false by
        default so a client that only sends <code>{{ '{' }} "message": "…" {{ '}' }}</code> keeps
        one continuous conversation.
      </p>

      <div class="card form">
        <label class="wide">
          <span class="lbl">Messages join</span>
          <select [ngModel]="thread()" (ngModelChange)="thread.set($event)">
            <option [value]="FRESH">A new conversation, created by the first message</option>
            @if (liveThreads(s).length) {
              <optgroup label="Running now">
                @for (t of liveThreads(s); track t.threadId) {
                  <option [value]="t.threadId">
                    {{ t.title || shortId(t.threadId) }} — {{ t.cwd
                    }}{{ accounts.multi() ? ' · ' + threadAccount(t) : '' }}
                  </option>
                }
              </optgroup>
            }
            @if (pastSessions(s).length) {
              <optgroup label="Earlier conversations">
                @for (p of pastSessions(s); track p.id) {
                  <option [value]="p.id">{{ label(p) }} — {{ rel(p.lastActivityAt) }}</option>
                }
              </optgroup>
            }
          </select>
          <span class="hint">
            Picking an earlier conversation resumes it — same history, same working directory.
          </span>
        </label>

        @if (s.running && thread() !== (s.activeThread ?? FRESH)) {
          <button class="btn" type="button" [disabled]="busy()" (click)="applyThread()">
            Switch conversation
          </button>
        }
      </div>

      <!-- ── Access ─────────────────────────────────────────────── -->
      @if (s.port) {
        <h2>Access</h2>
        <div class="card form">
          <label class="check">
            <input
              type="checkbox"
              [ngModel]="openToTunnels()"
              (ngModelChange)="openToTunnels.set($event)"
              [disabled]="!isDesktop()"
            />
            <span
              >Open to tunnels — accept calls without a token from anyone who can reach port
              {{ s.port }}</span
            >
          </label>
          <span class="hint">
            @if (openToTunnels()) {
              On: through ngrok, cloudflared or any tunnel to this port, whoever has its URL can
              start conversations here and approve Claude's ordinary tool calls on this Mac.
              Dangerous actions stay limited to this Mac.
            } @else {
              Off: {{ closedLabel(s) }}
            }
            @if (!isDesktop()) {
              Only the desktop app can change this.
            }
          </span>
          @if (openToTunnels() && s.exposedByTunnel) {
            <p class="warn">
              A tunnel points at this port now{{ s.tunnelUrl ? ': ' + s.tunnelUrl : '' }}.
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
          {{ busy() ? 'Working…' : s.running ? 'Restart with these settings' : startLabel() }}
        </button>
        @if (!isMain() && !s.running) {
          <button
            class="btn"
            type="button"
            [disabled]="busy() || accountGone()"
            (click)="saveDefaults()"
          >
            Save without starting
          </button>
        }
        @if (!s.running) {
          <span class="muted small">
            Starting publishes
            <span class="mono">{{ s.url || 'the /v1 routes' }}</span>
            to local tools.
          </span>
        }
      </div>

      @if (notice(); as n) {
        @if (n.at === 'actions') {
          <p class="notice ok small said" role="status">{{ n.text }}</p>
        }
      }

      @if (!s.enabled) {
        <div class="card muted">
          The API is disabled for this process (<code>MOTHER_CLAUDE_BRIDGE=0</code>).
        </div>
      }
    } @else if (!error()) {
      <p class="muted">Loading…</p>
    }
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
  /** The desktop's `GET /api/remote`, for the LAN address. */
  private readonly remote = signal<RemoteInfo | null>(null);
  /** The server the form was seeded for. */
  private seededFor = '';
  private readonly panelTop = viewChild<ElementRef<HTMLElement>>('panelTop');

  readonly efforts = computed(() => this.launch()?.efforts ?? Object.keys(EFFORT_LABELS));
  readonly bridgeDefaultName = computed(() => {
    const id = this.accounts.view()?.defaults.bridge;
    return this.accounts.byId(id)?.name ?? 'Default';
  });
  /** The account picked for a server besides the main one is gone (it keeps
   * a removed one until another is chosen; the main one forgets it). */
  readonly accountGone = computed(() => {
    const id = this.account();
    const server = this.selected();
    if (!id || !server || this.isMain()) return false;
    return (server.accountMissing && server.defaults.account === id) || this.accountUnknown(id);
  });
  readonly accountName = computed(() => this.accounts.byId(this.account())?.name ?? 'it');
  readonly startLabel = computed(() =>
    this.isMain() ? 'Start the API' : `Start ${this.panelName()}`,
  );

  readonly claudeLine = computed<ClaudeLine>(() => {
    const s = this.panel();
    if (!s) return { text: '', ok: true };
    const server = this.multi() ? this.selected() : null;
    const id = server?.defaults.account;
    if (!server || !id) return { text: s.claudeAccount, ok: s.claudeAuthenticated };
    const acc = serverAccount(server, (x) => this.accounts.byId(x)?.name);
    const only = acc.only ? ' · only this account' : '';
    if (acc.missing) return { text: 'A removed account', ok: false, account: id, gone: true };
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
      server: main && !this.multi() ? 'the API' : this.panelName(),
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
        text: `Saved: ${this.panelName()} starts with these settings.`,
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

  liveThreads(s: BridgeStatus) {
    return s.threads ?? [];
  }

  /** The account a running conversation is under (absent: the Default's). */
  threadAccount(t: BridgeThread): string {
    return t.account?.name ?? this.accounts.byId(DEFAULT_ACCOUNT)?.name ?? 'Default';
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
      ? 'every call needs a token.'
      : 'no token for programs on this Mac; through a tunnel a token is needed, and while a ' +
          'tunnel points at this port, every call needs one.';
  }

  /** The Authentication line: who may call this port without a token. */
  authLabel(s: BridgeStatus): string {
    switch (s.tokenPolicy) {
      case 'required':
        return 'bearer token required';
      case 'never':
        return 'none — anyone who can reach the port, through a tunnel too';
      default:
        return s.exposedByTunnel
          ? 'token required while a tunnel points at this port'
          : 'none — any local tool can call it';
    }
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
