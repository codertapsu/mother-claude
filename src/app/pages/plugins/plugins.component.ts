import {
  ChangeDetectionStrategy,
  Component,
  DestroyRef,
  computed,
  inject,
  signal,
} from '@angular/core';
import { RouterLink } from '@angular/router';

import { DEFAULT_ACCOUNT } from '../../core/accounts';
import { AccountsService } from '../../core/accounts.service';
import { ApiService, describeError } from '../../core/api.service';
import { AuthService } from '../../core/auth.service';
import { tr } from '../../core/i18n/i18n';
import { TPipe } from '../../core/i18n/t.pipe';
import { ProjectInfo } from '../../core/models';
import { NoticeService } from '../../core/notice.service';
import {
  AvailablePlugin,
  CatalogView,
  InstalledPlugin,
  Marketplace,
  PluginScope,
  PluginsView,
  ReloadAllResult,
  scopeText,
} from '../../core/plugins';
import { IconComponent } from '../../shared/icon.component';
import { PageHeadComponent } from '../../shared/page-head.component';
import { PluginInstallSheetComponent } from './install-sheet.component';

/** How long typing waits before the catalog is searched. */
const SEARCH_DEBOUNCE_MS = 300;

/** What a reload of every running session did, in words. */
export function reloadAllText(r: ReloadAllResult): string {
  const parts: string[] = [];
  if (r.reloaded) parts.push(tr('{n} reloaded', { n: r.reloaded }));
  if (r.busy) parts.push(tr('{n} working (reload them once they finish)', { n: r.busy }));
  if (r.held) {
    parts.push(
      tr('{n} left as they are: their next message would re-read the whole conversation', {
        n: r.held,
      }),
    );
  }
  if (r.failed) parts.push(tr('{n} could not reload', { n: r.failed }));
  return parts.length
    ? parts.join(' · ')
    : tr(
        'No session Mother Claude runs needed it: others load plugins when they start, or with /reload-plugins.',
      );
}

/**
 * *Plugins* (docs/PLUGINS.md): Claude Code's plugins — what is installed
 * and where it is on (everywhere, a project for everyone, a project only
 * for you; a project's own read with *Show a project*), turned on or off,
 * updated or removed; the marketplace's plugins, searched and installed
 * (`mc-plugin-install-sheet`: where, and a marketplace's command shown
 * first); the marketplaces, added (GitHub owner/repo, a git URL or a folder)
 * or removed. Every change runs `claude plugin …` on this computer, from
 * the desktop; a phone sees the same, read only. After a change, the
 * sessions Mother Claude runs can read their plugins again at once.
 * Mother Claude's own plugin is shown with a link to its place.
 */
