import {
  ChangeDetectionStrategy,
  Component,
  computed,
  effect,
  inject,
  input,
  output,
  signal,
  untracked,
} from '@angular/core';
import { FormsModule } from '@angular/forms';

import { AccountsService } from '../../core/accounts.service';
import {
  FIRST_SERVER_PORT,
  SERVER_NAME_MAX,
  ServerDefaults,
  ServerStatus,
  normalizeDefaults,
  parsePort,
  portProblem,
  serverErrorText,
  serverNameProblem,
} from '../../core/api-servers';
import { ApiError, ApiService } from '../../core/api.service';
import { LaunchDefaults } from '../../core/models';
import { IconComponent } from '../../shared/icon.component';
import { SheetComponent } from '../../shared/sheet.component';
import { EFFORT_LABELS, effortLabel } from '../../shared/util';
import { API_PERMISSION_MODES } from './permission-modes';

/**
 * Add an API server (docs/ACCOUNTS.md §13.6 item 5) — name, account, *Only
 * this account* (on once an account is picked), port (empty: the first free
 * from 5613) and its defaults, the main server's to start with — or, for
 * `server`, change its name or port, or remove it (stopped first). Any
 * signed-in device may; the server keeps a mode that skips approvals to the
 * desktop.
 */
@Component({
  selector: 'mc-api-server-sheet',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [FormsModule, IconComponent, SheetComponent],
  template: `
    <mc-sheet [open]="open()" [title]="title()" (closed)="closed.emit()">
      @if (open()) {
        <form class="form" (submit)="save($event)">
          <label class="field">
            <span class="field-label">Name</span>
            <input
              name="name"
              autocomplete="off"
              [attr.maxlength]="nameMax"
              placeholder="e.g. Work"
              [ngModel]="name()"
              (ngModelChange)="name.set($event); nameTouched.set(true); error.set('')"
            />
          </label>
          @if (nameProblem()) {
            <p class="error-text small">{{ nameProblem() }}</p>
          }

          @if (!server() && accounts.multi()) {
            <label class="field">
              <span class="field-label">Account</span>
              <select name="account" [ngModel]="account()" (ngModelChange)="pickAccount($event)">
                <option value="">
                  Automatic — the project's account, else {{ bridgeDefaultName() }}
                </option>
                @for (a of accounts.list(); track a.id) {
                  <option [value]="a.id">{{ a.name }}{{ a.email ? ' — ' + a.email : '' }}</option>
                }
              </select>
            </label>
            @if (account()) {
              <label class="check">
                <input
                  type="checkbox"
                  name="only"
                  [ngModel]="only()"
                  (ngModelChange)="only.set($event)"
                />
                <span
                  ><strong>Only this account</strong> — every conversation runs as
                  {{ accountName() }}, a request naming another account is refused, and other
                  accounts' conversations stay out of reach: its API can go behind a tunnel without
                  opening the others.</span
                >
              </label>
            }
          }

          <label class="field">
            <span class="field-label">Port</span>
            <input
              name="port"
              inputmode="numeric"
              pattern="[0-9]*"
              autocomplete="off"
              [placeholder]="portPlaceholder()"
              [ngModel]="port()"
              (ngModelChange)="port.set($event); error.set('')"
            />
          </label>
          @if (portProblem()) {
            <p class="error-text small">{{ portProblem() }}</p>
          } @else {
            <p class="field-hint">
              @if (server()) {
                On this Mac only. A running server restarts on the new port.
              } @else {
                On this Mac only. Empty: the first free port from {{ firstPort }}.
              }
            </p>
          }

          @if (!server()) {
            <details class="defaults">
              <summary>Defaults — the main server's, to start with</summary>
              <div class="dfields">
                <label class="field">
                  <span class="field-label">Working directory</span>
                  <input
                    name="cwd"
                    placeholder="your home directory"
                    spellcheck="false"
                    autocapitalize="off"
                    [ngModel]="cwd()"
                    (ngModelChange)="cwd.set($event)"
                  />
                </label>
                <div class="trio">
                  <label class="field">
                    <span class="field-label">Model</span>
                    <select name="model" [ngModel]="model()" (ngModelChange)="model.set($event)">
                      <option value="">
                        Default{{ launch()?.model ? ' — ' + launch()!.model : '' }}
                      </option>
                      @for (m of launch()?.models ?? []; track m.value) {
                        <option [value]="m.value">{{ m.label }}</option>
                      }
                    </select>
                  </label>
                  <label class="field">
                    <span class="field-label">Effort</span>
                    <select name="effort" [ngModel]="effort()" (ngModelChange)="effort.set($event)">
                      <option value="">
                        Default{{ launch()?.effort ? ' — ' + effortName(launch()!.effort!) : '' }}
                      </option>
                      @for (e of efforts(); track e) {
                        <option [value]="e">{{ effortName(e) }}</option>
                      }
                    </select>
                  </label>
                  <label class="field">
                    <span class="field-label">Thinking</span>
                    <select
                      name="thinking"
                      [ngModel]="thinking()"
                      (ngModelChange)="thinking.set($event)"
                    >
                      <option value="">Default</option>
                      <option value="on">On</option>
                      <option value="off">Off</option>
                    </select>
                  </label>
                </div>
                <label class="field">
                  <span class="field-label">Permissions</span>
                  <select
                    name="permissionMode"
                    [ngModel]="permissionMode()"
                    (ngModelChange)="permissionMode.set($event)"
                  >
                    @for (p of permissionModes; track p.value) {
                      <option [value]="p.value">{{ p.label }} — {{ p.hint }}</option>
                    }
                  </select>
                </label>
                <label class="check">
                  <input
                    type="checkbox"
                    name="isolate"
                    [ngModel]="isolate()"
                    (ngModelChange)="isolate.set($event)"
                  />
                  <span>Ignore project settings (<code>CLAUDE.md</code>, hooks, MCP servers)</span>
                </label>
                <p class="field-hint">
                  Change them here or later on its panel; a request can still override any of them.
                </p>
              </div>
            </details>
          }

          @if (error()) {
            <p class="error-text small" role="alert">{{ error() }}</p>
          }
          <div class="row actions">
            <button class="btn primary" type="submit" [disabled]="!canSave()">
              @if (server()) {
                {{ busy() ? 'Saving…' : 'Save' }}
              } @else {
                <mc-icon name="plus" [size]="15" /> {{ busy() ? 'Adding…' : 'Add API server' }}
              }
            </button>
            <button class="btn ghost" type="button" [disabled]="busy()" (click)="closed.emit()">
              Cancel
            </button>
          </div>
        </form>

        @if (server(); as s) {
          <h3>Remove</h3>
          @if (!confirmRemove()) {
            <p class="muted small">{{ removeText(s) }}</p>
            <div class="row">
              <button
                class="btn danger small"
                type="button"
                [disabled]="busy()"
                (click)="confirmRemove.set(true)"
              >
                <mc-icon name="trash" [size]="14" /> Remove {{ s.name }}…
              </button>
            </div>
          } @else {
            <p class="notice warn small" role="alert">
              Remove <strong>{{ s.name }}</strong
              >? {{ removeText(s) }}
            </p>
            <div class="row">
              <button class="btn danger" type="button" [disabled]="busy()" (click)="remove(s)">
                {{ busy() ? 'Removing…' : 'Remove' }}
              </button>
              <button class="btn ghost" type="button" (click)="confirmRemove.set(false)">
                Keep it
              </button>
            </div>
          }
          @if (removeError()) {
            <p class="error-text small" role="alert">{{ removeError() }}</p>
          }
        }
      }
    </mc-sheet>
  `,
  styles: [
    `
      .form {
        display: flex;
        flex-direction: column;
        gap: 0.6rem;
      }
      .form p {
        margin: 0;
      }
      label.check {
        display: flex;
        align-items: flex-start;
        gap: 0.5rem;
        font-size: 0.875rem;
        line-height: 1.45;
        cursor: pointer;
      }
      label.check input {
        margin-top: 0.2rem;
      }
      .defaults {
        border: 1px solid var(--border);
        border-radius: var(--radius-sm);
        background: var(--panel-2);
        padding: 0 0.75rem;
      }
      .defaults summary {
        display: flex;
        align-items: center;
        min-height: var(--tap);
        cursor: pointer;
        font-size: 0.875rem;
        font-weight: 600;
      }
      .defaults[open] summary {
        margin-bottom: 0.2rem;
      }
      .dfields {
        display: flex;
        flex-direction: column;
        gap: 0.6rem;
        padding-bottom: 0.75rem;
      }
      .trio {
        display: grid;
        grid-template-columns: repeat(3, minmax(0, 1fr));
        gap: 0.5rem;
      }
      .actions {
        margin-top: 0.3rem;
      }
      h3 {
        margin: 1.2rem 0 0.3rem;
        font-size: 0.9rem;
      }
      h3 + p {
        margin: 0 0 0.5rem;
      }
      @media (max-width: 480px) {
        .trio {
          grid-template-columns: minmax(0, 1fr);
        }
      }
    `,
  ],
})
export class ApiServerSheetComponent {
  private api = inject(ApiService);
  protected readonly accounts = inject(AccountsService);

