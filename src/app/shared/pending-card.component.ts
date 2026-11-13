import {
  ChangeDetectionStrategy,
  Component,
  ElementRef,
  computed,
  effect,
  inject,
  input,
  linkedSignal,
  output,
  signal,
  untracked,
} from '@angular/core';
import { NgTemplateOutlet } from '@angular/common';
import { FormsModule } from '@angular/forms';
import { Router, RouterLink } from '@angular/router';

import { describeError } from '../core/api.service';
import { AuthService } from '../core/auth.service';
import { computerNoun, tr } from '../core/i18n/i18n';
import { TPipe } from '../core/i18n/t.pipe';
import { MachineContext } from '../core/machine-context';
import { MachineScopes } from '../core/machine-scope';
import {
  PendingItem,
  PendingRef,
  PermissionSuggestion,
  Question,
  RespondBody,
  SessionSummary,
} from '../core/models';
import { NoticeService } from '../core/notice.service';
import { PendingService, isFullItem, resolutionText } from '../core/pending.service';
import { machineRunInput } from '../core/runs';
import { machineTaskInput } from '../core/tasks';
import { RealtimeService } from '../core/realtime.service';
import { sessionLink } from '../core/session-links';
import { AccountChipComponent } from './account-chip.component';
import { BrowserActionComponent } from './browser-action.component';
import { CapLimitsComponent } from './cap-limits.component';
import { ChipComponent } from './chip.component';
import { CodeCopyDirective } from './code-copy.directive';
import { IconComponent, IconName } from './icon.component';
import { MachineRunActionComponent } from './machine-run-action.component';
import { MachineTaskActionComponent } from './machine-task-action.component';
import { renderMarkdown } from './markdown';
import { SheetComponent } from './sheet.component';
import { TimeAgoComponent } from './time-ago.component';
import { ToolPreviewComponent } from './tool-preview.component';
import { sessionName, surfaceLabel } from './util';

/** What the card emits once an answer landed (or someone else was first). */
export interface PendingResolution {
  item: PendingItem | PendingRef;
  body: RespondBody | null;
  alreadyDecided: boolean;
}

type Panel = 'main' | 'always' | 'feedback' | 'planFeedback';

/** Where an "always" rule is saved, and what that means. English:
 * translated where they are read (`destination`). */
const DESTINATIONS: Record<string, string> = {
  session: 'This session only',
  localSettings: 'Saved in .claude/settings.local.json (this project, only you)',
  projectSettings: 'Saved in .claude/settings.json (this project, shared with the repo)',
  userSettings: 'Saved in ~/.claude/settings.json (all your projects)',
  cliArg: 'This run only',
  browser: 'Saved in Mother Claude (Settings › Browser › Always allowed)',
  machines: 'Saved in Mother Claude (Computers › Activity › Always allowed runs)',
};

interface SchemaField {
  key: string;
  label: string;
  type: 'string' | 'number' | 'boolean' | 'enum';
  options?: string[];
  required: boolean;
}

/**
 * The one pending-item card (Inbox, the session page's dock, session list):
 * permission, AskUserQuestion, ExitPlanMode plan and MCP elicitation.
 *
 * Inputs:
 * - `item` (required) — a full PendingItem, or a list preview (with its
 *   `sessionId`); the full item is loaded on demand through PendingService.
 * - `session` — its session (surface guidance, cwd for Bash previews).
 * - `compact` — list version: quick actions only for exactly-one simple,
 *   answerable, non-dangerous item (a permission, or one single-select
 *   question with ≤ 4 options), else "Review".
 * - `showSession` — show which session asks (Inbox), with its Claude
 *   account's chip while there are several.
 * - `gone` — the item left the store; the card shows how it was settled.
 * - `bodyMax` — cap the body height (the dock uses 45vh) and scroll inside.
 * - `machine` — another computer's request (its id; the Inbox and the
 *   list): answered through that computer's scope, named by its chip. On
 *   that computer's session page its scope says so without it.
 * Outputs: `resolved`, `move` (J/K: ±1), `sendAsMessage` (typed feedback
 * when the item was answered elsewhere first).
 *
 * Per-tool previews (Bash command, Edit diff, Write head, fetch URL, MCP
 * server · tool); an agent badge; dangerous items get a red border, the
 * reason, and Allow is never focused for you. In-flight answers disable the
 * buttons everywhere the same item is shown. Desktop shortcuts on a focused
 * card: A allow · Shift+A first "always" · D deny · F deny with feedback ·
 * J/K next/previous · 1–9 pick an option.
 */