@Component({
  selector: 'mc-plugins',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [IconComponent, PageHeadComponent, PluginInstallSheetComponent, RouterLink, TPipe],
  template: `
    <div class="page narrow">
      <mc-page-head
        [heading]="'Plugins' | t"
        [lede]="
          'Plugins add skills, commands, agents, hooks and MCP servers to Claude Code. Install one for every session, or for one project.'
            | t
        "
        help="plugins"
      />

      <section class="card intro">
        <p class="small">
          <mc-icon name="info" [size]="14" />
          {{
            'Claude Code loads plugins when a session starts. After a change here, the sessions Mother Claude runs can load it at once; a session started elsewhere loads it with /reload-plugins.'
              | t
          }}
        </p>
        <p class="small muted">
          {{
            'A plugin can run commands on this computer (its hooks and MCP servers). Install plugins only from marketplaces you trust.'
              | t
          }}
        </p>
        @if (!canChange()) {
          <p class="notice small">
            {{
              'You can look here; plugins are installed and changed in Mother Claude on the computer itself.'
                | t
            }}
          </p>
        }
      </section>

      @if (homes().length > 1) {
        <label class="field account">
          <span>{{ 'Claude account' | t }}</span>
          <select [value]="account()" (change)="pickAccount($any($event.target).value)">
            @for (a of homes(); track a.id) {
              <option [value]="a.id">{{ a.name }}</option>
            }
          </select>
        </label>
      }

      @if (changed()) {
        <div class="notice small changed" role="status">
          <span>{{
            'Running sessions load this change when they read their plugins again.' | t
          }}</span>
          @if (canChange()) {
            <button
              class="btn small"
              type="button"
              [disabled]="busy() === 'reload'"
              (click)="reloadAll()"
            >
              {{ busy() === 'reload' ? ('Reloading…' | t) : ('Reload in running sessions' | t) }}
            </button>
          }
        </div>
      }

      <!-- Installed -->
      <section class="card" aria-labelledby="pl-installed">
        <div class="head">
          <h2 id="pl-installed">{{ 'Installed' | t }}</h2>
          <label class="field inline">
            <span class="muted small">{{ 'Show a project' | t }}</span>
            <select [value]="project()" (change)="pickProject($any($event.target).value)">
              <option value="">{{ 'Everywhere only' | t }}</option>
              @for (pr of projects(); track pr.path) {
                <option [value]="pr.path">{{ pr.name }}</option>
              }
            </select>
          </label>
        </div>
        @if (loadError()) {
          <p class="notice error small" role="alert">{{ loadError() }}</p>
        } @else if (!view()) {
          <p class="muted small">{{ 'Loading…' | t }}</p>
        } @else {
          <ul class="list">
            @for (p of view()!.installed; track p.id + p.scope) {
              <li class="plugin" [attr.data-id]="p.id" [attr.data-scope]="p.scope">
                <mc-icon name="puzzle" [size]="16" />
                <div class="grow">
                  <div class="line">
                    <strong>{{ p.name }}</strong>
                    @if (p.version) {
                      <span class="muted small">{{ p.version }}</span>
                    }
                    <span class="chip">{{ scope(p.scope) }}</span>
                    @if (!p.enabled) {
                      <span class="chip off">{{ 'Off' | t }}</span>
                    }
                  </div>
                  @if (p.marketplace) {
                    <span class="muted small">{{
                      'From {marketplace}' | t: { marketplace: p.marketplace }
                    }}</span>
                  }
                  @if (p.mcpServers.length) {
                    <span class="muted small">
                      {{ 'MCP servers: {names}' | t: { names: serverNames(p) } }}
                    </span>
                  }
                  @for (n of p.notes; track $index) {
                    <span class="muted small">{{ n }}</span>
                  }
                  @if (p.ours) {
                    <a class="small" routerLink="/elsewhere">{{
                      "Mother Claude's own: turned on and off in Sessions started elsewhere." | t
                    }}</a>
                  } @else if (!p.changeable) {
                    <span class="muted small">{{
                      'Changed where it comes from, not here.' | t
                    }}</span>
                  }
                </div>
                @if (canChange() && p.changeable && !p.ours) {
                  <div class="acts">
                    @if (confirming() === key(p)) {
                      <button
                        class="btn danger small"
                        type="button"
                        [disabled]="!!busy()"
                        (click)="uninstall(p)"
                      >
                        {{ busy() === key(p) ? ('Removing…' | t) : ('Remove' | t) }}
                      </button>
                      <button class="btn ghost small" type="button" (click)="confirming.set(null)">
                        {{ 'Keep it' | t }}
                      </button>
                    } @else {
                      <button
                        class="btn ghost small"
                        type="button"
                        [disabled]="!!busy()"
                        [attr.aria-label]="
                          (p.enabled ? 'Turn {name} off' : 'Turn {name} on') | t: { name: p.name }
                        "
                        (click)="setEnabled(p, !p.enabled)"
                      >
                        {{ p.enabled ? ('Turn off' | t) : ('Turn on' | t) }}
                      </button>
                      <button
                        class="btn ghost small"
                        type="button"
                        [disabled]="!!busy()"
                        (click)="update(p)"
                      >
                        {{ busy() === 'update:' + key(p) ? ('Updating…' | t) : ('Update' | t) }}
                      </button>
                      <button
                        class="btn ghost small"
                        type="button"
                        [disabled]="!!busy()"
                        [attr.aria-label]="'Remove {name}' | t: { name: p.name }"
                        (click)="confirming.set(key(p))"
                      >
                        {{ 'Remove…' | t }}
                      </button>
                    }
                  </div>
                }
              </li>
            } @empty {
              <li class="muted small">{{ 'No plugins installed yet. Find one below.' | t }}</li>
            }
          </ul>
        }
      </section>

      <!-- Find -->
      <section class="card" aria-labelledby="pl-find">
        <h2 id="pl-find">{{ 'Find a plugin' | t }}</h2>
        <input
          class="search"
          type="search"
          [attr.aria-label]="'Search the marketplaces' | t"
          [placeholder]="'Search by name or what it does' | t"
          [value]="query()"
          (input)="search($any($event.target).value)"
        />
        @if (catalogError()) {
          <p class="notice error small" role="alert">{{ catalogError() }}</p>
        } @else if (!catalog()) {
          <p class="muted small">{{ 'Loading…' | t }}</p>
        } @else {
          <ul class="list">
            @for (a of catalog()!.plugins; track a.id) {
              <li class="offer" [attr.data-id]="a.id">
                <mc-icon name="puzzle" [size]="16" />
                <div class="grow">
                  <div class="line">
                    <strong>{{ a.name }}</strong>
                    <span class="muted small">{{ a.marketplace }}</span>
                    @if (a.installCount) {
                      <span class="muted small">{{
                        '{n} installs' | t: { n: a.installCount }
                      }}</span>
                    }
                  </div>
                  @if (a.description) {
                    <span class="small desc">{{ a.description }}</span>
                  }
                </div>
                @if (installedIds().has(a.id)) {
                  <span class="chip">{{ 'Installed' | t }}</span>
                } @else if (canChange()) {
                  <button class="btn small" type="button" (click)="installing.set(a)">
                    {{ 'Install…' | t }}
                  </button>
                }
              </li>
            } @empty {
              <li class="muted small">{{ 'No plugin matches.' | t }}</li>
            }
          </ul>
          @if (catalog()!.total > catalog()!.plugins.length) {
            <p class="muted small">
              {{
                'Showing {shown} of {total}: search to narrow it down.'
                  | t: { shown: catalog()!.plugins.length, total: catalog()!.total }
              }}
            </p>
          }
        }
      </section>

      <!-- Marketplaces -->
      <section class="card" aria-labelledby="pl-markets">
        <h2 id="pl-markets">{{ 'Marketplaces' | t }}</h2>
        <p class="muted small">{{ 'Where plugins come from.' | t }}</p>
        @if (view(); as v) {
          <ul class="list">
            @for (m of v.marketplaces; track m.name) {
              <li class="market" [attr.data-name]="m.name">
                <mc-icon name="layers" [size]="16" />
                <div class="grow">
                  <strong>{{ m.name }}</strong>
                  @if (m.location) {
                    <span class="muted small loc">{{ m.location }}</span>
                  }
                </div>
                @if (canChange() && !m.ours) {
                  @if (confirming() === 'm:' + m.name) {
                    <button
                      class="btn danger small"
                      type="button"
                      [disabled]="!!busy()"
                      (click)="removeMarket(m)"
                    >
                      {{ 'Remove' | t }}
                    </button>
                    <button class="btn ghost small" type="button" (click)="confirming.set(null)">
                      {{ 'Keep it' | t }}
                    </button>
                  } @else {
                    <button
                      class="btn ghost small"
                      type="button"
                      [disabled]="!!busy()"
                      [attr.aria-label]="'Remove {name}' | t: { name: m.name }"
                      (click)="confirming.set('m:' + m.name)"
                    >
                      {{ 'Remove…' | t }}
                    </button>
                  }
                }
              </li>
            } @empty {
              <li class="muted small">{{ 'No marketplaces.' | t }}</li>
            }
          </ul>
        }
        @if (canChange()) {
          <form class="add" (submit)="addMarket($event)">
            <label class="field">
              <span>{{ 'Add a marketplace' | t }}</span>
              <input
                type="text"
                name="source"
                autocomplete="off"
                spellcheck="false"
                [placeholder]="'owner/repo, https://… or a folder' | t"
                [value]="source()"
                (input)="source.set($any($event.target).value)"
              />
            </label>
            <p class="muted small">
              {{
                'A GitHub repository (owner/repo), a git address (https://…), or the full path of a folder on this computer.'
                  | t
              }}
            </p>
            <button class="btn small" type="submit" [disabled]="!source().trim() || !!busy()">
              {{ busy() === 'market' ? ('Adding…' | t) : ('Add' | t) }}
            </button>
          </form>
        }
      </section>

      <mc-plugin-install-sheet
        [plugin]="installing()"
        [account]="accountParam()"
        [initialProject]="project()"
        (closed)="installing.set(null)"
        (installed)="installed($event)"
      />
    </div>
  `,
  styles: [
    `
      .intro p {
        margin: 0 0 0.4rem;
      }
      .intro mc-icon {
        vertical-align: -2px;
        color: var(--accent-text);
      }
      .head {
        display: flex;
        align-items: center;
        justify-content: space-between;
        flex-wrap: wrap;
        gap: 0.5rem;
      }
      h2 {
        margin: 0 0 0.4rem;
      }
      .field {
        display: flex;
        flex-direction: column;
        gap: 0.25rem;
      }
      .field.inline {
        flex-direction: row;
        align-items: center;
        gap: 0.4rem;
      }
      .account {
        margin: 0 0 0.75rem;
      }
      .changed {
        display: flex;
        align-items: center;
        justify-content: space-between;
        flex-wrap: wrap;
        gap: 0.5rem;
        margin: 0 0 0.75rem;
      }
      .list {
        list-style: none;
        margin: 0.5rem 0 0;
        padding: 0;
      }
      .list li {
        display: flex;
        align-items: flex-start;
        gap: 0.6rem;
        padding: 0.6rem 0;
        border-top: 1px solid var(--border);
      }
      .list li:first-child {
        border-top: 0;
      }
      .list mc-icon {
        color: var(--muted);
        margin-top: 0.15rem;
        flex: 0 0 auto;
      }
      .grow {
        flex: 1 1 auto;
        min-width: 0;
        display: flex;
        flex-direction: column;
        gap: 0.15rem;
      }
      .line {
        display: flex;
        flex-wrap: wrap;
        align-items: baseline;
        gap: 0.45rem;
      }
      .line strong {
        font-weight: 500;
      }
      .desc,
      .loc {
        overflow-wrap: anywhere;
      }
      .chip {
        font-size: 0.72rem;
        padding: 0.05rem 0.45rem;
        border-radius: 1rem;
        border: 1px solid var(--border);
        color: var(--muted);
        white-space: nowrap;
      }
      .chip.off {
        border-color: var(--amber-border);
        color: var(--amber);
      }
      .acts {
        display: flex;
        flex-wrap: wrap;
        gap: 0.35rem;
        justify-content: flex-end;
      }
      .search {
        width: 100%;
        font: inherit;
      }
      .add {
        margin-top: 0.75rem;
      }
      .add input {
        font: inherit;
        font-family: var(--mono);
        font-size: 0.85rem;
      }
      .add p {
        margin: 0.3rem 0 0.5rem;
      }
      @media (max-width: 767px) {
        .list li {
          flex-wrap: wrap;
        }
        .acts {
          width: 100%;
          justify-content: flex-start;
        }
      }
    `,
  ],
})
export class PluginsComponent {
  private readonly api = inject(ApiService);
  private readonly auth = inject(AuthService);
  private readonly accounts = inject(AccountsService);
  private readonly notices = inject(NoticeService);