  readonly open = input(false);
  /** The server to change; null: add one. */
  readonly server = input<ServerStatus | null>(null);
  /** Every server (main first), for the name and port checks. */
  readonly servers = input<ServerStatus[]>([]);
  readonly mainPort = input<number | undefined>(undefined);
  /** The main server's defaults now: a new server's, to start with. */
  readonly mainDefaults = input<ServerDefaults>({});
  readonly launch = input<LaunchDefaults | null>(null);
  readonly closed = output<void>();
  /** The server's status, as the server answered it. */
  readonly saved = output<unknown>();
  readonly removed = output<string>();

  protected readonly nameMax = SERVER_NAME_MAX;
  protected readonly firstPort = FIRST_SERVER_PORT;
  protected readonly permissionModes = API_PERMISSION_MODES;

  protected readonly name = signal('');
  protected readonly nameTouched = signal(false);
  protected readonly port = signal('');
  protected readonly account = signal('');
  protected readonly only = signal(false);
  protected readonly cwd = signal('');
  protected readonly model = signal('');
  protected readonly effort = signal('');
  protected readonly thinking = signal('');
  protected readonly permissionMode = signal('default');
  protected readonly isolate = signal(false);
  protected readonly busy = signal(false);
  protected readonly error = signal('');
  protected readonly removeError = signal('');
  protected readonly confirmRemove = signal(false);

