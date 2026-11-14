import { Component, DestroyRef, computed, effect, inject, signal, untracked } from '@angular/core';

import { DEFAULT_ACCOUNT } from '../../core/accounts';
import { AccountsService } from '../../core/accounts.service';
import { ApiService, errorMessage } from '../../core/api.service';
import { AuthService } from '../../core/auth.service';
import { tr } from '../../core/i18n/i18n';
import { TPipe } from '../../core/i18n/t.pipe';
import { AccountView, ForeignControlStatus } from '../../core/models';
import { RealtimeService } from '../../core/realtime.service';
import { PageHeadComponent } from '../../shared/page-head.component';
import { RelativeTimePipe } from '../../shared/time.pipes';
import { shortPath } from '../../shared/util';

/** How the plugin stands in one config directory. */
type PluginState = 'installed' | 'off' | 'missing' | 'unknown';

/** One config directory: the Default's, or a directory account's. */
interface Row {
  /** The account (null while the accounts are not read: the Default). */
  account: AccountView | null;
  id: string;
  name: string;
  dir: string;
  state: PluginState;
  version?: string;
  /** Live sessions whose hooks report here. */
  reporting: number;
  lastEventAt?: number;
}

/**
 * *Sessions started elsewhere* (docs/UI-REDESIGN.md §6.7), a place of its
 * own since 2026-11-14 (§11; Settings' before): the
 * Mother Claude plugin that lets you answer the sessions you start in a
 * terminal, VS Code or JetBrains from here — approve them, answer their
 * questions, send them messages, interrupt them — in every config
 * directory, in one table: the Default's (Remote's *foreign-session
 * control* before) and each directory account's (Accounts' *Install hooks
 * plugin* before), with the sessions reporting and the last hook event.
 * Installed with `claude plugin install`, never by editing settings.json.
 * The desktop installs; another device reads.
 */
