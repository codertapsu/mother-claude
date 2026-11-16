import {
  ChangeDetectionStrategy,
  Component,
  computed,
  effect,
  inject,
  signal,
  untracked,
} from '@angular/core';
import { RouterLink } from '@angular/router';

import { ApiService } from '../../core/api.service';
import { computerNoun, tr } from '../../core/i18n/i18n';
import { TPipe } from '../../core/i18n/t.pipe';
import { MachineContext } from '../../core/machine-context';
import { McpServerStatus } from '../../core/models';
import { NoticeService } from '../../core/notice.service';
import {
  PluginReload,
  SessionPlugin,
  SessionPluginsView,
  mcpReconnectable,
  mcpStatusText,
} from '../../core/plugins';
import { CopyButtonComponent } from '../../shared/copy-button.component';
import { IconComponent } from '../../shared/icon.component';
import { SessionContext } from './session-context';

/**
 * A running session's MCP servers and plugins, on its Info tab
 * (docs/PLUGINS.md). A session Mother Claude runs: each server with its
 * status and why it failed, *Reconnect* for one that failed (say because
 * the app it talks to, a Figma plugin, was not open yet when the session
 * started), *Turn off* / *Turn on* for this session only; its plugins and
 * the errors loading them; *Reload plugins* (refused while it works; when
 * its next message would re-read the whole conversation, it says so and
 * asks); *Add a plugin folder to this session…* (the dangerous right). A
 * session in a tmux pane gets /reload-plugins typed; any other says to type
 * it there, with the command to copy. *Manage plugins…* opens the Plugins
 * page (this computer's sessions).
 */
