import {
  ChangeDetectionStrategy,
  Component,
  DestroyRef,
  computed,
  effect,
  inject,
  signal,
  untracked,
} from '@angular/core';
import { RouterLink } from '@angular/router';

import { AccountsService } from '../../core/accounts.service';
import { serversOf } from '../../core/api-servers';
import { ApiService } from '../../core/api.service';
import { AuthService } from '../../core/auth.service';
import { BrowserOverview } from '../../core/browser';
import { tr } from '../../core/i18n/i18n';
import { TPipe } from '../../core/i18n/t.pipe';
import {
  BridgeStatus,
  DevicesList,
  ForeignControlStatus,
  McpToolsStatus,
  RemoteInfo,
} from '../../core/models';
import { NavConnection, connections } from '../../core/nav';
import { RealtimeService } from '../../core/realtime.service';
import { IconComponent } from '../../shared/icon.component';

/** What a row says while its state is not known (English). */
const HINTS: Record<NavConnection['id'], string> = {
  devices: 'Pair a phone, internet access, the paired devices',
  elsewhere: 'Answer VS Code and terminal sessions from here',
  browser: 'Sessions in your browser: site modes, approvals',
  api: 'Drive Claude from scripts and tools',
  tools: 'Mother Claude’s tools for Claude Code sessions',
};

/**
 * Settings' *Connections* (docs/UI-REDESIGN.md §6.7): what else works with
 * Mother Claude — phones and devices, sessions started elsewhere, the
 * browser extension, the HTTP API, Claude Code tools — each a row with its
 * state that opens its page (core/nav.ts `CONNECTIONS`). The desktop sees
 * all five; a phone the four it may open. States are read once the page
 * opens, and again when the server says they changed.
 */
@Component({
  selector: 'mc-settings-connections',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [IconComponent, RouterLink, TPipe],
  template: `
    <h2>{{ 'Connections' | t }}</h2>
    <section class="card" [attr.aria-label]="'Connections' | t">
      <div class="list">
        @for (c of rows(); track c.id) {
          <a
            class="list-row link"
            [routerLink]="c.path"
            [fragment]="c.fragment"
            [attr.data-id]="c.id"
          >
            <mc-icon [name]="c.icon" />
            <span class="grow">
              <span>{{ c.label | t }}</span>
              <span class="muted small state">{{ line(c) }}</span>
            </span>
            <mc-icon name="chevronRight" [size]="16" />
          </a>
        }
      </div>
    </section>
  `,
  styles: [
    `
      :host {
        display: block;
      }
      section.card {
        padding-top: 0.35rem;
        padding-bottom: 0.35rem;
      }
      .link {
        color: var(--text);
        text-decoration: none;
      }
      .link mc-icon {
        color: var(--muted);
      }
      .grow {
        flex: 1;
        min-width: 0;
        display: flex;
        flex-direction: column;
        gap: 0.1rem;
      }
    `,
  ],
})
export class ConnectionsComponent {
  private readonly api = inject(ApiService);
  private readonly auth = inject(AuthService);
  private readonly accounts = inject(AccountsService);
  private readonly realtime = inject(RealtimeService);

  protected readonly rows = computed(() => connections(this.auth.isDesktop()));

  private readonly devices = signal<DevicesList | null>(null);
  private readonly remote = signal<RemoteInfo | null>(null);
  private readonly foreign = signal<ForeignControlStatus | null>(null);
  private readonly browser = signal<BrowserOverview | null>(null);
  private readonly bridge = signal<BridgeStatus | null>(null);
  private readonly tools = signal<McpToolsStatus | null>(null);

  constructor() {
    void this.loadShared();
    // The desktop's rows, once this client is known to be the desktop.
    let desktopLoaded = false;
    effect(() => {
      if (!this.auth.isDesktop() || desktopLoaded) return;
      desktopLoaded = true;
      untracked(() => void this.loadDesktop());
    });
    const subs = [
      this.realtime.browser$.subscribe(
        () => void this.load(() => this.api.getBrowser(), this.browser),
      ),
      this.realtime.devices$.subscribe(
        () => void this.load(() => this.api.listDevices(), this.devices),
      ),
      this.realtime.refetch$.subscribe(() => {
        void this.loadShared();
        if (this.auth.isDesktop()) void this.loadDesktop();
      }),
    ];
    inject(DestroyRef).onDestroy(() => subs.forEach((s) => s.unsubscribe()));
  }

  /** A row's state, or what it is while that is not known. */
  protected line(c: NavConnection): string {
    const state = this.stateOf(c.id);
    return state || tr(HINTS[c.id]);
  }