@Component({
  selector: 'mc-pending-card',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [
    FormsModule,
    NgTemplateOutlet,
    RouterLink,
    AccountChipComponent,
    BrowserActionComponent,
    CapLimitsComponent,
    ChipComponent,
    CodeCopyDirective,
    IconComponent,
    MachineRunActionComponent,
    MachineTaskActionComponent,
    SheetComponent,
    TimeAgoComponent,
    ToolPreviewComponent,
    TPipe,
  ],
  templateUrl: './pending-card.component.html',
  styleUrl: './pending-card.component.scss',
  host: {
    '[attr.data-kind]': 'item().kind',
    '[class.compact]': 'compact()',
    '[class.dangerous]': 'item().dangerous',
    '[class.settled]': 'settled()',
    '[attr.tabindex]': 'compact() ? null : 0',
    '[attr.aria-label]': 'ariaLabel()',
    '(keydown)': 'onKey($event)',
  },
})
export class PendingCardComponent {
  private ownPendings = inject(PendingService);
  private auth = inject(AuthService);
  private realtime = inject(RealtimeService);
  private notices = inject(NoticeService);
  private router = inject(Router);
  private scopes = inject(MachineScopes);
  /** On another computer's session page: that computer. */
  private scopedMachine = inject(MachineContext, { optional: true });
  private host = inject(ElementRef<HTMLElement>).nativeElement as HTMLElement;

  readonly item = input.required<PendingItem | PendingRef>();
  readonly session = input<SessionSummary | undefined>(undefined);
  readonly compact = input(false);
  readonly showSession = input(false);
  readonly gone = input(false);
  readonly bodyMax = input<string | undefined>(undefined);
  /** Another computer's request (its id), in a list of several computers'. */
  readonly machine = input<string | null>(null);
  readonly resolved = output<PendingResolution>();
  readonly move = output<number>();
  readonly sendAsMessage = output<string>();

  /** The other computer it waits at (null: this one). */
  protected readonly mctx = computed<MachineContext | null>(() => {
    const m = this.machine();
    return m ? this.scopes.context(m) : this.scopedMachine;
  });
  /** Who answers it: this computer's requests, or that computer's scope. */
  private readonly pendings = computed(() => {
    const m = this.machine();
    return m ? this.scopes.pending(m) : this.ownPendings;
  });
  /** Its session's page. */
  protected readonly link = computed(() => sessionLink(this.item().sessionId, this.mctx()?.id));

  private readonly itemId = computed(() => this.item().id);
  /** The full item: loaded, or the input itself when it is one. */
  protected readonly full = computed<PendingItem | undefined>(() => {
    const it = this.item();
    const cached = this.pendings().get(it.id);
    if (cached) return cached;
    return isFullItem(it) ? it : undefined;
  });
  protected readonly loadingFull = signal(false);
  protected readonly loadError = signal('');

  protected readonly busy = computed(() => this.pendings().inflight().has(this.itemId()));
  protected readonly offline = computed(
    () => !this.realtime.canAct() || this.mctx()?.online() === false,
  );
  /** Why answers wait: this device reconnecting, or the other computer
   * offline. */
  protected readonly offlineText = computed(() => {
    const m = this.mctx();
    return m && !m.online()
      ? tr('{name} is offline: answers wait until it is back.', { name: m.name() })
      : tr('Reconnecting… answers are paused until the connection is back.');
  });
  protected readonly error = linkedSignal({ source: this.itemId, computation: () => '' });
  /** Our own answer's confirmation text. */
  protected readonly done = linkedSignal({ source: this.itemId, computation: () => '' });
  protected readonly panel = linkedSignal<string, Panel>({
    source: this.itemId,
    computation: () => 'main',
  });
  protected readonly feedback = linkedSignal({ source: this.itemId, computation: () => '' });
  protected readonly stopTurn = linkedSignal({ source: this.itemId, computation: () => false });
  protected readonly picks = linkedSignal<string, Record<number, string[]>>({
    source: this.itemId,
    computation: () => ({}),
  });
  protected readonly other = linkedSignal<string, Record<number, string>>({
    source: this.itemId,
    computation: () => ({}),
  });
  protected readonly form = linkedSignal<string, Record<string, unknown>>({
    source: this.itemId,
    computation: () => ({}),
  });
  protected readonly planSheet = signal(false);

