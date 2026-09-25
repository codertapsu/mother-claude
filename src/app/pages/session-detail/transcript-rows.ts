import { UiEvent } from '../../core/models';

/** A tool call with its result (when one arrived). */
export interface ToolStep {
  tool: UiEvent;
  result?: UiEvent;
}

export type Row =
  | { type: 'event'; id: string; ev: UiEvent }
  | { type: 'tool'; id: string; step: ToolStep }
  | { type: 'steps'; id: string; steps: ToolStep[] };

/** Runs of at least this many tool calls collapse into one row. */
export const GROUP_MIN = 3;

/**
 * Display rows from normalised transcript events:
 * - each tool call is paired with its result (by `toolUseId`);
 * - consecutive tool calls within a turn (nothing but tools and results
 *   between them) of ≥ {@link GROUP_MIN} collapse into one "Ran N steps"
 *   row, identified by its first call so the row keeps its identity as the
 *   run grows;
 * - results without a matching call stay as their own rows.
 */
export function buildRows(events: readonly UiEvent[]): Row[] {
  const byToolUse = new Map<string, UiEvent>();
  const toolIds = new Set<string>();
  for (const e of events) {
    if (e.kind === 'tool' && e.tool?.id) toolIds.add(e.tool.id);
  }
  for (const e of events) {
    const tid = e.kind === 'result' ? e.result?.toolUseId : undefined;
    if (tid && toolIds.has(tid)) byToolUse.set(tid, e);
  }

  const rows: Row[] = [];
  let run: ToolStep[] = [];
  const flush = () => {
    if (!run.length) return;
    if (run.length >= GROUP_MIN) {
      rows.push({ type: 'steps', id: `steps:${run[0].tool.id}`, steps: run });
    } else {
      for (const step of run) rows.push({ type: 'tool', id: step.tool.id, step });
    }
    run = [];
  };

  for (const e of events) {
    if (e.kind === 'tool') {
      run.push({ tool: e, result: e.tool?.id ? byToolUse.get(e.tool.id) : undefined });
      continue;
    }
    if (e.kind === 'result' && e.result?.toolUseId && toolIds.has(e.result.toolUseId)) {
      continue; // shown with its call
    }
    flush();
    rows.push({ type: 'event', id: e.id, ev: e });
  }
  flush();
  return rows;
}

/** Same content ⇒ the previous row object can be kept (OnPush rows then
 * skip re-rendering). Events are compared by reference: the store replaces
 * an event object only when its content changed. */
export function sameRow(a: Row, b: Row): boolean {
  if (a.type !== b.type || a.id !== b.id) return false;
  if (a.type === 'event' && b.type === 'event') return a.ev === b.ev;
  if (a.type === 'tool' && b.type === 'tool') {
    return a.step.tool === b.step.tool && a.step.result === b.step.result;
  }
  if (a.type === 'steps' && b.type === 'steps') {
    return (
      a.steps.length === b.steps.length &&
      a.steps.every((s, i) => s.tool === b.steps[i].tool && s.result === b.steps[i].result)
    );
  }
  return false;
}

/** "Ran 12 steps · last: Bash npm test". */
export function stepsSummary(steps: readonly ToolStep[]): string {
  const last = steps[steps.length - 1]?.tool.tool;
  const tail = last ? ` · last: ${last.name}${last.summary ? ` ${last.summary}` : ''}` : '';
  return `Ran ${steps.length} steps${tail}`;
}