@Component({
  selector: 'mc-settings-elsewhere',
  imports: [PageHeadComponent, RelativeTimePipe, TPipe],
  template: `
    <div class="page narrow">
      <mc-page-head
        [heading]="'Sessions started elsewhere' | t"
        [lede]="
          'Answer the sessions you start in a terminal, VS Code or JetBrains from here: the Mother Claude plugin lets you approve them, answer their questions, send them messages and interrupt them.'
            | t
        "
        help="foreign-sessions"
      />

      @if (error()) {
        <p class="notice error small" role="alert">{{ error() }}</p>
      }
      @if (notice()) {
        <p class="notice ok small" role="status">{{ notice() }}</p>
      }

      <p class="muted small how">
        {{
          'Without it you can watch these sessions here but not answer them. It is a user-scope Claude Code plugin, installed with claude plugin install: your settings.json hooks are never touched. The session’s own window keeps working; whoever answers first wins.'
            | t
        }}
      </p>

      @if (foreign(); as f) {
        @if (!f.cliAvailable) {
          <p class="notice warn small">
            {{ 'Claude Code’s claude command is not found on PATH: it installs the plugin.' | t }}
          </p>
        }
      }

      <section class="card table-card" [attr.aria-label]="'Sessions started elsewhere' | t">
        <table class="plugins">
          <caption class="visually-hidden">
            {{
              'The Mother Claude plugin in each Claude account’s directory' | t
            }}
          </caption>
          <thead>
            <tr>
              <th scope="col">{{ 'Claude account' | t }}</th>
              <th scope="col">{{ 'Plugin' | t }}</th>
              <th scope="col">{{ 'Sessions reporting' | t }}</th>
              <th scope="col">{{ 'Last hook event' | t }}</th>
              @if (isDesktop()) {
                <th scope="col">
                  <span class="visually-hidden">{{ 'Action' | t }}</span>
                </th>
              }
            </tr>
          </thead>
          <tbody>
            @for (r of rows(); track r.id) {
              <tr [attr.data-state]="r.state">
                <th scope="row">
                  <span class="name">{{ r.name }}</span>
                  @if (r.dir) {
                    <span class="muted small mono dir" [title]="r.dir">{{ short(r.dir) }}</span>
                  }
                </th>
                <td>
                  <span class="state">{{ stateText(r) }}</span>
                  @if (r.version) {
                    <span class="muted small"> v{{ r.version }}</span>
                  }
                </td>
                <td>{{ r.reporting }}</td>
                <td>{{ r.lastEventAt ? (r.lastEventAt | relTime) : ('none yet' | t) }}</td>
                @if (isDesktop()) {
                  <td class="act">
                    @if (r.state === 'missing') {
                      <button
                        class="btn small primary"
                        type="button"
                        [disabled]="!!busy() || !canInstall()"
                        (click)="install(r)"
                      >
                        {{ busy() === r.id ? ('Installing…' | t) : ('Install' | t) }}
                      </button>
                    } @else if (r.state !== 'unknown') {
                      <button
                        class="btn small"
                        type="button"
                        [class.danger]="armed() === r.id"
                        [disabled]="!!busy()"
                        (click)="uninstall(r)"
                      >
                        {{
                          busy() === r.id
                            ? ('Removing…' | t)
                            : armed() === r.id
                              ? ('Tap again to uninstall' | t)
                              : ('Uninstall' | t)
                        }}
                      </button>
                    }
                  </td>
                }
              </tr>
            }
          </tbody>
        </table>
      </section>

      @if (foreign(); as f) {
        @if (f.sessionsNeedingReload) {
          <p class="small note">
            {{
              f.sessionsNeedingReload === 1
                ? ('{n} running session was started before the install: run' | t: { n: 1 })
                : ('{n} running sessions were started before the install: run'
                  | t: { n: f.sessionsNeedingReload })
            }}
            <code>/reload-plugins</code> {{ 'in them, or restart them.' | t }}
          </p>
        }
        @if (f.lastError) {
          <p class="error-text small">{{ f.lastError }}</p>
        }
        @if (f.notes.length) {
          <ul class="notes small">
            @for (n of f.notes; track $index) {
              <li>{{ n }}</li>
            }
          </ul>
        }
      }
      @if (!isDesktop()) {
        <p class="muted small">{{ 'Installed and removed on the desktop app.' | t }}</p>
      }
    </div>
  `,
  styles: [
    `
      .how {
        margin: 0 0 0.75rem;
      }
      .table-card {
        padding: 0;
        overflow-x: auto;
      }
      .plugins {
        width: 100%;
        border-collapse: collapse;
        font-size: 0.875rem;
      }
      .plugins th,
      .plugins td {
        text-align: left;
        padding: 0.6rem 0.75rem;
        border-bottom: 1px solid var(--border);
        vertical-align: top;
      }
      .plugins tbody tr:last-child th,
      .plugins tbody tr:last-child td {
        border-bottom: 0;
      }
      .plugins thead th {
        font-size: 0.75rem;
        color: var(--muted);
        font-weight: 600;
      }
      .plugins th[scope='row'] {
        display: flex;
        flex-direction: column;
        gap: 0.1rem;
        font-weight: 500;
      }
      .dir {
        overflow-wrap: anywhere;
      }
      tr[data-state='installed'] .state {
        color: var(--green);
      }
      tr[data-state='off'] .state,
      tr[data-state='unknown'] .state {
        color: var(--amber);
      }
      .act {
        text-align: right;
        white-space: nowrap;
      }
      .note,
      .notes {
        margin: 0.6rem 0 0;
      }
    `,
  ],
})
export class ElsewherePageComponent {
  private readonly api = inject(ApiService);
  private readonly realtime = inject(RealtimeService);
  protected readonly accounts = inject(AccountsService);
  protected readonly isDesktop = inject(AuthService).isDesktop;

  /** The Default's plugin (`GET /api/control/foreign`, the desktop's). */
  protected readonly foreign = signal<ForeignControlStatus | null>(null);
  protected readonly error = signal('');
  protected readonly notice = signal('');
  /** The row whose plugin changes now. */
  protected readonly busy = signal<string | null>(null);
  /** The row whose Uninstall was tapped once. */
  protected readonly armed = signal<string | null>(null);
  private armTimer?: ReturnType<typeof setTimeout>;

  /** Claude Code's command is there to install with. */
  protected readonly canInstall = computed(() => this.foreign()?.cliAvailable !== false);

