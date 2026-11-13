import { Injectable, computed, signal } from '@angular/core';

import { tr } from '../../core/i18n/i18n';
import { PeerMachine } from '../../core/machines';
import {
  MAX_ROLES,
  MERGE_NODE,
  NodePosition,
  TeamTemplate,
  TemplateIssue,
  TemplateReport,
  parseTemplate,
  templateJson,
} from '../../core/teams';
import {
  IssueTarget,
  Selection,
  boxOf,
  canRemove,
  connect,
  disconnect,
  duplicateRole,
  freeSpot,
  hasHandoff,
  issueTarget,
  leadOf,
  localIssues,
  makeLead,
  newRole,
  newRoleId,
  renameRole,
  resolvedLayout,
  roleIdProblem,
  roleOf,
  rolesOf,
  titleOf,
  addRole as withRole,
  removeRole as withoutRole,
} from './template-graph';

/** A problem as the editor shows it: on its node or edge, and in the list. */
export interface EditorIssue extends TemplateIssue {
  level: 'error' | 'warning';
  target: IssueTarget;
  /** The server said it about the template before the last change. */
  stale: boolean;
}

const EMPTY: TeamTemplate = { id: '', name: '', roles: [] };

/**
 * The template sheet's editing state, shared by its canvas and its panel
 * (provided by `TemplateSheetComponent`): the template being edited — the
 * parsed object, so fields this version does not know survive — its JSON
 * text, what is selected, and its problems: the instant ones
 * (`localIssues`) and what the server said at the last check.
 */
@Injectable()
export class TemplateEditor {
  readonly template = signal<TeamTemplate>(EMPTY);
  /** The JSON tab's text: the template's, or what is being typed there. */
  readonly text = signal('');
  /** Why the JSON tab's text is not a template ('' when it is). */
  readonly parseError = signal('');
  readonly readOnly = signal(false);
  /** A template not saved yet: its id may still change. */
  readonly isNew = signal(false);
  readonly selection = signal<Selection>(null);
  /** The paired computers a role may run on. */
  readonly peers = signal<PeerMachine[]>([]);
  /** Counts loads: the canvas fits each new template into view. */
  readonly loads = signal(0);
  /** Counts changes: a check made before the last one is out of date. */
  readonly revision = signal(0);
  private readonly report = signal<{ items: EditorIssue[]; revision: number } | null>(null);

  /** Nothing is editable in a built-in template, nor while the JSON tab's
   * text does not parse (the canvas shows the last template that did). */
  readonly editable = computed(() => !this.readOnly() && !this.parseError());
  readonly positions = computed(() => resolvedLayout(this.template()));
  readonly roleCount = computed(() => rolesOf(this.template()).length);
  readonly full = computed(() => this.roleCount() >= MAX_ROLES);

  /**
   * Every problem: what the last check said (all of it while it is current;
   * once the template changed, only where the instant checks are silent,
   * marked out of date) and the instant checks where that check said
   * nothing.
   */
  readonly issues = computed<EditorIssue[]>(() => {
    const t = this.template();
    const local: EditorIssue[] = localIssues(t).map((i) => ({
      ...i,
      level: 'error',
      target: issueTarget(t, i.path),
      stale: false,
    }));
    const r = this.report();
    const stale = !!r && r.revision !== this.revision();
    const server = (r?.items ?? []).map((i) => ({ ...i, stale }));
    const fresh = new Set(server.filter((i) => !i.stale).map((i) => i.path));
    const mine = new Set(local.map((i) => i.path));
    return [
      ...server.filter((i) => !i.stale),
      ...local.filter((i) => !fresh.has(i.path)),
      ...server.filter((i) => i.stale && !mine.has(i.path)),
    ];
  });

  /** Start editing `t`. */
  load(t: TeamTemplate, opts: { readOnly: boolean; isNew: boolean }): void {
    this.template.set(t);
    this.text.set(templateJson(t));
    this.parseError.set('');
    this.readOnly.set(opts.readOnly);
    this.isNew.set(opts.isNew);
    this.selection.set(null);
    this.report.set(null);
    this.revision.set(0);
    this.loads.update((n) => n + 1);
  }

