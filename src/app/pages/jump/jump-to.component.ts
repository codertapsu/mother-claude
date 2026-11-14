import {
  ChangeDetectionStrategy,
  Component,
  DestroyRef,
  ElementRef,
  computed,
  effect,
  inject,
  linkedSignal,
  signal,
  untracked,
  viewChild,
} from '@angular/core';
import { Router } from '@angular/router';

import { ApiService } from '../../core/api.service';
import { AuthService } from '../../core/auth.service';
import { DataProject, normalizeDataProjects } from '../../core/delete-data';
import { tr } from '../../core/i18n/i18n';
import { TPipe } from '../../core/i18n/t.pipe';
import { JumpItem, JumpKind, JumpService, rankJump, sessionKey } from '../../core/jump';
import { osLabel } from '../../core/machines';
import { MachinesService } from '../../core/machines.service';
import { SessionSummary } from '../../core/models';
import { HELP_ENTRY, PAGE_TITLES, SETTINGS_PAGES, sidebarPlaces } from '../../core/nav';
import { tildePath } from '../../core/paths';
import { PermissionsService } from '../../core/permissions.service';
import { ProjectSettingsService, projectRef } from '../../core/project-settings';
import { RealtimeService } from '../../core/realtime.service';
import { RemoteSessionsService } from '../../core/remote-sessions.service';
import { sessionLink } from '../../core/session-links';
import { IconComponent, IconName } from '../../shared/icon.component';
import { sessionName, statusLabel } from '../../shared/util';
import { ensureHelpLanguage, helpTopics } from '../help/help-localized';

/** More words a page is found by: its old names and what it holds. */
const PAGE_WORDS: Readonly<Record<string, string>> = {
  '/': 'list live',
  '/inbox': 'requests approvals waiting to fix',
  '/agents': 'subagents shells teammates',
  '/projects': 'folders delete',
  '/teams': 'roles board',
  '/teams/capacity': 'budgets routing models costs',
  '/computers': 'machines',
  '/computers/sync': 'machines sync',
  '/computers/activity': 'machines moves runs tasks',
  '/computers/options': 'machines folders roots',
  '/devices': 'remote pair phone internet tunnel',
  '/elsewhere': 'plugin hooks foreign vs code terminal',
  '/browser': 'extension chrome',
  '/http-api': 'bridge',
  '/accounts/usage': 'plan limits',
  '/accounts/changes': 'undo moves copies deletes',
  '/backups': 'backup restore pull',
  '/permissions': 'allow always remember rules grants ask',
  '/settings/tools': 'services mcp',
  '/new': 'start',
};

/** Settings' sections a link opens (`/settings#<id>`): English label, icon,
 * more words, and who sees it. */
const SETTINGS: readonly {
  id: string;
  label: string;
  icon: IconName;
  words: string;
  only?: 'device' | 'app';
}[] = [
  { id: 'notifications', label: 'Notifications', icon: 'bell', words: 'push alerts sound' },
  {
    id: 'appearance',
    label: 'Appearance',
    icon: 'sun',
    words: 'theme language dark light accent background',
  },
  { id: 'connection', label: 'Connection', icon: 'wifi', words: 'reconnect unpair' },
  { id: 'home-screen', label: 'Home Screen app', icon: 'phone', words: 'ios', only: 'device' },
  { id: 'startup', label: 'Startup', icon: 'play', words: 'login', only: 'app' },
  { id: 'updates', label: 'Software updates', icon: 'refresh', words: 'version', only: 'app' },
  { id: 'about', label: 'About', icon: 'info', words: 'version' },
];

/** Computers › Options' sections a link opens. */
const OPTIONS: readonly { id: string; label: string }[] = [
  { id: 'accounts-folders', label: 'Accounts and folders' },
  { id: 'each-project', label: 'Each project' },
];

let nextId = 0;