  protected readonly title = computed(() => this.server()?.name ?? 'Add an API server');
  protected readonly nameProblem = computed(() =>
    this.nameTouched() ? serverNameProblem(this.name(), this.servers(), this.server()?.id) : '',
  );
  protected readonly portProblem = computed(() =>
    portProblem(this.port(), this.servers(), this.server()?.id, this.mainPort()),
  );
  protected readonly portPlaceholder = computed(() => {
    const p = this.server()?.port;
    return p ? String(p) : 'automatic';
  });
  /** Changing a server: its name or port differs. */
  private readonly changed = computed(() => {
    const s = this.server();
    if (!s) return true;
    const port = parsePort(this.port());
    return this.name().trim() !== s.name || (port !== undefined && port !== s.port);
  });
  protected readonly canSave = computed(
    () =>
      !this.busy() &&
      this.changed() &&
      !serverNameProblem(this.name(), this.servers(), this.server()?.id) &&
      !this.portProblem(),
  );
  protected readonly efforts = computed(() => this.launch()?.efforts ?? Object.keys(EFFORT_LABELS));
  protected readonly bridgeDefaultName = computed(() => {
    const id = this.accounts.view()?.defaults.bridge;
    return this.accounts.byId(id)?.name ?? 'Default';
  });
  protected readonly accountName = computed(
    () => this.accounts.byId(this.account())?.name ?? 'that account',
  );

