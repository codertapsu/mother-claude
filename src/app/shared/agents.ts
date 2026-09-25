import { AgentInfo, BackgroundTask } from '../core/models';
import { IconName } from './icon.component';

const KIND_ICONS: Record<string, IconName> = {
  subagent: 'agents',
  'workflow-agent': 'workflow',
  bash: 'terminal',
  monitor: 'activity',
  teammate: 'user',
};

const KIND_LABELS: Record<string, string> = {
  subagent: 'Subagent',
  'workflow-agent': 'Workflow agent',
  bash: 'Background shell',
  monitor: 'Monitor',
  teammate: 'Teammate',
};

export function agentKindIcon(a: Pick<AgentInfo, 'kind'>): IconName {
  return KIND_ICONS[a.kind] ?? 'agents';
}

export function agentKindLabel(a: Pick<AgentInfo, 'kind'>): string {
  return KIND_LABELS[a.kind] ?? a.kind;
}

/** A background shell or monitor. The server lists these among a session's
 * agents (they count as background work), but they have no transcript of
 * their own: their output comes from `GET /sessions/{id}/tasks/{taskId}/output`,
 * so they must never link to the agent (transcript) view. */
export function isTaskAgent(a: Pick<AgentInfo, 'kind'>): boolean {
  return a.kind === 'bash' || a.kind === 'monitor';
}

/** The background-task row for a shell/monitor agent. */
export function taskFromAgent(a: AgentInfo, outputAvailable = true): BackgroundTask {
  return {
    id: a.taskId ?? a.id,
    kind: a.kind === 'monitor' ? 'monitor' : 'bash',
    label: a.label,
    status: a.status,
    startedAt: a.startedAt,
    endedAt: a.endedAt,
    outputAvailable,
  };
}

/**
 * One list of background tasks without duplicates:
 * - shells/monitors the server only reported as agents are added (by task id);
 * - an async subagent's task is dropped: it has its own agent row (with Stop),
 *   whose `taskId` is that task.
 */
export function mergeBackgroundTasks(
  tasks: readonly BackgroundTask[],
  agents: readonly AgentInfo[],
): BackgroundTask[] {
  const shownElsewhere = new Set<string>();
  for (const a of agents) if (!isTaskAgent(a) && a.taskId) shownElsewhere.add(a.taskId);
  const out: BackgroundTask[] = [];
  const seen = new Set<string>();
  for (const t of tasks) {
    if (shownElsewhere.has(t.id) || seen.has(t.id)) continue;
    seen.add(t.id);
    out.push(t);
  }
  for (const a of agents) {
    if (!isTaskAgent(a)) continue;
    const t = taskFromAgent(a);
    if (seen.has(t.id)) continue;
    seen.add(t.id);
    out.push(t);
  }
  return out;
}

/** Parents before their children (depth-first), roots by start time. The
 * returned `level` is the nesting depth within this list. */
export function orderAgentTree(agents: readonly AgentInfo[]): (AgentInfo & { level: number })[] {
  const ids = new Set(agents.map((a) => a.id));
  const children = new Map<string, AgentInfo[]>();
  const roots: AgentInfo[] = [];
  for (const a of agents) {
    if (a.parentId && ids.has(a.parentId) && a.parentId !== a.id) {
      const list = children.get(a.parentId) ?? [];
      list.push(a);
      children.set(a.parentId, list);
    } else {
      roots.push(a);
    }
  }
  const byStart = (x: AgentInfo, y: AgentInfo) => (x.startedAt ?? 0) - (y.startedAt ?? 0);
  const out: (AgentInfo & { level: number })[] = [];
  const seen = new Set<string>();
  const visit = (a: AgentInfo, level: number) => {
    if (seen.has(a.id)) return;
    seen.add(a.id);
    out.push({ ...a, level });
    for (const c of (children.get(a.id) ?? []).sort(byStart)) visit(c, level + 1);
  };
  [...roots].sort(byStart).forEach((a) => visit(a, 0));
  return out;
}

/** Tone for an agent/task status chip. */
export function agentStatusTone(status: string): 'green' | 'red' | 'muted' | 'amber' {
  if (status === 'running') return 'green';
  if (status === 'failed' || status === 'killed') return 'red';
  if (status === 'stopped') return 'amber';
  return 'muted';
}