  protected readonly view = signal<PluginsView | null>(null);
  protected readonly loadError = signal('');
  protected readonly catalog = signal<CatalogView | null>(null);
  protected readonly catalogError = signal('');
  protected readonly projects = signal<ProjectInfo[]>([]);
  /** The project whose own plugins show ('' : none). */
  protected readonly project = signal('');
  protected readonly account = signal(DEFAULT_ACCOUNT);
  protected readonly query = signal('');
  protected readonly source = signal('');
  /** What runs now: a plugin's key, `update:<key>`, `market`, `reload`. */
  protected readonly busy = signal<string | null>(null);
  protected readonly confirming = signal<string | null>(null);
  protected readonly installing = signal<AvailablePlugin | null>(null);
  /** Something changed: running sessions may read their plugins again. */
  protected readonly changed = signal(false);

  /** Changes run on this computer, from its own window. */
  protected readonly canChange = this.auth.isDesktop;
  /** The accounts with plugins of their own (a token account uses the
   * Default's). */
  protected readonly homes = computed(() =>
    this.accounts.list().filter((a) => a.id === DEFAULT_ACCOUNT || a.kind !== 'token'),
  );
  /** The account sent to the server ('' for the Default). */
  protected readonly accountParam = computed(() =>
    this.account() === DEFAULT_ACCOUNT ? '' : this.account(),
  );
  protected readonly installedIds = computed(
    () => new Set((this.view()?.installed ?? []).map((p) => p.id)),
  );
  protected readonly scope = scopeText;