  constructor() {
    // Every opening starts over, from the server (or the main one's defaults).
    effect(() => {
      if (this.open()) untracked(() => this.reset());
    });
  }

  private reset(): void {
    const s = this.server();
    const d = normalizeDefaults(this.mainDefaults());
    this.name.set(s?.name ?? '');
    this.nameTouched.set(false);
    this.port.set(s?.port ? String(s.port) : '');
    this.account.set('');
    this.only.set(false);
    this.cwd.set(d.cwd ?? '');
    this.model.set(d.model ?? '');
    this.effort.set(d.effort ?? '');
    this.thinking.set(d.thinking ?? '');
    // Only a mode this form offers (one that skips approvals is the desktop's).
    this.permissionMode.set(
      API_PERMISSION_MODES.some((m) => m.value === d.permissionMode)
        ? (d.permissionMode as string)
        : 'default',
    );
    this.isolate.set(Array.isArray(d.settingSources) && d.settingSources.length === 0);
    this.busy.set(false);
    this.error.set('');
    this.removeError.set('');
    this.confirmRemove.set(false);
  }

  protected effortName(e: string): string {
    return effortLabel(e);
  }

  /** Picking an account turns *Only this account* on; Automatic turns it off. */
  protected pickAccount(id: string): void {
    if (!id) this.only.set(false);
    else if (!this.account()) this.only.set(true);
    this.account.set(id);
  }

  protected removeText(s: ServerStatus): string {
    return (
      (s.running ? 'It is stopped first. ' : '') +
      'Its conversations stay on disk, and the main server and the others carry on.'
    );
  }

  protected async save(e: Event): Promise<void> {
    e.preventDefault();
    this.nameTouched.set(true);
    if (!this.canSave()) return;
    const s = this.server();
    const name = this.name().trim();
    const port = parsePort(this.port());
    this.busy.set(true);
    this.error.set('');
    try {
      const reply = s
        ? await this.api.updateBridgeServer(s.id, {
            ...(name !== s.name ? { name } : {}),
            ...(port !== undefined && port !== s.port ? { port } : {}),
          })
        : await this.api.addBridgeServer({
            name,
            ...(port !== undefined ? { port } : {}),
            defaults: this.defaults(),
          });
      this.saved.emit(reply);
    } catch (err) {
      this.error.set(
        serverErrorText(err, { server: s?.name ?? name, servers: this.servers(), saving: true }),
      );
    } finally {
      this.busy.set(false);
    }
  }

  protected async remove(s: ServerStatus): Promise<void> {
    if (this.busy()) return;
    this.busy.set(true);
    this.removeError.set('');
    try {
      await this.api.removeBridgeServer(s.id);
      this.removed.emit(s.id);
    } catch (err) {
      // Removed elsewhere meanwhile: what was asked for.
      if (err instanceof ApiError && err.code === 'server_not_found') {
        this.removed.emit(s.id);
        return;
      }
      this.removeError.set(serverErrorText(err, { server: s.name, servers: this.servers() }));
      this.confirmRemove.set(false);
    } finally {
      this.busy.set(false);
    }
  }

  /** The new server's defaults: the main server's, as changed here (not
   * its conversation: that one is the main server's). */
  private defaults(): ServerDefaults {
    const base = normalizeDefaults(this.mainDefaults());
    const account = this.account() || undefined;
    const sources = this.isolate()
      ? []
      : base.settingSources?.length
        ? base.settingSources
        : undefined;
    return {
      cwd: this.cwd().trim() || undefined,
      model: this.model() || undefined,
      effort: this.effort() || undefined,
      thinking: (this.thinking() || undefined) as 'on' | 'off' | undefined,
      permissionMode: this.permissionMode() || undefined,
      settingSources: sources,
      systemPromptAppend: base.systemPromptAppend,
      account,
      accountOnly: account && this.only() ? true : undefined,
    };
  }
}