  private stateOf(id: NavConnection['id']): string {
    switch (id) {
      case 'devices':
        return this.devicesLine();
      case 'elsewhere':
        return this.elsewhereLine();
      case 'browser':
        return this.browserLine();
      case 'api':
        return this.apiLine();
      default:
        return this.toolsLine();
    }
  }

  /** "2 devices paired · internet access on". */
  private devicesLine(): string {
    const d = this.devices();
    const r = this.remote();
    const parts: string[] = [];
    if (d) {
      const n = (d.devices ?? []).filter((x) => !x.revoked).length;
      parts.push(
        n === 0
          ? tr('No device paired yet')
          : n === 1
            ? tr('1 device paired')
            : tr('{n} devices paired', { n }),
      );
    }
    if (r) parts.push(r.internet?.enabled ? tr('internet access on') : tr('internet access off'));
    return parts.join(' · ');
  }

  /** "VS Code and terminal sessions: on for Default and Work". */
  private elsewhereLine(): string {
    const f = this.foreign();
    if (!f) return '';
    const list = this.accounts.list();
    const on: string[] = [];
    if (f.installed && f.enabled !== false) {
      on.push(list.find((a) => a.isDefault)?.name ?? tr('Default'));
    }
    for (const a of list) {
      if (!a.isDefault && a.kind === 'directory' && a.hooksPlugin === 'installed') on.push(a.name);
    }
    if (on.length && list.length > 1) {
      return tr('VS Code and terminal sessions: on for {accounts}', { accounts: on.join(', ') });
    }
    if (on.length) return tr('VS Code and terminal sessions: on');
    if (f.installed) return tr('VS Code and terminal sessions: installed, switched off');
    return tr('VS Code and terminal sessions: off');
  }

  /** "Chrome connected · 2 sites may act". */
  private browserLine(): string {
    const b = this.browser();
    if (!b) return '';
    const paired = (b.browsers ?? []).filter((x) => !x.revoked);
    const connected = paired.filter((x) => x.connected);
    const parts = [
      !paired.length
        ? tr('No browser paired yet')
        : connected.length === 1
          ? tr('{name} connected', { name: connected[0].browser?.name || connected[0].name })
          : connected.length
            ? tr('{n} browsers connected', { n: connected.length })
            : tr('Not connected'),
    ];
    const act = (b.sites ?? []).filter((s) => s.mode === 'act').length;
    if (act) parts.push(act === 1 ? tr('1 site may act') : tr('{n} sites may act', { n: act }));
    return parts.join(' · ');
  }

  /** "Running on 127.0.0.1:5612 · 1 more server". */
  private apiLine(): string {
    const s = this.bridge();
    if (!s) return '';
    const address = (s.url ?? '').replace(/^https?:\/\//, '').replace(/\/+$/, '');
    const parts = [
      s.running
        ? address
          ? tr('Running on {address}', { address })
          : tr('Running')
        : tr('Not started'),
    ];
    const more = serversOf(s).length - 1;
    if (more > 0)
      parts.push(more === 1 ? tr('1 more server') : tr('{n} more servers', { n: more }));
    return parts.join(' · ');
  }

  /** "On · used by 4 sessions in the last 15 min". */
  private toolsLine(): string {
    const t = this.tools();
    if (!t) return '';
    if (!t.enabled) return tr('Off: the browser extension and the team tools need them');
    if (t.clients === 1) return tr('On · used by 1 session in the last 15 min');
    if (t.clients > 1) return tr('On · used by {n} sessions in the last 15 min', { n: t.clients });
    return tr('On');
  }

  /** What every client may read (a phone reads the devices paired, read
   * only). */
  private async loadShared(): Promise<void> {
    await Promise.all([
      this.load(() => this.api.listDevices(), this.devices),
      this.load(() => this.api.getBrowser(), this.browser),
      this.load(() => this.api.getBridge(), this.bridge),
      this.load(() => this.api.getMcpTools(false), this.tools),
    ]);
  }

  /** What only the desktop reads. */
  private async loadDesktop(): Promise<void> {
    await Promise.all([
      this.load(() => this.api.getRemote(), this.remote),
      this.load(() => this.api.getForeignControl(), this.foreign),
    ]);
  }

  /** One state: kept as it was when it can't be read (the row then says
   * what it is instead). */
  private async load<T>(read: () => Promise<T>, into: { set(v: T): void }): Promise<void> {
    try {
      into.set(await read());
    } catch {
      /* the row's hint stays */
    }
  }
}