/**
 * *Jump to…* (docs/UI-REDESIGN.md §9, stage 11): a search over every
 * session (this computer's and the other computers', by name, title,
 * project, folder or branch), the projects (opening their settings), the
 * paired computers (their sessions), every page and tab of the map, the
 * Settings sections and the Help topics. Matching forgives typos of order
 * and accents ("sao luu" finds "sao lưu"); with nothing typed, the recent
 * items lead and every page follows. ↑/↓ move, Enter opens, Esc closes —
 * a combobox over a listbox, the focus staying in the field.
 */
@Component({
  selector: 'mc-jump-to',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [IconComponent, TPipe],
  template: `
    <dialog #dlg class="jump" [attr.aria-label]="'Jump to…' | t" (cancel)="onCancel($event)">
      <div class="panel">
        <div class="search">
          <mc-icon name="search" [size]="18" />
          <input
            #field
            type="text"
            role="combobox"
            aria-autocomplete="list"
            [attr.aria-expanded]="results().length > 0"
            [attr.aria-controls]="listId"
            [attr.aria-activedescendant]="activeId()"
            [attr.aria-label]="'Jump to…' | t"
            [placeholder]="'Sessions, projects, computers, pages, Help…' | t"
            autocomplete="off"
            autocapitalize="off"
            spellcheck="false"
            [value]="query()"
            (input)="setQuery($event)"
            (keydown)="onKey($event)"
          />
          <kbd class="esc">Esc</kbd>
        </div>
        @if (!query().trim() && recentShown()) {
          <p class="group">{{ 'Recent' | t }}</p>
        }
        <ul class="results" role="listbox" [id]="listId" [attr.aria-label]="'Jump to…' | t">
          @for (r of results(); track r.key; let i = $index) {
            <li
              role="option"
              tabindex="-1"
              [id]="optionId(i)"
              [attr.aria-selected]="i === active()"
              [class.active]="i === active()"
              (click)="open(r)"
              (keydown.enter)="open(r)"
              (mousemove)="active.set(i)"
            >
              <mc-icon [name]="r.icon" [size]="16" />
              <span class="text">
                <span class="title">{{ r.title }}</span>
                @if (r.detail) {
                  <span class="detail">{{ r.detail }}</span>
                }
              </span>
              <span class="kind">{{ kindLabel(r.kind) }}</span>
            </li>
          } @empty {
            <li class="none" role="presentation">
              {{ 'Nothing matches “{query}”.' | t: { query: query().trim() } }}
            </li>
          }
        </ul>
        <p class="keys">{{ '↑ ↓ to move · Enter to open · Esc to close' | t }}</p>
      </div>
    </dialog>
  `,
  styles: [
    `
      .jump {
        border: 0;
        padding: 0;
        margin: max(0.75rem, env(safe-area-inset-top)) auto auto;
        width: calc(100vw - 1.5rem);
        max-width: 40rem;
        background: transparent;
        color: var(--text);
        overflow: visible;
      }
      .jump::backdrop {
        background: var(--scrim);
      }
      .panel {
        display: flex;
        flex-direction: column;
        max-height: calc(var(--vvh, 100dvh) - 1.5rem);
        background: var(--panel);
        border: 1px solid var(--border-strong);
        border-radius: var(--radius);
        box-shadow: var(--shadow);
        overflow: hidden;
      }
      .search {
        display: flex;
        align-items: center;
        gap: 0.5rem;
        padding: 0.35rem 0.75rem;
        border-bottom: 1px solid var(--border);
        color: var(--muted);
      }
      .search input {
        flex: 1;
        min-width: 0;
        border: 0;
        background: transparent;
        box-shadow: none;
        font-size: 1rem;
        padding: 0.55rem 0;
        outline: none;
      }
      kbd {
        font-family: var(--mono);
        font-size: 0.7rem;
        color: var(--faint);
        border: 1px solid var(--border);
        border-radius: 0.3rem;
        padding: 0.05rem 0.35rem;
      }
      .group {
        margin: 0;
        padding: 0.55rem 0.9rem 0.15rem;
        font-size: 0.72rem;
        font-weight: 600;
        letter-spacing: 0.04em;
        text-transform: uppercase;
        color: var(--faint);
      }
      .results {
        list-style: none;
        margin: 0;
        padding: 0.3rem;
        overflow-y: auto;
        overscroll-behavior: contain;
        max-height: min(60vh, 28rem);
      }
      li[role='option'] {
        display: flex;
        align-items: center;
        gap: 0.65rem;
        min-height: var(--tap);
        padding: 0.35rem 0.6rem;
        border-radius: var(--radius-sm);
        cursor: pointer;
        outline: none;
      }
      li[role='option'] mc-icon {
        flex: 0 0 auto;
        color: var(--muted);
      }
      li.active {
        background: var(--accent-soft);
      }
      li.active mc-icon {
        color: var(--accent-text);
      }
      .text {
        flex: 1;
        min-width: 0;
        display: flex;
        flex-direction: column;
      }
      .title,
      .detail {
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
      }
      .title {
        font-weight: 500;
      }
      .detail {
        font-size: 0.78rem;
        color: var(--muted);
      }
      .kind {
        flex: 0 0 auto;
        font-size: 0.72rem;
        color: var(--faint);
      }
      .none {
        padding: 1rem 0.6rem;
        color: var(--muted);
        text-align: center;
      }
      .keys {
        margin: 0;
        padding: 0.45rem 0.9rem;
        border-top: 1px solid var(--border);
        font-size: 0.72rem;
        color: var(--faint);
      }
      @media (pointer: coarse) {
        .keys,
        .esc {
          display: none;
        }
      }
      @media (min-width: 768px) {
        .jump {
          margin-top: 12vh;
        }
        .panel {
          max-height: 76vh;
        }
      }
    `,
  ],
})
export class JumpToComponent {
  private readonly jump = inject(JumpService);
  private readonly router = inject(Router);
  private readonly api = inject(ApiService);
  private readonly auth = inject(AuthService);
  private readonly permissions = inject(PermissionsService);
  private readonly realtime = inject(RealtimeService);
  private readonly remote = inject(RemoteSessionsService);
  private readonly machines = inject(MachinesService);
  private readonly projectSettings = inject(ProjectSettingsService);