  /** How someone else settled it (pendingResolved), when they did. */
  protected readonly resolution = computed(() => this.pendings().resolutionOf(this.itemId()));
  /** While this client's own answer is in flight, the server's echo of it
   * (`pendingResolved` and the sessions diff land before the HTTP response)
   * says nothing about someone else: wait for the response to tell. */
  protected readonly settled = computed(
    () =>
      !!this.done() ||
      (!this.busy() && (this.gone() || !!(this.resolution() && !this.resolution()?.mine))),
  );
  protected readonly settledText = computed(() => {
    if (this.done()) return this.done();
    const r = this.resolution();
    if (r) return resolutionText(r, this.mctx()?.name());
    return tr('Answered elsewhere');
  });

  protected readonly canAct = computed(
    () => this.item().answerable && (this.full()?.source ?? this.item().source) !== 'observed',
  );
  /** Dangerous answers from here: this device's right, and — another
   * computer's request — that computer's Dangerous permission for this one. */
  protected readonly canDangerous = computed(
    () => this.auth.canDangerous() && this.mctx()?.mayDangerous() !== false,
  );
  protected readonly allowBlocked = computed(() => this.item().dangerous && !this.canDangerous());
  /** Where a blocked dangerous request can be allowed instead. */
  protected readonly blockedText = computed(() => {
    const m = this.mctx();
    if (!m) {
      return tr(
        'Approve dangerous actions on the {computer}, or allow this device to take them (Settings › Phone & devices). You can still deny it here.',
        { computer: computerNoun() },
      );
    }
    return m.mayDangerous()
      ? tr('Answer it at {name} or on this {computer}. You can still deny it here.', {
          name: m.name(),
          computer: computerNoun(),
        })
      : tr('Answer it at {name}. You can still deny it here.', { name: m.name() });
  });
  protected readonly questions = computed<Question[]>(() => this.full()?.questions ?? []);
  protected readonly suggestions = computed<PermissionSuggestion[]>(
    () => this.full()?.suggestions ?? [],
  );
  protected readonly firstAlways = computed(() =>
    this.suggestions().find((s) => !this.suggestionBlocked(s)),
  );
  /** List quick actions: exactly the simple cases (design review). */
  protected readonly quick = computed<'permission' | 'question' | null>(() => {
    const it = this.item();
    if (!it.answerable || it.dangerous) return null;
    if (it.kind === 'permission') return 'permission';
    const opts = this.quickOptions();
    if (it.kind === 'question' && opts.length > 0 && opts.length <= 4) return 'question';
    return null;
  });
  protected readonly quickOptions = computed<string[]>(() => {
    const it = this.item();
    const full = this.full();
    if (full?.questions?.length === 1 && !full.questions[0].multiSelect) {
      return full.questions[0].options.map((o) => o.label);
    }
    if (full) return [];
    const preview = it as PendingRef;
    if ((preview.questionCount ?? 1) !== 1) return [];
    return preview.quick?.options ?? [];
  });
  protected readonly planHtml = computed(() => renderMarkdown(this.full()?.plan ?? ''));
  /** A browser action waiting for approval (docs/BROWSER.md). */
  protected readonly isBrowser = computed(
    () => (this.full()?.source ?? this.item().source) === 'browser',
  );
  /** A command a session asks to run on another computer (Machines), or a
   * task it asks to hand to a session there. */
  protected readonly isMachineRun = computed(
    () => (this.full()?.source ?? this.item().source) === 'machine',
  );
  /** A task a session asks to hand to a session on another computer. */
  protected readonly isMachineTask = computed(
    () => this.isMachineRun() && this.toolName() === 'machine_task',
  );
  /** Its "always allow …" is a standing grant: offered only to whoever may
   * attach one (the dangerous right), never shown greyed out to others. */
  protected readonly showAlways = computed(
    () => this.suggestions().length > 0 && (!this.isMachineRun() || this.canDangerous()),
  );
  /** The item's detail as the card shows it (a run's, in the language
   * shown). */
  protected readonly detailText = computed(() => {
    const d = this.item().detail ?? '';
    if (this.isMachineTask()) return taskDetail(d);
    return this.isMachineRun() ? runDetail(d) : d;
  });
  protected readonly toolName = computed(() => this.full()?.toolName ?? this.item().toolName ?? '');
  protected readonly hasPreview = computed(() => {
    const f = this.full();
    return (
      !!f &&
      f.kind === 'permission' &&
      f.input !== undefined &&
      f.input !== null &&
      !!this.toolName()
    );
  });
  protected readonly schemaFields = computed<SchemaField[]>(() => {
    const input = this.full()?.input as { requestedSchema?: unknown } | undefined;
    const schema = input?.requestedSchema as
      { properties?: Record<string, Record<string, unknown>>; required?: string[] } | undefined;
    const props = schema?.properties;
    if (!props || typeof props !== 'object') return [];
    const required = new Set(Array.isArray(schema?.required) ? schema.required : []);
    return Object.entries(props).map(([key, p]) => {
      const en = Array.isArray(p['enum']) ? (p['enum'] as unknown[]).map(String) : undefined;
      const t = String(p['type'] ?? 'string');
      return {
        key,
        label: String(p['title'] ?? p['description'] ?? key),
        type: en
          ? 'enum'
          : t === 'boolean'
            ? 'boolean'
            : t === 'number' || t === 'integer'
              ? 'number'
              : 'string',
        options: en,
        required: required.has(key),
      };
    });
  });
  protected readonly elicitationMessage = computed(() => {
    const input = this.full()?.input as { message?: unknown } | undefined;
    return typeof input?.message === 'string' ? input.message : '';
  });
  protected readonly icon = computed<IconName>(() => {
    switch (this.item().kind) {
      case 'question':
        return 'question';
      case 'plan':
        return 'map';
      case 'elicitation':
        return 'info';
      default:
        return this.isMachineRun() ? 'monitor' : this.isBrowser() ? 'globe' : 'shield';
    }
  });
  protected readonly heading = computed(() => {
    const it = this.item();
    switch (it.kind) {
      case 'question': {
        // The server titles a question with its own text, which the card
        // shows below: the heading says how many there are instead.
        const n = this.questionCount();
        return n > 1 ? tr('Claude has {n} questions', { n }) : tr('Claude has a question');
      }
      case 'plan':
        return tr('Plan ready for review');
      case 'elicitation':
        return it.title || tr('Input requested');
      default: {
        const task = this.isMachineTask() ? machineTaskInput(this.full()?.input) : null;
        if (task) return tr('Hand a task to {name}?', { name: task.machineName });
        const run = this.isMachineRun() ? machineRunInput(this.full()?.input) : null;
        if (run) return tr('Run a command on {name}?', { name: run.machineName });
        return (
          pendingTitle(it.title) ||
          (it.toolName ? tr('Allow {tool}?', { tool: it.toolName }) : tr('Allow this tool?'))
        );
      }
    }
  });
  /** How many questions Claude asks: the full item's, else the preview's count. */
  private readonly questionCount = computed(() => {
    const it = this.item();
    const preview = 'questionCount' in it ? (it.questionCount ?? 0) : 0;
    return this.full()?.questions?.length || preview;
  });
  /** The question a compact card shows: the server's detail or title, unless
   * that is only its generic "Claude has a question". */
  protected readonly questionText = computed(() => {
    const it = this.item();
    const text = (it.detail || it.title || '').trim();
    return GENERIC_QUESTION.test(text) ? '' : text;
  });
  protected readonly ariaLabel = computed(() =>
    this.compact()
      ? null
      : tr('{heading}. Shortcuts: A allow, D deny, J and K move.', { heading: this.heading() }),
  );
  protected readonly sessionLabel = computed(() => {
    const s = this.session();
    return s ? sessionName(s) : this.item().sessionId.slice(0, 8);
  });
  protected readonly surface = computed(() => surfaceLabel(this.session()?.surface ?? 'unknown'));
  protected readonly allAnswered = computed(() =>
    this.questions().every(
      (_, i) => (this.picks()[i]?.length ?? 0) > 0 || !!this.other()[i]?.trim(),
    ),
  );
  protected readonly formValid = computed(() =>
    this.schemaFields().every((f) => !f.required || this.form()[f.key] !== undefined),
  );
  /** Where an unanswerable prompt waits, in the compact card: "Waiting at
   * the Mac (CLI)", "Waiting at DESKTOP-PC (CLI)". */
  protected readonly waitingAt = computed(() => {
    const m = this.mctx();
    return m
      ? tr('Waiting at {name} ({app})', { name: m.name(), app: this.surface() })
      : tr('Waiting at the {computer} ({app})', { computer: computerNoun(), app: this.surface() });
  });
  /** Where an unanswerable prompt can be answered. */
  protected readonly guidance = computed(() => {
    const s = this.session();
    const m = this.mctx();
    if (m) {
      return s?.waitingFor
        ? tr('Waiting for an answer at {name} in {app} ({what}).', {
            name: m.name(),
            app: this.surface(),
            what: s.waitingFor,
          })
        : tr('Waiting for an answer at {name} in {app}.', { name: m.name(), app: this.surface() });
    }
    if (!s) return tr('Answer it where the session runs.');
    const params = { computer: computerNoun(), app: this.surface(), what: s.waitingFor };
    return s.waitingFor
      ? tr('Waiting for an answer at the {computer} in {app} ({what}).', params)
      : tr('Waiting for an answer at the {computer} in {app}.', params);
  });

