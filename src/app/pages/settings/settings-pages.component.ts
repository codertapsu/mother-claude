import { ChangeDetectionStrategy, Component, DestroyRef, inject, signal } from '@angular/core';
import { RouterLink } from '@angular/router';

import { ApiService } from '../../core/api.service';
import { tr } from '../../core/i18n/i18n';
import { TPipe } from '../../core/i18n/t.pipe';
import { McpToolsStatus } from '../../core/models';
import { NavSettingsPage, SETTINGS_PAGES } from '../../core/nav';
import { RealtimeService } from '../../core/realtime.service';
import { IconComponent } from '../../shared/icon.component';

/** What a row says while its state is not known (English). */
const HINTS: Record<NavSettingsPage['id'], string> = {
  tools: 'Mother Claude’s tools for Claude Code sessions',
};

/**
 * Settings' own pages (core/nav.ts `SETTINGS_PAGES`), each a row with how
 * it stands that opens its page: Claude Code tools, Mother Claude's tools
 * for the Claude Code sessions on this computer (docs/MCP.md). Phone &
 * devices, Sessions started elsewhere, Browser and HTTP API were rows here
 * too, Settings' *Connections*, until they became places of their own
 * (docs/UI-REDESIGN.md §11, 2026-11-14). The state is read once the page
 * opens, and again when the server says it changed.
 */
@Component({
  selector: 'mc-settings-pages',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [IconComponent, RouterLink, TPipe],
  template: `
    <h2 id="claude-code">Claude Code</h2>
    <section class="card" aria-labelledby="claude-code">
      <div class="list">
        @for (p of pages; track p.id) {
          <a class="list-row link" [routerLink]="p.path" [attr.data-id]="p.id">
            <mc-icon [name]="p.icon" />
            <span class="grow">
              <span>{{ p.label | t }}</span>
              <span class="muted small state">{{ line(p) }}</span>
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
export class SettingsPagesComponent {
  private readonly api = inject(ApiService);
  private readonly realtime = inject(RealtimeService);

  protected readonly pages = SETTINGS_PAGES;
  private readonly tools = signal<McpToolsStatus | null>(null);

  constructor() {
    void this.load();
    const sub = this.realtime.refetch$.subscribe(() => void this.load());
    inject(DestroyRef).onDestroy(() => sub.unsubscribe());
  }

  /** A row's state, or what it is while that is not known. */
  protected line(p: NavSettingsPage): string {
    return this.stateOf(p.id) || tr(HINTS[p.id]);
  }

  private stateOf(id: NavSettingsPage['id']): string {
    switch (id) {
      case 'tools':
        return this.toolsLine();
    }
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

  /** Every client reads it; kept as it was when it can't be read (the row
   * then says what it is instead). */
  private async load(): Promise<void> {
    try {
      this.tools.set(await this.api.getMcpTools(false));
    } catch {
      /* the row's hint stays */
    }
  }
}