@Component({
  selector: 'mc-session-plugins',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [CopyButtonComponent, IconComponent, RouterLink, TPipe],
  template: `
    <section class="card" aria-labelledby="sp-mcp">
      <div class="head">
        <h2 id="sp-mcp" class="first">{{ 'MCP servers' | t }}</h2>
        @if (owned()) {
          <button class="btn ghost small" type="button" [disabled]="loading()" (click)="refresh()">
            <mc-icon name="refresh" [size]="14" /> {{ 'Refresh' | t }}
          </button>
        }
      </div>
      @if (!owned()) {
        <p class="muted small">
          {{
            'Type /mcp in the session itself to see its MCP servers and reconnect one: Mother Claude reads them for the sessions it runs.'
              | t
          }}
        </p>
      } @else if (mcpError()) {
        <p class="muted small">{{ mcpError() }}</p>
      } @else if (servers(); as list) {
        <ul class="mcp">
          @for (m of list; track m.name) {
            <li [attr.data-server]="m.name">
              <mc-icon name="plug" [size]="14" />
              <span class="grow">
                <span class="name">{{ m.name }}</span>
                @if (m.error && m.status !== 'connected') {
                  <span class="muted small why">{{ m.error }}</span>
                } @else if (m.tools) {
                  <span class="muted small why">{{ '{n} tools' | t: { n: m.tools } }}</span>
                }
              </span>
              <span class="mstatus" [attr.data-status]="m.status">{{ statusText(m.status) }}</span>
              @if (reconnectable(m.status)) {
                <button
                  class="btn small"
                  type="button"
                  [disabled]="acting() !== null || !ctx.canAct()"
                  [attr.aria-label]="'Reconnect {name}' | t: { name: m.name }"
                  (click)="reconnect(m)"
                >
                  {{ acting() === m.name ? ('Connecting…' | t) : ('Reconnect' | t) }}
                </button>
              } @else if (m.status === 'disabled') {
                <button
                  class="btn ghost small"
                  type="button"
                  [disabled]="acting() !== null || !ctx.canAct()"
                  [attr.aria-label]="'Turn {name} on in this session' | t: { name: m.name }"
                  (click)="toggle(m, true)"
                >
                  {{ 'Turn on' | t }}
                </button>
              } @else if (m.status === 'connected') {
                <button
                  class="btn ghost small"
                  type="button"
                  [disabled]="acting() !== null || !ctx.canAct()"
                  [attr.aria-label]="'Turn {name} off in this session' | t: { name: m.name }"
                  (click)="toggle(m, false)"
                >
                  {{ 'Turn off' | t }}
                </button>
              }
            </li>
          } @empty {
            <li class="muted">{{ 'No MCP servers.' | t }}</li>
          }
        </ul>
        @if (anyFailed()) {
          <p class="muted small hint">
            {{
              'A server that failed when the session started — for example because the app it talks to was not open yet — connects with Reconnect, without restarting the session.'
                | t
            }}
          </p>
        }
      } @else {
        <p class="muted small">{{ 'Loading…' | t }}</p>
      }
    </section>

    <section class="card" aria-labelledby="sp-plugins">
      <div class="head">
        <h2 id="sp-plugins" class="first">{{ 'Plugins' | t }}</h2>
        @if (via() !== 'none') {
          <button
            class="btn small"
            type="button"
            [disabled]="reloading() || !ctx.canAct()"
            (click)="reload(false)"
          >
            <mc-icon name="refresh" [size]="14" />
            {{ reloading() ? ('Reloading…' | t) : ('Reload plugins' | t) }}
          </button>
        }
      </div>
      @if (held(); as h) {
        <div class="notice warn small held" role="status">
          <p>{{ heldText(h) }}</p>
          <div class="acts">
            <button class="btn small" type="button" [disabled]="reloading()" (click)="reload(true)">
              {{ 'Reload anyway' | t }}
            </button>
            <button class="btn ghost small" type="button" (click)="held.set(null)">
              {{ 'Not now' | t }}
            </button>
          </div>
        </div>
      }
      @if (view(); as v) {
        @if (v.reload === 'none') {
          <p class="muted small typeit">
            {{
              'To load plugin changes in this session, type /reload-plugins in it: Mother Claude can’t type into it where it runs.'
                | t
            }}
            <mc-copy-button text="/reload-plugins" label="Copy the command" />
          </p>
        } @else if (v.reload === 'tmux') {
          <p class="muted small">
            {{ 'Reload plugins types /reload-plugins into its terminal.' | t }}
          </p>
        }
        @if (v.known) {
          <ul class="plugins">
            @for (p of v.plugins; track $index) {
              <li [attr.data-plugin]="p.name">
                <mc-icon name="puzzle" [size]="14" />
                <span class="grow">
                  <span class="name">{{ p.name }}</span>
                  @if (where(p); as w) {
                    <span class="muted small why">{{ w }}</span>
                  }
                </span>
                @if (p.version) {
                  <span class="muted small">{{ p.version }}</span>
                }
              </li>
            } @empty {
              <li class="muted">{{ 'No plugins.' | t }}</li>
            }
          </ul>
          @for (e of v.errors; track $index) {
            <p class="notice error small">{{ e }}</p>
          }
        } @else if (v.reload === 'control') {
          <p class="muted small">
            {{ 'Its plugins show once its first message runs, or after Reload plugins.' | t }}
          </p>
        }
      } @else if (viewError()) {
        <p class="muted small">{{ viewError() }}</p>
      }
      @if (adding()) {
        <form class="add" (submit)="addFolder($event)">
          <label class="field">
            <span>{{ folderLabel() }}</span>
            <input
              type="text"
              name="folder"
              autocomplete="off"
              spellcheck="false"
              [value]="folder()"
              [placeholder]="'/Users/you/plugins/my-plugin' | t"
              (input)="folder.set($any($event.target).value)"
            />
          </label>
          <p class="muted small">
            {{
              'Only this session loads it, until it ends. A plugin can run commands on the computer: add only folders you trust.'
                | t
            }}
          </p>
          <div class="acts">
            <button
              class="btn small primary"
              type="submit"
              [disabled]="!folder().trim() || reloading()"
            >
              {{ reloading() ? ('Adding…' | t) : ('Add it' | t) }}
            </button>
            <button class="btn ghost small" type="button" (click)="adding.set(false)">
              {{ 'Cancel' | t }}
            </button>
          </div>
        </form>
      }
      <div class="acts more">
        @if (via() === 'control' && ctx.canDangerous() && !adding()) {
          <button class="btn ghost small" type="button" (click)="adding.set(true)">
            <mc-icon name="plus" [size]="14" /> {{ 'Add a plugin folder to this session…' | t }}
          </button>
        }
        @if (!machine) {
          <a class="btn ghost small" routerLink="/plugins">
            <mc-icon name="puzzle" [size]="14" /> {{ 'Manage plugins…' | t }}
          </a>
        }
      </div>
    </section>
  `,
  styles: [
    `
      .head {
        display: flex;
        align-items: center;
        justify-content: space-between;
        gap: 0.5rem;
      }
      .head h2 {
        margin: 0;
      }
      ul {
        list-style: none;
        margin: 0.5rem 0 0;
        padding: 0;
      }
      li {
        display: flex;
        align-items: center;
        gap: 0.5rem;
        padding: 0.45rem 0;
        border-top: 1px solid var(--border);
        min-width: 0;
      }
      li:first-child {
        border-top: 0;
      }
      li mc-icon {
        color: var(--muted);
        flex: 0 0 auto;
      }
      .grow {
        flex: 1 1 auto;
        min-width: 0;
        display: flex;
        flex-direction: column;
      }
      .name {
        overflow-wrap: anywhere;
      }
      .why {
        overflow-wrap: anywhere;
      }
      .mstatus {
        font-size: 0.78rem;
        color: var(--muted);
        white-space: nowrap;
      }
      .mstatus[data-status='connected'] {
        color: var(--green);
      }
      .mstatus[data-status='failed'],
      .mstatus[data-status='needs-auth'] {
        color: var(--red);
      }
      .hint,
      .typeit {
        margin: 0.5rem 0 0;
      }
      .typeit mc-copy-button {
        vertical-align: middle;
      }
      .held {
        margin: 0.5rem 0;
      }
      .held p {
        margin: 0 0 0.4rem;
      }
      .acts {
        display: flex;
        flex-wrap: wrap;
        gap: 0.4rem;
      }
      .more {
        margin-top: 0.6rem;
      }
      .add {
        margin-top: 0.6rem;
      }
      .field {
        display: flex;
        flex-direction: column;
        gap: 0.25rem;
      }
      .field input {
        font: inherit;
        font-family: var(--mono);
        font-size: 0.85rem;
      }
      .notice {
        margin: 0.4rem 0 0;
      }
    `,
  ],
})
export class SessionPluginsComponent {
  protected readonly ctx = inject(SessionContext);
  private readonly api = inject(ApiService);
  private readonly notices = inject(NoticeService);
  /** Another computer's session (none: this computer's). */
  protected readonly machine = inject(MachineContext, { optional: true });