  private searchTimer?: ReturnType<typeof setTimeout>;
  private searchGen = 0;

  constructor() {
    void this.accounts.ensureLoaded();
    void this.load();
    void this.runSearch('');
    void this.loadProjects();
    inject(DestroyRef).onDestroy(() => clearTimeout(this.searchTimer));
  }

  protected key(p: InstalledPlugin): string {
    return `${p.id}|${p.scope}`;
  }

  protected serverNames(p: InstalledPlugin): string {
    return p.mcpServers.map((s) => (s.host ? `${s.name} (${s.host})` : s.name)).join(', ');
  }

  private async loadProjects(): Promise<void> {
    try {
      this.projects.set((await this.api.getProjects()).projects ?? []);
    } catch {
      this.projects.set([]);
    }
  }

  protected async load(): Promise<void> {
    try {
      this.view.set(
        await this.api.getPlugins({
          account: this.accountParam() || undefined,
          project: this.project() || undefined,
        }),
      );
      this.loadError.set('');
    } catch (e) {
      this.loadError.set(describeError(e).text);
    }
  }

  protected pickAccount(id: string): void {
    this.account.set(id || DEFAULT_ACCOUNT);
    this.view.set(null);
    void this.load();
    void this.runSearch(this.query());
  }

  protected pickProject(path: string): void {
    this.project.set(path);
    void this.load();
  }

