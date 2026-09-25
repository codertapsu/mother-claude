import {
  ChangeDetectionStrategy,
  Component,
  computed,
  inject,
  input,
  linkedSignal,
  output,
  signal,
} from '@angular/core';
import { FormsModule } from '@angular/forms';
import { RouterLink } from '@angular/router';

import { ApiError, ApiService, errorMessage } from '../core/api.service';
import { AuthService } from '../core/auth.service';
import {
  PendingItem,
  PermissionSuggestion,
  Question,
  RespondBody,
  SessionSummary,
} from '../core/models';
import { IconComponent, IconName } from './icon.component';
import { renderMarkdown } from './markdown';
import { RelativeTimePipe } from './time.pipes';
import { sessionName, surfaceLabel } from './util';

/** What the card emits once the server accepted an answer (or someone else
 * answered first — `alreadyDecided`). Parents may hide the card right away;
 * the item also disappears from the live store when the server withdraws it. */
export interface PendingResolution {
  item: PendingItem;
  body: RespondBody | null;
  alreadyDecided: boolean;
}

type Panel = 'main' | 'always' | 'feedback' | 'planFeedback';

/** Where an "always" rule is saved (Claude Code settings destinations). */
const DESTINATIONS: Record<string, string> = {
  session: 'this session only',
  localSettings: 'this project, just you',
  projectSettings: 'this project, shared',
  userSettings: 'all your projects',
  cliArg: 'this run',
};

/**
 * The one pending-item card used everywhere (Inbox, session detail, session
 * list): permission, AskUserQuestion, ExitPlanMode plan and MCP elicitation.
 *
 * Inputs:
 * - `item` (required) — the PendingItem.
 * - `session` — its session (for surface/capability guidance and the header).
 * - `compact` — one-line version for list cards: Allow/Deny, or the options of
 *   a single simple question, else an "Open" link.
 * - `showSession` — show which session asks (Inbox).
 * Output: `resolved` ({@link PendingResolution}).
 *
 * Permission: Allow once · Always… (the item's suggestions) · Deny · Deny with
 * feedback (optionally stopping the turn). Question: every question, single or
 * multi-select, plus free-text "Other". Plan: rendered markdown, Approve &
 * auto-accept edits · Approve & review edits · Keep planning with feedback.
 * `observed` / unanswerable items render read-only with guidance.
 * Dangerous approvals (and standing "always" rules) need a local client or a
 * device the desktop granted "allow dangerous"; Deny always works.
 */
@Component({
  selector: 'mc-pending-card',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [FormsModule, RouterLink, IconComponent, RelativeTimePipe],
  templateUrl: './pending-card.component.html',
  styleUrl: './pending-card.component.scss',
  host: {
    '[attr.data-kind]': 'item().kind',
    '[class.compact]': 'compact()',
    '[class.dangerous]': 'item().dangerous',
  },
})
export class PendingCardComponent {
  private api = inject(ApiService);
  private auth = inject(AuthService);

  readonly item = input.required<PendingItem>();
  readonly session = input<SessionSummary | undefined>(undefined);
  readonly compact = input(false);
  readonly showSession = input(false);
  readonly resolved = output<PendingResolution>();

  /** Local state resets whenever a different item is shown. */
  private readonly itemId = computed(() => this.item().id);
  protected readonly busy = signal(false);
  protected readonly error = linkedSignal({ source: this.itemId, computation: () => '' });
  protected readonly done = linkedSignal({ source: this.itemId, computation: () => '' });
  protected readonly panel = linkedSignal<string, Panel>({
    source: this.itemId,
    computation: () => 'main',
  });
  protected readonly feedback = linkedSignal({ source: this.itemId, computation: () => '' });
  protected readonly stopTurn = linkedSignal({ source: this.itemId, computation: () => false });
  /** Per question index: chosen option labels. */
  protected readonly picks = linkedSignal<string, Record<number, string[]>>({
    source: this.itemId,
    computation: () => ({}),
  });
  /** Per question index: free-text "Other" answer. */
  protected readonly other = linkedSignal<string, Record<number, string>>({
    source: this.itemId,
    computation: () => ({}),
  });
  protected readonly showInput = signal(false);
  protected readonly planOpen = signal(false);

