import { Component, DestroyRef, computed, inject } from '@angular/core';
import { ActivatedRoute, ParamMap, Router, RouterLink } from '@angular/router';

import { ViewportService } from '../../core/viewport.service';
import { IconComponent } from '../../shared/icon.component';
import { AgentsPanelComponent } from './agents-panel.component';
import { ChangesPanelComponent } from './changes-panel.component';
import { ComposerComponent } from './composer.component';
import { InfoPanelComponent } from './info-panel.component';
import { PendingDockComponent } from './pending-dock.component';
import { SessionChipsComponent } from './session-chips.component';
import { SessionContext, SessionTab } from './session-context';
import { SessionHeaderComponent } from './session-header.component';
import { TerminalSheetComponent } from './terminal-sheet.component';
import { TranscriptComponent } from './transcript.component';

interface TabDef {
  id: SessionTab;
  label: string;
}

const TABS: TabDef[] = [
  { id: 'chat', label: 'Chat' },
  { id: 'agents', label: 'Agents' },
  { id: 'changes', label: 'Changes' },
  { id: 'info', label: 'Info' },
];

/**
 * One session: a sticky header, tabs (Chat · Agents · Changes · Info), the
 * active panel in the page's one scroll area (the chip row scrolls away with
 * it), and — on Chat — the pending requests and the composer pinned to the
 * bottom. On phones the page takes the whole screen (the shell hides its
 * bars) and is sized from the visual viewport, so the composer sits right
 * above the on-screen keyboard.
 *
 * `?view=terminal` opens the terminal view; `?tab=agents|changes|info`
 * selects a tab; `?pending=<id>` (a notification tap) opens Chat with that
 * request in front.
 */
@Component({
  selector: 'mc-session-detail',
  providers: [SessionContext],
  imports: [
    RouterLink,
    AgentsPanelComponent,
    ChangesPanelComponent,
    ComposerComponent,
    IconComponent,
    InfoPanelComponent,
    PendingDockComponent,
    SessionChipsComponent,
    SessionHeaderComponent,
    TerminalSheetComponent,
    TranscriptComponent,
  ],
  templateUrl: './session-detail.component.html',
  styleUrl: './session-detail.component.scss',
  host: { '[class.kbd]': 'compact()' },
})
export class SessionDetailComponent {
  protected readonly ctx = inject(SessionContext);
  private route = inject(ActivatedRoute);
  private router = inject(Router);
  private viewport = inject(ViewportService);

  protected readonly tabs = TABS;
  /** Keyboard up on a phone: tabs fold away to leave room for the chat. */
  protected readonly compact = computed(
    () => this.viewport.phone() && this.ctx.composerFocused() && this.viewport.keyboardOpen(),
  );
  protected readonly relayVia = computed(() => {
    const ch = this.ctx.caps()?.channel;
    return ch === 'tmux' ? 'via tmux' : ch === 'hooks' ? 'via hook' : '';
  });
  protected readonly agentLinks = computed(() => {
    const map = new Map<string, string>();
    for (const a of this.ctx.agents()) if (a.toolUseId) map.set(a.toolUseId, a.id);
    return map;
  });
  protected readonly counts = computed<Record<SessionTab, number>>(() => {
    const s = this.ctx.session();
    const a = s?.agents;
    return {
      chat: s?.pending?.length ?? 0,
      agents: (a?.running ?? 0) + (a?.backgroundRunning ?? 0),
      changes: this.ctx.detail()?.filesChanged?.length ?? 0,
      info: 0,
    };
  });

  constructor() {
    const subs = [
      this.route.paramMap.subscribe((p) => {
        const id = p.get('id') ?? '';
        if (id === this.ctx.id()) return;
        this.ctx.open(id);
        // The router emits query params before params: re-apply them to the
        // session just opened (open() resets the tab).
        this.applyQuery(this.route.snapshot.queryParamMap);
      }),
      this.route.queryParamMap.subscribe((q) => this.applyQuery(q)),
    ];
    inject(DestroyRef).onDestroy(() => {
      subs.forEach((s) => s.unsubscribe());
      this.ctx.destroy();
    });
  }

  private applyQuery(q: ParamMap): void {
    const tab = q.get('tab') as SessionTab | null;
    if (tab && TABS.some((t) => t.id === tab)) this.ctx.tab.set(tab);
    const pending = q.get('pending');
    if (pending) {
      // The request is answered on Chat, whatever tab was open before.
      this.ctx.tab.set('chat');
      this.ctx.focusPending.set(pending);
    }
    const terminal = q.get('view') === 'terminal';
    if (terminal) this.ctx.terminalOpen.set(true);
    if (terminal || pending) {
      // Consume them, so a reload doesn't reopen the sheet or jump back.
      void this.router.navigate([], {
        relativeTo: this.route,
        queryParams: { view: null, pending: null },
        queryParamsHandling: 'merge',
        replaceUrl: true,
      });
    }
  }

  protected select(tab: SessionTab): void {
    this.ctx.tab.set(tab);
  }

  /** Tabs: arrows / Home / End move and select (roving tabindex). */
  protected onTabKey(e: KeyboardEvent, i: number): void {
    const n = TABS.length;
    let next = -1;
    if (e.key === 'ArrowRight') next = (i + 1) % n;
    else if (e.key === 'ArrowLeft') next = (i - 1 + n) % n;
    else if (e.key === 'Home') next = 0;
    else if (e.key === 'End') next = n - 1;
    if (next < 0) return;
    e.preventDefault();
    this.select(TABS[next].id);
    queueMicrotask(() => document.getElementById(`sd-tab-${TABS[next].id}`)?.focus());
  }
}
