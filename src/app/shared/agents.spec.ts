import { AgentInfo, BackgroundTask } from '../core/models';
import { isTaskAgent, mergeBackgroundTasks, orderAgentTree, taskFromAgent } from './agents';

const agent = (id: string, over: Partial<AgentInfo> = {}): AgentInfo => ({
  id,
  kind: 'subagent',
  sessionId: 's1',
  depth: 1,
  label: id,
  status: 'running',
  background: false,
  canStop: false,
  ...over,
});

const task = (id: string, over: Partial<BackgroundTask> = {}): BackgroundTask => ({
  id,
  kind: 'bash',
  label: id,
  status: 'running',
  outputAvailable: true,
  ...over,
});

describe('shared/agents', () => {
  it('recognises shells and monitors as tasks, not transcript agents', () => {
    expect(isTaskAgent({ kind: 'bash' })).toBeTrue();
    expect(isTaskAgent({ kind: 'monitor' })).toBeTrue();
    expect(isTaskAgent({ kind: 'subagent' })).toBeFalse();
    expect(isTaskAgent({ kind: 'workflow-agent' })).toBeFalse();
    expect(isTaskAgent({ kind: 'teammate' })).toBeFalse();
  });

  it('turns a shell agent into its task row, keyed by the task id', () => {
    const t = taskFromAgent(
      agent('b3x9', { kind: 'monitor', taskId: 'b3x9', label: 'tail -f log', startedAt: 5 }),
    );
    expect(t).toEqual({
      id: 'b3x9',
      kind: 'monitor',
      label: 'tail -f log',
      status: 'running',
      startedAt: 5,
      endedAt: undefined,
      outputAvailable: true,
    });
  });

  it('lists every shell once and drops async subagent tasks shown as agents', () => {
    const agents = [
      agent('b1', { kind: 'bash', taskId: 'b1' }),
      agent('b2', { kind: 'bash', taskId: 'b2', label: 'only as agent' }),
      agent('a1', { kind: 'subagent', background: true, taskId: 'a1' }),
    ];
    const tasks = [
      task('b1', { label: 'from detail' }),
      task('a1', { kind: 'agent' }),
      task('w1', { kind: 'workflow' }),
    ];
    const merged = mergeBackgroundTasks(tasks, agents);
    // The workflow task stays: it is the run's Stop/Output row.
    expect(merged.map((t) => t.id)).toEqual(['b1', 'w1', 'b2']);
    // The detail's own row wins over the agent-derived one.
    expect(merged[0].label).toBe('from detail');
    expect(merged[2].label).toBe('only as agent');
  });

  it('orders parents before children', () => {
    const list = orderAgentTree([
      agent('c', { parentId: 'p', startedAt: 2 }),
      agent('p', { startedAt: 1 }),
    ]);
    expect(list.map((a) => [a.id, a.level])).toEqual([
      ['p', 0],
      ['c', 1],
    ]);
  });
});