  /** Live sessions with hooks reporting, by their account. */
  private readonly reporting = computed(() => {
    const out = new Map<string, number>();
    for (const s of this.realtime.sessions()) {
      if (!s.live || s.owned || !s.control?.hooksActive) continue;
      const id = s.account?.id ?? DEFAULT_ACCOUNT;
      out.set(id, (out.get(id) ?? 0) + 1);
    }
    return out;
  });

  /** The Default, then each directory account (a token account uses the
   * Default's directory). */
  protected readonly rows = computed<Row[]>(() => {
    const f = this.foreign();
    const list = this.accounts.list();
    const def = list.find((a) => a.isDefault) ?? null;
    const rows: Row[] = [
      {
        account: def,
        id: def?.id ?? DEFAULT_ACCOUNT,
        name: def?.name ?? tr('Default'),
        dir: def?.dir ?? '',
        state: f
          ? f.installed
            ? f.enabled === false
              ? 'off'
              : 'installed'
            : 'missing'
          : 'unknown',
        version: f?.pluginVersion,
        reporting: f?.sessionsWithHooks ?? this.reporting().get(def?.id ?? DEFAULT_ACCOUNT) ?? 0,
        lastEventAt: f?.lastEventAt,
      },
    ];
    for (const a of list) {
      if (a.isDefault || a.kind !== 'directory') continue;
      rows.push({
        account: a,
        id: a.id,
        name: a.name,
        dir: a.dir ?? '',
        state: pluginState(a.hooksPlugin),
        reporting: this.reporting().get(a.id) ?? 0,
      });
    }
    return rows;
  });

  constructor() {
    void this.accounts.ensureLoaded();
    let loaded = false;
    effect(() => {
      if (loaded || !this.isDesktop()) return;
      loaded = true;
      untracked(() => void this.loadForeign());
    });
    inject(DestroyRef).onDestroy(() => clearTimeout(this.armTimer));
  }

  protected short(p: string): string {
    return shortPath(p);
  }

  protected stateText(r: Row): string {
    switch (r.state) {
      case 'installed':
        return tr('On');
      case 'off':
        return tr('Installed, switched off');
      case 'missing':
        return tr('Not installed');
      default:
        return tr('Not known');
    }
  }

  protected async install(r: Row): Promise<void> {
    await this.run(r, async () => {
      if (r.account && !r.account.isDefault) {
        await this.accounts.setHooksPlugin(r.id, true);
      } else {
        const s = await this.api.installForeignControl();
        if (s) this.foreign.set(s);
        else await this.loadForeign();
      }
      this.notice.set(
        tr('Installed for {name}. Its sessions started before need /reload-plugins.', {
          name: r.name,
        }),
      );
    });
  }

  protected async uninstall(r: Row): Promise<void> {
    if (this.armed() !== r.id) {
      this.armed.set(r.id);
      clearTimeout(this.armTimer);
      this.armTimer = setTimeout(() => this.armed.set(null), 4000);
      return;
    }
    this.armed.set(null);
    await this.run(r, async () => {
      if (r.account && !r.account.isDefault) {
        await this.accounts.setHooksPlugin(r.id, false);
      } else {
        const s = await this.api.uninstallForeignControl();
        if (s) this.foreign.set(s);
        else await this.loadForeign();
      }
      this.notice.set(tr('Removed for {name}.', { name: r.name }));
    });
  }

  private async loadForeign(): Promise<void> {
    try {
      this.foreign.set(await this.api.getForeignControl());
    } catch (e) {
      this.error.set(errorMessage(e));
    }
  }

  private async run(r: Row, fn: () => Promise<void>): Promise<void> {
    if (this.busy()) return;
    this.busy.set(r.id);
    this.error.set('');
    this.notice.set('');
    try {
      await fn();
    } catch (e) {
      this.error.set(`${r.name}: ${errorMessage(e)}`);
    } finally {
      this.busy.set(null);
    }
  }
}

/** An account's plugin state as the accounts list reports it. */
function pluginState(s: string | undefined): PluginState {
  return s === 'installed' || s === 'off' || s === 'missing' ? s : 'unknown';
}