  constructor() {
    // Full cards need the complete item (input, questions, plan, suggestions).
    effect(() => {
      const it = this.item();
      const need = !this.compact() && (!this.full() || this.full()?.inputTruncated);
      if (!need || this.gone()) return;
      untracked(() => void this.loadFull(it.sessionId, it.id));
    });
    // Answered elsewhere while this card was open: say so once. Not while
    // our own answer is in flight: the echo usually arrives first, and once
    // the response lands it turns the resolution into ours (a 409 leaves it
    // someone else's, and this runs again when the flag clears).
    let told = '';
    effect(() => {
      const r = this.resolution();
      if (!r || r.mine || this.compact() || this.busy() || told === r.id) return;
      told = r.id;
      untracked(() => {
        const result = resolutionText(r, this.mctx()?.name());
        this.notices.info(
          this.feedback().trim()
            ? tr("{result}. Your note wasn't sent.", { result })
            : `${result}.`,
        );
      });
    });
  }

  // ────────────────────────────────────────────────────────── Permission

  allowOnce(): Promise<void> {
    return this.respond({ decision: 'allow' }, tr('Allowed'));
  }

  allowAlways(s: PermissionSuggestion): Promise<void> {
    return this.respond(
      { decision: 'allow', suggestion: s.index },
      tr('Allowed · {rule}', { rule: this.suggestionLabel(s) }),
    );
  }