  protected readonly canAct = computed(
    () => this.item().answerable && this.item().source !== 'observed',
  );
  protected readonly canDangerous = this.auth.canDangerous;
  /** Allowing is blocked for this client (Deny still works). */
  protected readonly allowBlocked = computed(() => this.item().dangerous && !this.canDangerous());
  protected readonly questions = computed<Question[]>(() => this.item().questions ?? []);
  protected readonly simpleQuestion = computed(() => {
    const qs = this.questions();
    return (
      qs.length === 1 && !qs[0].multiSelect && qs[0].options.length > 0 && qs[0].options.length <= 4
    );
  });
  protected readonly planHtml = computed(() => renderMarkdown(this.item().plan ?? ''));
  protected readonly planLong = computed(() => (this.item().plan ?? '').split('\n').length > 14);
  protected readonly inputJson = computed(() => {
    const input = this.item().input;
    if (input === undefined || input === null) return '';
    try {
      const text = JSON.stringify(input, null, 2);
      return text && text !== '{}' ? text : '';
    } catch {
      return '';
    }
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
        return 'shield';
    }
  });
  protected readonly heading = computed(() => {
    const it = this.item();
    switch (it.kind) {
      case 'question':
        return it.title || 'Claude has a question';
      case 'plan':
        return 'Plan ready for review';
      case 'elicitation':
        return it.title || 'Input requested';
      default:
        return it.title || `Allow ${it.toolName ?? 'this tool'}?`;
    }
  });
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
  /** Where an unanswerable prompt can be answered. */
  protected readonly guidance = computed(() => {
    const s = this.session();
    const where = s ? `in ${this.surface()}` : 'where the session runs';
    const parts = [`Answer it ${where}.`];
    if (s?.control.continueOwned || s?.control.fork) {
      parts.push(
        'Or open the session and use “Continue here” to take it over as an owned session.',
      );
    }
    if (s && !s.control.hooksActive && s.control.channel === 'none') {
      parts.push(
        'Installing foreign-session control (on the desktop: Remote → Foreign sessions) lets you answer prompts like this from here.',
      );
    }
    return parts.join(' ');
  });

  // ────────────────────────────────────────────────────────── Permission

  allowOnce(): Promise<void> {
    return this.respond({ decision: 'allow' }, 'Allowed');
  }

  allowAlways(s: PermissionSuggestion): Promise<void> {
    return this.respond({ decision: 'allow', suggestion: s.index }, `Allowed · ${s.label}`);
  }

  deny(): Promise<void> {
    return this.respond({ decision: 'deny' }, 'Denied');
  }

  denyWithFeedback(): Promise<void> {
    const message = this.feedback().trim();
    return this.respond(
      {
        decision: 'deny',
        ...(message ? { message } : {}),
        ...(this.stopTurn() ? { interrupt: true } : {}),
      },
      this.stopTurn() ? 'Denied and stopped' : 'Denied with feedback',
    );
  }

  destination(d: string | undefined): string {
    return DESTINATIONS[d ?? ''] ?? (d ? `saved to ${d}` : 'saved to settings');
  }

  suggestionBlocked(s: PermissionSuggestion): boolean {
    return (s.standing || this.item().dangerous) && !this.canDangerous();
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

  setOther(qi: number, value: string): void {
    this.other.update((prev) => ({ ...prev, [qi]: value }));
    const q = this.questions()[qi];
    if (q && !q.multiSelect && value.trim()) this.picks.update((prev) => ({ ...prev, [qi]: [] }));
  }

  submitAnswers(): Promise<void> {
    const answers: Record<string, string> = {};
    this.questions().forEach((q, i) => {
      const picked = (this.picks()[i] ?? []).slice();
      // Keep the question's own option order for multi-select answers.
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
    return this.respond({ decision: 'allow', answers }, 'Answer sent');
  }

  // ──────────────────────────────────────────────────────────────── Plan

  approvePlan(mode: 'acceptEdits' | 'default'): Promise<void> {
    return this.respond(
      { decision: 'allow', mode },
      mode === 'acceptEdits' ? 'Approved · auto-accepting edits' : 'Approved · reviewing edits',
    );
  }

  keepPlanning(): Promise<void> {
    const message = this.feedback().trim();
    return this.respond(
      { decision: 'deny', ...(message ? { message } : {}) },
      'Sent back to keep planning',
    );
  }

  // ─────────────────────────────────────────────────────────────── Shared

  private async respond(body: Omit<RespondBody, 'requestId'>, doneText: string): Promise<void> {
    if (this.busy()) return;
    const item = this.item();
    const full: RespondBody = { requestId: item.id, ...body };
    this.busy.set(true);
    this.error.set('');
    try {
      await this.api.respond(item.sessionId, full);
      this.done.set(doneText);
      this.resolved.emit({ item, body: full, alreadyDecided: false });
    } catch (e) {
      if (e instanceof ApiError && (e.status === 409 || e.status === 410)) {
        this.done.set('Already answered elsewhere');
        this.resolved.emit({ item, body: null, alreadyDecided: true });
      } else if (e instanceof ApiError && e.status === 403) {
        this.error.set(
          `${errorMessage(e)} — this needs the desktop app, or a device the desktop allowed to approve dangerous actions.`,
        );
      } else {
        this.error.set(errorMessage(e));
      }
    } finally {
      this.busy.set(false);
    }
  }
}