  /** What the server found in `checked` (the template as it was sent, at
   * `revision`). */
  setReport(r: TemplateReport | null, checked: TeamTemplate, revision = this.revision()): void {
    if (!r) {
      this.report.set(null);
      return;
    }
    const map =
      (level: EditorIssue['level']) =>
      (i: TemplateIssue): EditorIssue => ({
        ...i,
        level,
        target: issueTarget(checked, i.path),
        stale: false,
      });
    this.report.set({
      revision,
      items: [...r.errors.map(map('error')), ...r.warnings.map(map('warning'))],
    });
  }

  /** Take `next` as the template (from the canvas or the panel). */
  apply(next: TeamTemplate): void {
    if (!this.editable() || next === this.template()) return;
    this.template.set(next);
    this.text.set(templateJson(next));
    this.revision.update((n) => n + 1);
    this.keepSelection();
  }

  /** Text typed in the JSON tab: the template when it parses. */
  setText(value: string): void {
    if (this.readOnly()) return;
    this.text.set(value);
    this.revision.update((n) => n + 1);
    const parsed = parseTemplate(value);
    if (!parsed.template) {
      this.parseError.set(parsed.error ?? '');
      return;
    }
    this.parseError.set('');
    this.template.set(parsed.template);
    this.keepSelection();
  }

  /** Drop the JSON tab's text for the last template that parsed. */
  restoreText(): void {
    this.text.set(templateJson(this.template()));
    this.parseError.set('');
  }

  select(s: Selection): void {
    this.selection.set(s);
  }

  /** A new role at `at`, selected (the lead, when there is none). */
  addRole(at: NodePosition): string | null {
    if (!this.editable() || this.full()) return null;
    const t = this.template();
    const { id, n } = newRoleId(t);
    const role = newRole(id, tr('Role {n}', { n }));
    if (!leadOf(t)) role.lead = true;
    this.apply(withRole(t, role, at));
    this.select({ kind: 'role', id });
    return id;
  }

  /** A copy of role `id` beside it, selected. */
  duplicateRole(id: string): string | null {
    if (!this.editable() || this.full()) return null;
    const t = this.template();
    const from = this.positions()[id];
    if (!from) return null;
    const taken = Object.values(this.positions()).map(boxOf);
    const at = freeSpot(taken, { x: from.x + 32, y: from.y + 32 });
    const copy = duplicateRole(t, id, tr('{title} (copy)', { title: titleOf(t, id) }), at);
    if (!copy) return null;
    this.apply(copy.template);
    this.select({ kind: 'role', id: copy.id });
    return copy.id;
  }

  /** Delete role `id` once the user confirms: its handoffs go with it. */
  deleteRole(id: string): boolean {
    const t = this.template();
    if (!this.editable() || !canRemove(t, id)) return false;
    const ask = tr('Delete the {role} role? Its handoffs go with it.', { role: titleOf(t, id) });
    if (!confirm(ask)) return false;
    this.apply(withoutRole(t, id));
    this.select(null);
    return true;
  }

  makeLead(id: string): void {
    this.apply(makeLead(this.template(), id));
  }

  /** Rename role `from`: why it can't be `to`, or ''. */
  renameRole(from: string, to: string): string {
    const t = this.template();
    const problem = roleIdProblem(t, to, from);
    if (problem || from === to || !roleOf(t, from)) return problem;
    const s = this.selection();
    this.apply(renameRole(t, from, to));
    if (s?.kind === 'role' && s.id === from) this.select({ kind: 'role', id: to });
    return '';
  }

  /** `from` hands its work to `to` (to the merge node: it merges), and the
   * new handoff is selected (to note what passes along). */
  connect(from: string, to: string): void {
    this.apply(connect(this.template(), from, to));
    this.select(to === MERGE_NODE ? { kind: 'merge' } : { kind: 'handoff', from, to });
  }

  disconnect(from: string, to: string): void {
    this.apply(disconnect(this.template(), from, to));
  }

  /** The selection still names something after a change. */
  private keepSelection(): void {
    const s = this.selection();
    const t = this.template();
    if (s?.kind === 'role' && !roleOf(t, s.id)) this.selection.set(null);
    if (s?.kind === 'handoff' && !hasHandoff(t, s.from, s.to)) this.selection.set(null);
  }
}