  private readonly dlg = viewChild.required<ElementRef<HTMLDialogElement>>('dlg');
  private readonly field = viewChild.required<ElementRef<HTMLInputElement>>('field');

  protected readonly listId = `mc-jump-list-${nextId++}`;
  protected readonly query = signal('');
  /** Every project on disk (read when the palette opens). */
  private readonly diskProjects = signal<DataProject[]>([]);
  private returnFocus: HTMLElement | null = null;

  protected readonly items = computed<JumpItem[]>(() => [
    ...this.sessionItems(),
    ...this.projectItems(),
    ...this.computerItems(),
    ...this.pageItems(),
    ...this.settingItems(),
    ...this.helpItems(),
  ]);
  protected readonly results = computed(() =>
    rankJump(this.items(), this.query(), this.jump.recent()),
  );
  /** The option Enter opens: the first, until ↑/↓ move. */
  protected readonly active = linkedSignal<JumpItem[], number>({
    source: this.results,
    computation: () => 0,
  });
  protected readonly activeId = computed(() =>
    this.results().length ? this.optionId(this.active()) : null,
  );
  protected readonly recentShown = computed(() => {
    const keys = new Set(this.items().map((i) => i.key));
    return this.jump.recent().some((k) => keys.has(k));
  });

  constructor() {
    effect(() => {
      const el = this.dlg().nativeElement;
      const open = this.jump.opened();
      untracked(() => (open ? this.show(el) : this.hide(el)));
    });
    // A click on the dialog box itself (not its panel) is a backdrop click.
    effect((onCleanup) => {
      const el = this.dlg().nativeElement;
      const onClick = (e: MouseEvent) => {
        if (e.target === el) this.jump.close();
      };
      el.addEventListener('click', onClick);
      onCleanup(() => el.removeEventListener('click', onClick));
    });
    inject(DestroyRef).onDestroy(() => {
      const el = this.dlg().nativeElement;
      if (el.open) el.close();
    });
  }