  /** Mother Claude runs it: its servers and plugins are asked of it. */
  protected readonly owned = computed(() => {
    const s = this.ctx.session();
    return !!s?.owned && !!s.live;
  });
  protected readonly servers = signal<McpServerStatus[] | null>(null);
  protected readonly mcpError = signal('');
  protected readonly view = signal<SessionPluginsView | null>(null);
  protected readonly viewError = signal('');
  protected readonly loading = signal(false);
  /** The server a reconnect or switch is on its way for. */
  protected readonly acting = signal<string | null>(null);
  protected readonly reloading = signal(false);
  /** A reload not done: what it would change. */
  protected readonly held = signal<PluginReload | null>(null);
  protected readonly adding = signal(false);
  protected readonly folder = signal('');

  protected readonly via = computed(() => this.view()?.reload ?? 'none');
  protected readonly anyFailed = computed(() =>
    (this.servers() ?? []).some((m) => mcpReconnectable(m.status)),
  );
  protected readonly statusText = mcpStatusText;
  protected readonly reconnectable = mcpReconnectable;
  protected readonly folderLabel = computed(() =>
    this.machine
      ? tr("The plugin's folder on {name}", { name: this.machine.name() })
      : tr("The plugin's folder on this {computer}", { computer: computerNoun() }),
  );

  constructor() {
    // Read once per session (and again once Mother Claude runs it).
    let read = '';
    effect(() => {
      const id = this.ctx.id();
      const owned = this.owned();
      const live = !!this.ctx.session()?.live;
      const key = `${id}|${owned}|${live}`;
      if (!id || !live || key === read) return;
      read = key;
      untracked(() => void this.load(id, owned));
    });
  }

  private async load(id: string, owned: boolean): Promise<void> {
    this.loading.set(true);
    try {
      await Promise.all([owned ? this.loadServers(id) : Promise.resolve(), this.loadView(id)]);
    } finally {
      this.loading.set(false);
    }
  }

  private async loadServers(id: string): Promise<void> {
    try {
      this.servers.set((await this.api.getMcp(id)).servers);
      this.mcpError.set('');
    } catch {
      this.mcpError.set(tr('MCP status is not available for this session.'));
    }
  }