  deny(): Promise<void> {
    return this.respond({ decision: 'deny' }, tr('Denied'));
  }

  denyWithFeedback(): Promise<void> {
    const message = this.feedback().trim();
    return this.respond(
      {
        decision: 'deny',
        ...(message ? { message } : {}),
        ...(this.stopTurn() ? { interrupt: true } : {}),
      },
      this.stopTurn() ? tr('Denied and stopped') : tr('Denied with feedback'),
    );
  }

  destination(d: string | undefined): string {
    const known = DESTINATIONS[d ?? ''];
    if (known) return tr(known);
    return d ? tr('Saved to {place}', { place: d }) : tr('Saved to your settings');
  }

  /** A suggestion's label: a run's standing grant in the language shown
   * (the server's English for the rest). */
  suggestionLabel(s: PermissionSuggestion): string {
    if (s.kind !== 'machineRunGrant') return s.label;
    const raw = s.raw as { prefix?: unknown } | null | undefined;
    const prefix = typeof raw?.prefix === 'string' ? raw.prefix : '';
    const name = machineRunInput(this.full()?.input)?.machineName ?? '';
    if (!prefix || !name) return s.label;
    return tr('Always allow “{prefix}” on {name} for this project', { prefix, name });
  }

  suggestionBlocked(s: PermissionSuggestion): boolean {
    return (s.standing || this.item().dangerous) && !this.canDangerous();
  }

  /** Rules saved to settings files: where they can be allowed instead. */
  protected standingText(): string {
    const m = this.mctx();
    return m
      ? tr(
          'Rules saved to settings files need {name}, or a computer it allows dangerous actions.',
          {
            name: m.name(),
          },
        )
      : tr(
          'Rules saved to settings files need the {computer}, or a device allowed to take dangerous actions (Settings › Phone & devices).',
          { computer: computerNoun() },
        );
  }

