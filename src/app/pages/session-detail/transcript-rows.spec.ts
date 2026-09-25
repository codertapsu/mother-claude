import { UiEvent } from '../../core/models';
import { buildRows, sameRow, stepsSummary } from './transcript-rows';

function tool(id: string, name = 'Bash', summary = 'ls'): UiEvent {
  return { id: `${id}:0`, role: 'assistant', kind: 'tool', tool: { id, name, summary } };
}
function result(id: string, isError = false): UiEvent {
  return { id: `r-${id}:0`, role: 'user', kind: 'result', result: { toolUseId: id, isError } };
}
function text(id: string): UiEvent {
  return { id, role: 'assistant', kind: 'text', text: id };
}

describe('buildRows', () => {
  it('pairs results with their calls', () => {
    const rows = buildRows([text('a'), tool('t1'), result('t1'), text('b')]);
    expect(rows.map((r) => r.type)).toEqual(['event', 'tool', 'event']);
    const r = rows[1];
    expect(r.type === 'tool' && r.step.result?.id).toBe('r-t1:0');
  });

  it('collapses runs of three or more tool calls', () => {
    const rows = buildRows([
      tool('t1'),
      result('t1'),
      tool('t2'),
      result('t2'),
      tool('t3', 'Read', 'a.ts'),
      text('done'),
    ]);
    expect(rows.length).toBe(2);
    const g = rows[0];
    expect(g.type).toBe('steps');
    if (g.type === 'steps') {
      expect(g.id).toBe('steps:t1:0');
      expect(g.steps.length).toBe(3);
      expect(g.steps[2].result).toBeUndefined();
      expect(stepsSummary(g.steps)).toBe('Ran 3 steps · last: Read a.ts');
    }
  });

  it('keeps orphan results as their own rows', () => {
    const rows = buildRows([result('nope'), text('x')]);
    expect(rows[0].type).toBe('event');
  });

  it('treats identical rows as the same', () => {
    const events = [tool('t1'), result('t1')];
    const a = buildRows(events);
    const b = buildRows(events);
    expect(sameRow(a[0], b[0])).toBeTrue();
    const c = buildRows([tool('t1')]);
    expect(sameRow(a[0], c[0])).toBeFalse();
  });
});