  private show(el: HTMLDialogElement): void {
    if (el.open) return;
    this.query.set('');
    this.returnFocus =
      document.activeElement instanceof HTMLElement ? document.activeElement : null;
    try {
      el.showModal();
    } catch {
      el.setAttribute('open', '');
    }
    this.field().nativeElement.focus();
    // What it searches besides the live lists: every project on disk, the
    // Help in the language shown, the paired computers.
    ensureHelpLanguage();
    if (!this.machines.view()) void this.machines.load();
    void this.loadProjects();
  }

  private hide(el: HTMLDialogElement): void {
    if (!el.open) return;
    el.close();
    this.returnFocus?.focus?.();
    this.returnFocus = null;
  }

  private async loadProjects(): Promise<void> {
    try {
      this.diskProjects.set(normalizeDataProjects(await this.api.getDataProjects()));
    } catch {
      // The sessions' own projects still show.
    }
  }

  protected optionId(i: number): string {
    return `${this.listId}-${i}`;
  }

  /** What each kind is called on its row. */
  protected kindLabel(kind: JumpKind): string {
    switch (kind) {
      case 'session':
        return tr('Session');
      case 'project':
        return tr('Project');
      case 'computer':
        return tr('Computer');
      case 'page':
        return tr('Page');
      case 'setting':
        return tr('Setting');
      case 'help':
        return tr('Help topic');
    }
  }

  protected setQuery(e: Event): void {
    this.query.set((e.target as HTMLInputElement).value);
  }

  /** ↑/↓ move through the results (around at the ends), Enter opens. */
  protected onKey(e: KeyboardEvent): void {
    const n = this.results().length;
    if (e.key === 'Enter') {
      e.preventDefault();
      const item = this.results()[this.active()];
      if (item) this.open(item);
      return;
    }
    if ((e.key !== 'ArrowDown' && e.key !== 'ArrowUp') || !n) return;
    e.preventDefault();
    const step = e.key === 'ArrowDown' ? 1 : -1;
    this.active.set((this.active() + step + n) % n);
    queueMicrotask(() =>
      document.getElementById(this.optionId(this.active()))?.scrollIntoView({ block: 'nearest' }),
    );
  }

  protected open(item: JumpItem): void {
    this.jump.remember(item.key);
    // Nowhere to give the focus back to: the page that opens takes it.
    this.returnFocus = null;
    this.jump.close();
    const t = item.target;
    if (t.kind === 'project') {
      this.projectSettings.open(t.project);
      return;
    }
    void this.router.navigate(t.commands, {
      ...(t.queryParams ? { queryParams: t.queryParams } : {}),
      ...(t.fragment ? { fragment: t.fragment } : {}),
    });
  }

  protected onCancel(e: Event): void {
    // Esc: the service owns `opened`.
    e.preventDefault();
    this.jump.close();
  }

  // ─────────────────────────────────────────────────────────── The items

  private readonly sessionItems = computed<JumpItem[]>(() => {
    const local = this.realtime.sessions().map((s) => this.sessionItem(s, ''));
    const remote = this.remote
      .list()
      .flatMap((m) => m.sessions.map((s) => this.sessionItem({ ...s, machine: m.id }, m.name)));
    return [...local, ...remote];
  });

  private sessionItem(s: SessionSummary, computer: string): JumpItem {
    return {
      key: sessionKey(s.id, s.machine),
      kind: 'session',
      title: sessionName(s),
      detail: [s.projectName, statusLabel(s.status, s.activity), computer]
        .filter(Boolean)
        .join(' · '),
      icon: 'sessions',
      words: [s.title, s.name, s.cwd, s.gitBranch, s.id].filter(Boolean).join(' '),
      target: { kind: 'link', commands: sessionLink(s.id, s.machine) },
    };
  }