  private async loadView(id: string): Promise<void> {
    try {
      this.view.set(await this.api.getSessionPlugins(id));
      this.viewError.set('');
    } catch {
      this.viewError.set(tr('Its plugins could not be read.'));
    }
  }

  protected refresh(): void {
    const id = this.ctx.id();
    if (id) void this.load(id, this.owned());
  }

  /** Where a plugin comes from, in words. */
  protected where(p: SessionPlugin): string {
    const source = p.source ?? '';
    if (source.endsWith('@inline') || (!source.includes('@') && p.path)) {
      return tr('Loaded for this session');
    }
    const market = source.split('@')[1];
    return market ? tr('From {marketplace}', { marketplace: market }) : '';
  }

  protected async reconnect(m: McpServerStatus): Promise<void> {
    const id = this.ctx.id();
    if (!id || this.acting()) return;
    this.acting.set(m.name);
    try {
      const after = await this.api.reconnectMcp(id, m.name);
      this.servers.set(after);
      const now = after.find((x) => x.name === m.name);
      if (now?.status === 'connected') {
        this.notices.success(tr('{name} is connected.', { name: m.name }));
      } else {
        this.notices.show(
          'warning',
          tr('{name} did not connect: {why}', {
            name: m.name,
            why: now?.error || mcpStatusText(now?.status ?? ''),
          }),
        );
      }
    } catch (e) {
      this.notices.error(e, () => void this.reconnect(m), tr('Reconnect {name}', { name: m.name }));
    } finally {
      this.acting.set(null);
    }
  }

  protected async toggle(m: McpServerStatus, enabled: boolean): Promise<void> {
    const id = this.ctx.id();
    if (!id || this.acting()) return;
    this.acting.set(m.name);
    try {
      this.servers.set(await this.api.setMcpEnabled(id, m.name, enabled));
    } catch (e) {
      this.notices.error(e, () => void this.toggle(m, enabled), m.name);
    } finally {
      this.acting.set(null);
    }
  }

  protected async reload(force: boolean): Promise<void> {
    const id = this.ctx.id();
    if (!id || this.reloading()) return;
    this.reloading.set(true);
    try {
      const r = await this.api.reloadSessionPlugins(id, force);
      this.applied(r, tr('Plugins reloaded.'));
    } catch (e) {
      this.notices.error(e, () => void this.reload(force), tr('Reload plugins'));
    } finally {
      this.reloading.set(false);
    }
  }

  protected async addFolder(e: Event): Promise<void> {
    e.preventDefault();
    const id = this.ctx.id();
    const path = this.folder().trim();
    if (!id || !path || this.reloading()) return;
    this.reloading.set(true);
    try {
      const r = await this.api.addSessionPlugin(id, path);
      this.adding.set(false);
      this.folder.set('');
      this.applied(r, tr('This session loaded the plugin.'));
    } catch (err) {
      this.notices.error(err, undefined, tr('Add a plugin folder'));
    } finally {
      this.reloading.set(false);
    }
  }

  /** What a reload answered: held (asked about), typed, or done. */
  private applied(r: PluginReload, done: string): void {
    if (r.held) {
      this.held.set(r);
      return;
    }
    this.held.set(null);
    if (r.typed) {
      this.notices.success(tr('/reload-plugins typed into the session.'));
      return;
    }
    const v = this.view();
    if (v) this.view.set({ ...v, plugins: r.plugins, known: true });
    if (r.servers.length) this.servers.set(r.servers);
    this.notices.success(
      r.errorCount ? tr('{done} {n} could not be loaded.', { done, n: r.errorCount }) : done,
    );
  }

  protected heldText(r: PluginReload): string {
    const added = r.cacheImpact?.serversAdded ?? [];
    const removed = r.cacheImpact?.serversRemoved ?? [];
    const what = [
      ...added.map((n) => tr('adds {name}', { name: n })),
      ...removed.map((n) => tr('removes {name}', { name: n })),
    ].join(', ');
    return what
      ? tr(
          'Reloading {what}: Claude then re-reads the whole conversation with its next message, which costs more. Reload anyway?',
          { what },
        )
      : tr(
          'Reloading changes its tools: Claude then re-reads the whole conversation with its next message, which costs more. Reload anyway?',
        );
  }
}
