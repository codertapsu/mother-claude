import { MERGE_NODE, NodePosition, TeamTemplate } from '../../core/teams';
import {
  GRID,
  NODE_H,
  NODE_W,
  addRole,
  arrange,
  autoLayout,
  boxOf,
  canConnect,
  canRemove,
  connect,
  disconnect,
  duplicateRole,
  edgePath,
  flowSummary,
  freeSpot,
  isRoleId,
  issueTarget,
  localIssues,
  makeLead,
  mergerOf,
  moveNode,
  newRole,
  newRoleId,
  removeRole,
  renameRole,
  resolvedLayout,
  roleIdProblem,
  routeEdge,
  selectionOf,
  setHandoffNote,
  setMergeBy,
  slugId,
  uniqueRoleId,
  updateIntegration,
  updateRole,
  withLayout,
} from './template-graph';

/** Planner (lead) → writer → qa, a designer beside them; unknown fields on
 * every level. */
const template = (): TeamTemplate => ({
  id: 'video',
  name: 'Video',
  roles: [
    { id: 'planner', title: 'Planner', lead: true, model: 'opus', future: { a: 1 } },
    { id: 'writer', title: 'Writer', model: 'sonnet' },
    { id: 'designer', title: 'Designer' },
    { id: 'qa', title: 'QA', readOnly: true },
  ],
  flow: [
    { from: 'writer', to: 'qa', note: 'the script', weight: 2 },
    { from: 'designer', to: 'qa' },
  ],
  integration: { branch: 'main', mergeBy: 'qa', strategy: 'squash' },
  idleStopMinutes: 20,
  extra: [1, 2],
});

const overlaps = (a: NodePosition, b: NodePosition) =>
  a.x < b.x + NODE_W && b.x < a.x + NODE_W && a.y < b.y + NODE_H && b.y < a.y + NODE_H;