  /** This computer's projects: every one on disk, and the live sessions'
   * folders (another computer's are its own). */
  private readonly projectItems = computed<JumpItem[]>(() => {
    const byCwd = new Map<string, string>();
    for (const p of this.diskProjects()) if (!byCwd.has(p.cwd)) byCwd.set(p.cwd, p.name);
    for (const s of this.realtime.sessions()) {
      if (s.cwd && !byCwd.has(s.cwd)) byCwd.set(s.cwd, s.projectName);
    }
    return [...byCwd].map(([cwd, name]) => {
      const project = projectRef(cwd, name);
      return {
        key: `project:${cwd}`,
        kind: 'project' as const,
        title: project.name,
        detail: tildePath(cwd),
        icon: 'folder' as const,
        words: tr('Project settings'),
        target: { kind: 'project' as const, project },
      };
    });
  });

  /** The paired computers, opening their sessions. */
  private readonly computerItems = computed<JumpItem[]>(() => {
    const v = this.machines.view();
    if (!v?.enabled) return [];
    return v.peers.map((p) => ({
      key: `computer:${p.id}`,
      kind: 'computer' as const,
      title: p.name,
      detail: p.online ? tr('Online · its sessions') : tr('Offline · its sessions'),
      icon: 'monitor' as const,
      words: osLabel(p.os),
      target: { kind: 'link' as const, commands: ['/'], queryParams: { machine: p.id } },
    }));
  });

  /** Every page of the map: the places, their tabs, Settings' own pages,
   * New session and Help — the English names found too. */
  private readonly pageItems = computed<JumpItem[]>(() => {
    const desktop = this.auth.isDesktop();
    const out: JumpItem[] = [];
    const page = (path: string, label: string, within: string, icon: IconName): JumpItem => ({
      key: `page:${path}`,
      kind: 'page',
      title: tr(label),
      detail: within ? tr(within) : undefined,
      icon,
      words: [label, within, PAGE_WORDS[path] ?? ''].join(' '),
      target: { kind: 'link', commands: [path] },
    });
    for (const p of sidebarPlaces(desktop)) {
      out.push(page(p.path, p.label, '', p.icon));
      for (const t of p.tabs ?? []) {
        if (t.path !== p.path) out.push(page(t.path, t.label, p.label, p.icon));
      }
    }
    for (const s of SETTINGS_PAGES) out.push(page(s.path, s.label, 'Settings', s.icon));
    out.push(page('/new', PAGE_TITLES['new'], 'Sessions', 'plus'));
    out.push(page(HELP_ENTRY.path, HELP_ENTRY.label, '', HELP_ENTRY.icon));
    return out;
  });

  /** Settings' sections, and Computers › Options' per-project ones. */
  private readonly settingItems = computed<JumpItem[]>(() => {
    const device = this.auth.me()?.principal === 'device';
    const app = this.permissions.desktop();
    const settings = SETTINGS.filter(
      (s) => !s.only || (s.only === 'device' ? device : app),
    ).map<JumpItem>((s) => ({
      key: `setting:/settings#${s.id}`,
      kind: 'setting',
      title: tr(s.label),
      detail: tr('Settings'),
      icon: s.icon,
      words: `${s.label} ${s.words} settings`,
      target: { kind: 'link', commands: ['/settings'], fragment: s.id },
    }));
    const options = OPTIONS.map<JumpItem>((o) => ({
      key: `setting:/computers/options#${o.id}`,
      kind: 'setting',
      title: tr(o.label),
      detail: tr('Computers › Options'),
      icon: 'monitor',
      words: `${o.label} computers machines options`,
      target: { kind: 'link', commands: ['/computers/options'], fragment: o.id },
    }));
    return [...settings, ...options];
  });

  /** Help's topics, in the language shown (their English keywords too). */
  private readonly helpItems = computed<JumpItem[]>(() =>
    helpTopics().map((t) => ({
      key: `help:${t.id}`,
      kind: 'help' as const,
      title: t.title,
      detail: t.summary,
      icon: t.icon,
      words: (t.keywords ?? []).join(' '),
      target: { kind: 'link' as const, commands: ['/help', t.id] },
    })),
  );
}