  // ─────────────────────────────────────────────────────────── Question

  isPicked(qi: number, label: string): boolean {
    return this.picks()[qi]?.includes(label) ?? false;
  }

  pick(qi: number, q: Question, label: string): void {
    this.picks.update((prev) => {
      const cur = prev[qi] ?? [];
      const next = q.multiSelect
        ? cur.includes(label)
          ? cur.filter((l) => l !== label)
          : [...cur, label]
        : [label];
      return { ...prev, [qi]: next };
    });
    if (!q.multiSelect) {
      this.other.update((prev) => ({ ...prev, [qi]: '' }));
      // One single-choice question: a tap is the answer.
      if (this.questions().length === 1) void this.submitAnswers();
    }
  }

  /** Quick answer from the list: needs the question text, so load it. */
  async quickAnswer(label: string): Promise<void> {
    const it = this.item();
    let full = this.full();
    if (!full) {
      this.loadingFull.set(true);
      try {
        full = await this.pendings().ensure(it.sessionId, it.id);
      } catch {
        full = undefined;
      } finally {
        this.loadingFull.set(false);
      }
    }
    const q = full?.questions?.[0];
    if (!q) {
      void this.router.navigate(this.link());
      return;
    }
    await this.respond(
      { decision: 'allow', answers: { [q.question]: label } },
      tr('Answered: {answer}', { answer: label }),
    );
  }

  setOther(qi: number, value: string): void {
    this.other.update((prev) => ({ ...prev, [qi]: value }));
    const q = this.questions()[qi];
    if (q && !q.multiSelect && value.trim()) this.picks.update((prev) => ({ ...prev, [qi]: [] }));
  }

  submitAnswers(): Promise<void> {
    const answers: Record<string, string> = {};
    this.questions().forEach((q, i) => {
      const picked = (this.picks()[i] ?? []).slice();
      picked.sort(
        (a, b) =>
          q.options.findIndex((o) => o.label === a) - q.options.findIndex((o) => o.label === b),
      );
      const free = this.other()[i]?.trim();
      const parts = q.multiSelect ? [...picked, ...(free ? [free] : [])] : [free || picked[0]];
      const value = parts.filter(Boolean).join(', ');
      if (value) answers[q.question] = value;
    });
    if (!Object.keys(answers).length) return Promise.resolve();
    return this.respond({ decision: 'allow', answers }, tr('Answer sent'));
  }

  // ──────────────────────────────────────────────────────────────── Plan

  approvePlan(mode: 'acceptEdits' | 'default'): Promise<void> {
    this.planSheet.set(false);
    return this.respond(
      { decision: 'allow', mode },
      mode === 'acceptEdits'
        ? tr('Approved · auto-accepting edits')
        : tr('Approved · reviewing edits'),
    );
  }

  keepPlanning(): Promise<void> {
    const message = this.feedback().trim();
    this.planSheet.set(false);
    return this.respond(
      { decision: 'deny', ...(message ? { message } : {}) },
      tr('Sent back to keep planning'),
    );
  }

  // ──────────────────────────────────────────────────────── Elicitation

  setField(key: string, value: unknown): void {
    this.form.update((prev) => ({ ...prev, [key]: value }));
  }

  accept(): Promise<void> {
    const content = this.form();
    return this.respond(
      { decision: 'allow', ...(Object.keys(content).length ? { content } : {}) },
      tr('Accepted'),
    );
  }

  // ────────────────────────────────────────────────────────── Keyboard