  protected search(q: string): void {
    this.query.set(q);
    clearTimeout(this.searchTimer);
    this.searchTimer = setTimeout(() => void this.runSearch(q), SEARCH_DEBOUNCE_MS);
  }

  private async runSearch(q: string): Promise<void> {
    const gen = ++this.searchGen;
    try {
      const c = await this.api.searchPlugins(q, this.accountParam() || undefined);
      if (gen !== this.searchGen) return;
      this.catalog.set(c);
      this.catalogError.set('');
    } catch (e) {
      if (gen !== this.searchGen) return;
      this.catalogError.set(describeError(e).text);
    }
  }

  /** The scope and project a change of `p` names. */
  private where(p: InstalledPlugin): { scope: PluginScope; project?: string; account?: string } {
    const scope = (p.scope === 'project' || p.scope === 'local' ? p.scope : 'user') as PluginScope;
    return {
      scope,
      ...(scope !== 'user' && this.project() ? { project: this.project() } : {}),
      ...(this.accountParam() ? { account: this.accountParam() } : {}),
    };
  }

  /** Runs a change; `true` when it was made. */
  private async act(
    key: string,
    run: () => Promise<unknown>,
    ok: string,
    what: string,
  ): Promise<boolean> {
    if (this.busy()) return false;
    this.busy.set(key);
    try {
      await run();
      this.confirming.set(null);
      this.changed.set(true);
      this.notices.success(ok);
      await this.load();
      return true;
    } catch (e) {
      this.notices.error(e, undefined, what);
      return false;
    } finally {
      this.busy.set(null);
    }
  }

  protected setEnabled(p: InstalledPlugin, enabled: boolean): Promise<boolean> {
    return this.act(
      this.key(p),
      () => this.api.setPluginEnabled({ plugin: p.id, enabled, ...this.where(p) }),
      enabled ? tr('{name} is on.', { name: p.name }) : tr('{name} is off.', { name: p.name }),
      p.name,
    );
  }

  protected update(p: InstalledPlugin): Promise<boolean> {
    return this.act(
      `update:${this.key(p)}`,
      () => this.api.updatePlugin({ plugin: p.id, ...this.where(p) }),
      tr('{name} is up to date.', { name: p.name }),
      tr('Update {name}', { name: p.name }),
    );
  }

  protected uninstall(p: InstalledPlugin): Promise<boolean> {
    return this.act(
      this.key(p),
      () => this.api.uninstallPlugin({ plugin: p.id, ...this.where(p) }),
      tr('{name} is removed.', { name: p.name }),
      tr('Remove {name}', { name: p.name }),
    );
  }

  protected removeMarket(m: Marketplace): Promise<boolean> {
    return this.act(
      `m:${m.name}`,
      () => this.api.removeMarketplace(m.name, this.accountParam() || undefined),
      tr('{name} is removed.', { name: m.name }),
      tr('Remove {name}', { name: m.name }),
    );
  }

  protected async addMarket(e: Event): Promise<void> {
    e.preventDefault();
    const source = this.source().trim();
    if (!source) return;
    const added = await this.act(
      'market',
      () => this.api.addMarketplace(source, this.accountParam() || undefined),
      tr('The marketplace is added: its plugins show under Find a plugin.'),
      tr('Add a marketplace'),
    );
    if (added) {
      this.source.set('');
      void this.runSearch(this.query());
    }
  }

  protected installed(e: { plugin: AvailablePlugin; scope: PluginScope }): void {
    this.installing.set(null);
    this.changed.set(true);
    this.notices.success(
      tr('{name} is installed ({where}).', { name: e.plugin.name, where: scopeText(e.scope) }),
    );
    void this.load();
  }

  protected async reloadAll(): Promise<void> {
    if (this.busy()) return;
    this.busy.set('reload');
    try {
      const r = await this.api.reloadPluginsEverywhere(this.accountParam() || undefined);
      this.notices.success(reloadAllText(r));
      this.changed.set(false);
    } catch (e) {
      this.notices.error(e, undefined, tr('Reload in running sessions'));
    } finally {
      this.busy.set(null);
    }
  }
}
