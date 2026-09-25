import { AgentInfo } from '../core/models';
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