  onKey(e: KeyboardEvent): void {
    if (this.compact() || this.settled() || e.metaKey || e.ctrlKey || e.altKey) return;
    const t = e.target as HTMLElement | null;
    if (t && (t.isContentEditable || /^(INPUT|TEXTAREA|SELECT)$/.test(t.tagName))) return;
    const kind = this.item().kind;
    const key = e.key;
    let handled = true;
    if (key === 'j' || key === 'J') this.move.emit(1);
    else if (key === 'k' || key === 'K') this.move.emit(-1);
    else if (!this.canAct() || this.busy()) handled = false;
    else if (key === 'A' && e.shiftKey && kind === 'permission') {
      const s = this.firstAlways();
      if (s && !this.allowBlocked()) void this.allowAlways(s);
    } else if (key === 'a' && !e.shiftKey) {
      if (kind === 'permission' && !this.allowBlocked()) void this.allowOnce();
      else if (kind === 'plan') void this.approvePlan('default');
      else if (kind === 'elicitation') void this.accept();
      else handled = false;
    } else if (key === 'd' || key === 'D') void this.deny();
    else if ((key === 'f' || key === 'F') && kind === 'permission') {
      this.panel.set('feedback');
      queueMicrotask(() => this.host.querySelector<HTMLTextAreaElement>('textarea')?.focus());
    } else if (/^[1-9]$/.test(key) && kind === 'question') {
      const q = this.questions()[0];
      const opt = q?.options[Number(key) - 1];
      if (q && opt) this.pick(0, q, opt.label);
      else handled = false;
    } else handled = false;
    if (handled) e.preventDefault();
  }

  // ─────────────────────────────────────────────────────────────── Shared

  protected async retryLoad(): Promise<void> {
    const it = this.item();
    await this.loadFull(it.sessionId, it.id);
  }

  private async loadFull(sessionId: string, id: string): Promise<void> {
    if (this.loadingFull()) return;
    this.loadingFull.set(true);
    this.loadError.set('');
    try {
      const full = await this.pendings().ensure(sessionId, id);
      if (!full && id === this.itemId() && !this.gone()) {
        this.loadError.set(tr('This request is no longer available.'));
      }
    } catch (e) {
      if (id === this.itemId()) this.loadError.set(describeError(e).text);
    } finally {
      this.loadingFull.set(false);
    }
  }

  private async respond(body: Omit<RespondBody, 'requestId'>, doneText: string): Promise<void> {
    const item = this.item();
    this.error.set('');
    try {
      const res = await this.pendings().respond(item, body, doneText);
      if (!res) return; // already in flight elsewhere
      this.done.set(res.text);
      if (res.alreadyDecided) {
        this.resolved.emit({ item, body: null, alreadyDecided: true });
      } else {
        this.notices.announce(`${this.heading()}: ${res.text}`);
        this.resolved.emit({ item, body: { requestId: item.id, ...body }, alreadyDecided: false });
      }
    } catch (e) {
      this.error.set(describeError(e).text);
    }
  }
}

/** The server's title for a question it has no text for. */
const GENERIC_QUESTION = /^Claude has a question( for you)?\.?$/;

/** A title in the language shown. The server's own come from a fixed list
 * (`claude/control/protocol.rs`, `server/hooks.rs`, `claude/summaries.rs`:
 * "Claude has a question", "Claude wants to use Bash", "Allow Bash?"); any
 * other title (a question's own text, the CLI's) shows as it came. */
export function pendingTitle(title: string | undefined): string {
  const t = title?.trim() ?? '';
  if (GENERIC_QUESTION.test(t)) return tr('Claude has a question');
  const use = /^Claude wants to use (.+?)\.?$/.exec(t);
  if (use) return tr('Claude wants to use {tool}', { tool: use[1] });
  const allow = /^Allow (.+)\?$/.exec(t);
  if (allow) return tr('Allow {tool}?', { tool: allow[1] });
  const run = /^Run on (.+?): (.+)$/s.exec(t);
  if (run) return tr('Run on {name}: {command}', { name: run[1], command: run[2] });
  const task = /^Hand a task to (.+?): (.+)$/s.exec(t);
  if (task) return tr('Hand a task to {name}: {task}', { name: task[1], task: task[2] });
  return title ?? '';
}

/** A task approval's detail (`machines::work::tasks::source::approve`) in
 * the language shown; any other text as it came. */
export function taskDetail(detail: string): string {
  const m = /^In its own worktree of (.+) on (.+), at this session's code\.$/s.exec(detail);
  if (!m) return detail;
  return tr('In its own worktree of {folder} on {name}, at this session’s code.', {
    folder: m[1],
    name: m[2],
  });
}

/** A run approval's detail (`machines::work::source::approve`) in the
 * language shown; any other text as it came. */
export function runDetail(detail: string): string {
  const m = /^In (.+) on (.+) \((.+)\), in a work folder at this session's code\.$/s.exec(detail);
  if (!m) return detail;
  return tr('In {folder} on {name} ({shell}), in a work folder at this session’s code.', {
    folder: m[1],
    name: m[2],
    shell: m[3],
  });
}