describe('template graph', () => {
  describe('auto layout', () => {
    it('layers the roles left to right by the flow: the lead first, Merge last', () => {
      const pos = autoLayout(template());
      expect(pos['planner'].x).toBeLessThan(pos['writer'].x);
      expect(pos['writer'].x).toBe(pos['designer'].x);
      expect(pos['qa'].x).toBeGreaterThan(pos['writer'].x);
      expect(pos[MERGE_NODE].x).toBeGreaterThan(pos['qa'].x);
      // On the grid, and no two nodes of a layer overlap.
      for (const p of Object.values(pos)) {
        expect(p.x % GRID).toBe(0);
        expect(p.y % GRID).toBe(0);
      }
      expect(overlaps(pos['writer'], pos['designer'])).toBe(false);
      // The lead and Merge sit in the middle of the tallest layer.
      expect(pos['planner'].y).toBe(pos[MERGE_NODE].y);
    });

    it('puts roles without a handoff in the first layer after the lead', () => {
      const t = template();
      t.flow = [];
      const pos = autoLayout(t);
      const xs = new Set(['writer', 'designer', 'qa'].map((id) => pos[id].x));
      expect(xs.size).toBe(1);
      expect(pos[MERGE_NODE].x).toBeGreaterThan(pos['qa'].x);
    });

    it('breaks a cycle where it closes and never moves the lead for work handed back', () => {
      const t = template();
      t.flow = [
        { from: 'writer', to: 'qa' },
        { from: 'qa', to: 'writer' },
        { from: 'qa', to: 'planner' },
      ];
      const pos = autoLayout(t);
      expect(pos['planner'].x).toBeLessThan(pos['writer'].x);
      expect(pos['writer'].x).toBeLessThan(pos['qa'].x);
    });

    it('places a template without roles, and one without a lead', () => {
      expect(autoLayout({ id: 'x', name: 'X', roles: [] })[MERGE_NODE]).toBeDefined();
      const t = template();
      t.roles[0].lead = false;
      const pos = autoLayout(t);
      expect(pos['planner']).toBeDefined();
      expect(pos[MERGE_NODE].x).toBeGreaterThan(pos['qa'].x);
    });
  });

  describe('layout', () => {
    it('uses the stored places, and places the others clear of them', () => {
      const t = template();
      t.layout = { planner: { x: 400, y: 400 }, qa: { x: 'no' } as unknown as NodePosition };
      const pos = resolvedLayout(t);
      expect(pos['planner']).toEqual({ x: 400, y: 400 });
      for (const k of ['writer', 'designer', 'qa', MERGE_NODE]) {
        expect(overlaps(pos[k], pos['planner']), k).toBe(false);
      }
    });

    it('stores every place on the first move, snapped to the grid, other keys kept', () => {
      const t = template();
      t.layout = { '@viewport': { x: 1, y: 2 } };
      const moved = moveNode(t, 'writer', { x: 103, y: 41 });
      expect(moved.layout?.['writer']).toEqual({ x: 96, y: 48 });
      expect(Object.keys(moved.layout ?? {})).toEqual([
        '@viewport',
        'planner',
        'writer',
        'designer',
        'qa',
        MERGE_NODE,
      ]);
      expect(moved.layout?.['@viewport']).toEqual({ x: 1, y: 2 });
      // The others stay where they showed.
      expect(moved.layout?.['qa']).toEqual(resolvedLayout(t)['qa']);
      expect(t.layout).toEqual({ '@viewport': { x: 1, y: 2 } });
    });

    it('leaves a complete layout alone, and auto-arranges every node', () => {
      const placed = withLayout(template());
      expect(withLayout(placed)).toBe(placed);
      const scattered = moveNode(placed, 'planner', { x: 2000, y: 2000 });
      const arranged = arrange(scattered);
      expect(arranged.layout?.['planner']).toEqual(autoLayout(scattered)['planner']);
    });

    it('finds a free spot near the one asked for', () => {
      const taken = [boxOf({ x: 0, y: 0 })];
      const spot = freeSpot(taken, { x: 8, y: 8 });
      expect(overlaps(spot, { x: 0, y: 0 })).toBe(false);
      expect(Math.hypot(spot.x, spot.y)).toBeLessThan(NODE_W + 64);
      expect(freeSpot([], { x: 7, y: 9 })).toEqual({ x: 0, y: 16 });
    });

    it('routes a long edge around the nodes in its way', () => {
      const a = boxOf({ x: 0, y: 0 });
      const between = boxOf({ x: 272, y: 0 });
      const b = boxOf({ x: 544, y: 0 });
      const over = routeEdge(a, b, [between], 'above');
      expect(over.d).toContain(' L');
      expect(over.mid.y).toBeLessThan(0);
      const under = routeEdge(a, b, [between], 'below', 40);
      expect(under.mid.y).toBe(NODE_H + 40);
      // Work sent back to the left loops under both.
      const back = routeEdge(b, a, [between], 'below');
      expect(back.d).toContain(' L');
      expect(back.mid.y).toBe(NODE_H + 24);
      expect(back.head.startsWith('-2,')).toBe(true);
      // Nothing in the way, or next to each other: the plain curve.
      expect(routeEdge(a, b, [boxOf({ x: 272, y: 400 })], 'above')).toEqual(edgePath(a, b));
      expect(routeEdge(a, between, [b], 'above')).toEqual(edgePath(a, between));
    });

    it('draws an edge from the right edge of one node into the left edge of another', () => {
      const e = edgePath(boxOf({ x: 0, y: 0 }), boxOf({ x: 400, y: 0 }));
      expect(e.d.startsWith(`M${NODE_W},${NODE_H / 2} C`)).toBe(true);
      expect(e.head.startsWith(`398,${NODE_H / 2} `)).toBe(true);
      expect(e.mid.y).toBe(NODE_H / 2);
    });
  });

  describe('roles', () => {
    it('adds a role with a new id at its place', () => {
      const t = template();
      const { id, n } = newRoleId(t);
      expect([id, n]).toEqual(['role-5', 5]);
      const next = addRole(t, newRole(id, 'Role 5'), { x: 33, y: 47 });
      expect(next.roles.at(-1)).toEqual({
        id: 'role-5',
        title: 'Role 5',
        model: 'sonnet',
        permissionMode: 'default',
      });
      expect(next.layout?.['role-5']).toEqual({ x: 32, y: 48 });
      expect(uniqueRoleId(next, 'writer')).toBe('writer-2');
      expect(uniqueRoleId(next, 'a'.repeat(40)).length).toBeLessThanOrEqual(32);
    });

    it('duplicates a role without its lead, keeping its unknown fields', () => {
      const copy = duplicateRole(template(), 'planner', 'Planner (copy)', { x: 0, y: 0 });
      const role = copy?.template.roles.find((r) => r.id === copy?.id);
      expect(copy?.id).toBe('planner-copy');
      expect(role?.lead).toBeUndefined();
      expect(role?.['future']).toEqual({ a: 1 });
      expect(role?.title).toBe('Planner (copy)');
    });

    it('deletes a role with its handoffs, its place and who merges', () => {
      const t = withLayout(template());
      const next = removeRole(t, 'qa');
      expect(next.roles.map((r) => r.id)).toEqual(['planner', 'writer', 'designer']);
      expect(next.flow).toEqual([]);
      expect(next.integration).toEqual({ branch: 'main', strategy: 'squash' });
      expect(next.layout?.['qa']).toBeUndefined();
      expect(mergerOf(next)).toEqual({ id: 'planner', explicit: false });
      expect(canRemove(t, 'planner')).toBe(false);
      expect(canRemove(t, 'writer')).toBe(true);
    });

    it('makes one role the lead, and the lead runs here', () => {
      const t = updateRole(template(), 'writer', { machine: 'os:windows' });
      const next = makeLead(t, 'writer');
      expect(next.roles.filter((r) => r.lead).map((r) => r.id)).toEqual(['writer']);
      expect(next.roles[1].machine).toBeUndefined();
      expect('lead' in next.roles[0]).toBe(false);
      expect(next.roles[0]['future']).toEqual({ a: 1 });
    });

    it('renames a role in its handoffs, who merges and the layout', () => {
      const t = withLayout(template());
      const next = renameRole(t, 'qa', 'checker');
      expect(next.roles[3].id).toBe('checker');
      expect(next.flow).toEqual([
        { from: 'writer', to: 'checker', note: 'the script', weight: 2 },
        { from: 'designer', to: 'checker' },
      ]);
      expect(next.integration?.mergeBy).toBe('checker');
      expect(Object.keys(next.layout ?? {})).toEqual([
        'planner',
        'writer',
        'designer',
        'checker',
        MERGE_NODE,
      ]);
      // Taken or missing: nothing changes.
      expect(renameRole(t, 'qa', 'writer')).toBe(t);
      expect(renameRole(t, 'nobody', 'x')).toBe(t);
      expect(roleIdProblem(t, 'writer', 'qa')).toContain('Another role');
      expect(roleIdProblem(t, 'Bad Id', 'qa')).toContain('lower case');
      expect(roleIdProblem(t, 'qa', 'qa')).toBe('');
    });

    it('changes a field and removes one set to undefined', () => {
      const next = updateRole(template(), 'writer', { effort: 'high', model: undefined });
      expect(next.roles[1]).toEqual({ id: 'writer', title: 'Writer', effort: 'high' });
      expect(updateRole(template(), 'nobody', { title: 'x' }).roles).toEqual(template().roles);
    });

    it('tells a role id apart, and a name as an id', () => {
      expect(isRoleId('front-end2')).toBe(true);
      for (const bad of ['', '-a', 'a-', 'A', 'a b', 'a@b', 'x'.repeat(33)]) {
        expect(isRoleId(bad), bad).toBe(false);
      }
      expect(slugId('Weekly  Newsletter!')).toBe('weekly-newsletter');
      expect(slugId('Đội viết bài')).toBe('doi-viet-bai');
    });
  });

  describe('flow', () => {
    it('connects two roles once, never a role to itself', () => {
      const t = template();
      expect(canConnect(t, 'writer', 'qa')).toBe(false);
      expect(canConnect(t, 'writer', 'writer')).toBe(false);
      expect(canConnect(t, 'qa', 'planner')).toBe(true);
      const next = connect(t, 'qa', 'planner');
      expect(next.flow?.at(-1)).toEqual({ from: 'qa', to: 'planner' });
      expect(connect(next, 'qa', 'planner').flow?.length).toBe(3);
      expect(connect(t, 'writer', 'nobody')).toBe(t);
    });

    it('connecting a role to Merge makes it the one who merges', () => {
      const next = connect(template(), 'writer', MERGE_NODE);
      expect(next.integration).toEqual({ branch: 'main', mergeBy: 'writer', strategy: 'squash' });
      expect(canConnect(template(), 'writer', MERGE_NODE)).toBe(true);
      expect(setMergeBy(next, undefined).integration).toEqual({
        branch: 'main',
        strategy: 'squash',
      });
      const bare: TeamTemplate = { id: 'b', name: 'B', roles: [] };
      expect(setMergeBy(bare, undefined)).toBe(bare);
      expect(updateIntegration(bare, { gate: 'npm test' }).integration).toEqual({
        gate: 'npm test',
      });
    });

    it('disconnects a handoff and notes what passes along', () => {
      const t = template();
      expect(disconnect(t, 'writer', 'qa').flow).toEqual([{ from: 'designer', to: 'qa' }]);
      expect(disconnect(t, 'qa', 'writer')).toBe(t);
      const noted = setHandoffNote(t, 'designer', 'qa', 'the frames');
      expect(noted.flow?.[1]).toEqual({ from: 'designer', to: 'qa', note: 'the frames' });
      expect(setHandoffNote(noted, 'writer', 'qa', '  ').flow?.[0]).toEqual({
        from: 'writer',
        to: 'qa',
        weight: 2,
      });
    });

    it('sums the flow up as chains of titles', () => {
      const t = template();
      expect(flowSummary(t)).toBe('Writer → QA · Designer → QA');
      t.flow = [
        { from: 'writer', to: 'designer' },
        { from: 'designer', to: 'qa' },
        { from: 'qa', to: 'writer' },
      ];
      expect(flowSummary(t)).toBe('Writer → Designer → QA → Writer');
      t.flow = [{ from: 'writer', to: 'ghost' }];
      expect(flowSummary(t)).toBe('');
      expect(flowSummary({ id: 'x', name: 'X', roles: [] })).toBe('');
    });
  });

  describe('problems', () => {
    it('finds at once what the server would refuse in the structure', () => {
      const t = template();
      t.roles.push({ id: 'writer', title: '' }, { id: 'Bad', title: 'B', lead: true });
      t.roles[0].machine = 'os:macos';
      t.flow = [
        { from: 'writer', to: 'ghost' },
        { from: 'qa', to: 'qa' },
        { from: 'writer', to: 'qa' },
        { from: 'writer', to: 'qa', note: 'x'.repeat(201) },
        'nonsense' as unknown as { from: string; to: string },
      ];
      t.integration = { mergeBy: 'nobody' };
      const paths = localIssues(t).map((i) => i.path);
      expect(paths).toEqual([
        'roles',
        'roles[0].machine',
        'roles[4].id',
        'roles[4].title',
        'roles[5].id',
        'flow[0].to',
        'flow[1]',
        'flow[3]',
        'flow[3].note',
        'flow[4]',
        'integration.mergeBy',
      ]);
      expect(localIssues(template())).toEqual([]);
      expect(localIssues({ id: 'X y', name: ' ', roles: [] }).map((i) => i.path)).toEqual([
        'id',
        'name',
        'roles',
      ]);
    });

    it('says which node or edge a problem is about', () => {
      const t = template();
      expect(issueTarget(t, 'roles[1].paths[0]')).toEqual({
        kind: 'role',
        id: 'writer',
        field: 'paths',
      });
      expect(issueTarget(t, 'roles[3]')).toEqual({ kind: 'role', id: 'qa' });
      expect(issueTarget(t, 'roles[9].id')).toEqual({ kind: 'template', field: 'roles' });
      expect(issueTarget(t, 'flow[1].to')).toEqual({
        kind: 'handoff',
        from: 'designer',
        to: 'qa',
        field: 'to',
      });
      expect(issueTarget(t, 'flow[0]')).toEqual({ kind: 'handoff', from: 'writer', to: 'qa' });
      expect(issueTarget(t, 'flow[5]')).toEqual({ kind: 'template', field: 'flow' });
      expect(issueTarget(t, 'integration.mergeBy')).toEqual({ kind: 'merge', field: 'mergeBy' });
      expect(issueTarget(t, 'integration')).toEqual({ kind: 'merge' });
      expect(issueTarget(t, 'idleStopMinutes')).toEqual({
        kind: 'merge',
        field: 'idleStopMinutes',
      });
      expect(issueTarget(t, 'roles')).toEqual({ kind: 'template', field: 'roles' });
      expect(issueTarget(t, '')).toEqual({ kind: 'template' });
      expect(selectionOf(issueTarget(t, 'flow[1].to'))).toEqual({
        kind: 'handoff',
        from: 'designer',
        to: 'qa',
      });
      expect(selectionOf({ kind: 'template' })).toBeNull();
    });
  });

  it('keeps every field it does not know through a round of edits', () => {
    let t = template();
    t = moveNode(t, 'writer', { x: 500, y: 0 });
    t = connect(t, 'planner', 'writer');
    t = renameRole(t, 'designer', 'artist');
    t = updateRole(t, 'planner', { effort: 'max' });
    t = setHandoffNote(t, 'writer', 'qa', 'the final script');
    const back = JSON.parse(JSON.stringify(t)) as TeamTemplate;
    expect(back['extra']).toEqual([1, 2]);
    expect(back.roles[0]['future']).toEqual({ a: 1 });
    expect(back.integration?.['strategy']).toBe('squash');
    expect(back.flow?.[0]).toEqual({
      from: 'writer',
      to: 'qa',
      note: 'the final script',
      weight: 2,
    });
    expect(back.flow?.[1]).toEqual({ from: 'artist', to: 'qa' });
    expect(back.idleStopMinutes).toBe(20);
  });

  it('never throws on a template typed as anything', () => {
    const odd = {
      id: 'odd',
      name: 'Odd',
      roles: [null, 7, { id: 'a', title: 'A', lead: true }, { title: 'no id' }],
      flow: 'not a list',
      layout: [1, 2],
      integration: 'nope',
    } as unknown as TeamTemplate;
    expect(() => {
      resolvedLayout(odd);
      localIssues(odd);
      flowSummary(odd);
      moveNode(odd, 'a', { x: 1, y: 1 });
      connect(odd, 'a', MERGE_NODE);
      removeRole(odd, 'a');
      renameRole(odd, 'a', 'b');
      arrange(odd);
    }).not.toThrow();
    expect(Object.keys(resolvedLayout(odd))).toEqual(['a', MERGE_NODE]);
    expect(connect(odd, 'a', MERGE_NODE).integration).toEqual({ mergeBy: 'a' });
  });
});
