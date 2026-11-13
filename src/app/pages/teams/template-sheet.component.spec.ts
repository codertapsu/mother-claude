import { ComponentFixture, TestBed } from '@angular/core/testing';
import { provideRouter } from '@angular/router';

import { FakeBackend, flush, useFakeServer } from '../../core/mock/fake-server.testing';
import { setActiveMock } from '../../core/mock/mock-api';
import { MERGE_NODE, TeamTemplate, TemplateEntry } from '../../core/teams';
import { TemplateEditor } from './template-editor';
import { GRID } from './template-graph';
import { TemplateSheetComponent, TemplateSheetMode } from './template-sheet.component';

const ok = { errors: [], warnings: [] };

/** Planner (lead), Writer → QA; fields this version does not know on the
 * template, a role, the integration and a handoff. */
const entry = (over: Partial<TemplateEntry> = {}): TemplateEntry => ({
  id: 'video',
  name: 'Video',
  builtin: false,
  report: ok,
  roles: [
    {
      id: 'planner',
      title: 'Planner',
      lead: true,
      model: 'opus',
      permissionMode: 'default',
      future: 1,
    },
    { id: 'writer', title: 'Writer', model: 'sonnet', permissionMode: 'acceptEdits' },
    { id: 'qa', title: 'QA', permissionMode: 'plan', readOnly: true },
  ],
  flow: [{ from: 'writer', to: 'qa', note: 'the script', weight: 2 }],
  integration: { branch: 'main', review: true, strategy: 'squash' },
  custom: { keep: true },
  ...over,
});

describe('TemplateSheetComponent', () => {
  let backend: FakeBackend;
  let fixture: ComponentFixture<TemplateSheetComponent>;

  beforeEach(() => {
    backend = new FakeBackend();
    useFakeServer(backend, [provideRouter([])]);
    backend.routes.set('/machines', () => ({ status: 200, body: { peers: [] } }));
    localStorage.removeItem('mc_template_hint');
  });

  afterEach(() => {
    setActiveMock(null);
    vi.restoreAllMocks();
  });

  const render = async (e: TemplateEntry | null, mode: TemplateSheetMode = 'edit') => {
    fixture = TestBed.createComponent(TemplateSheetComponent);
    fixture.componentRef.setInput('entry', e);
    fixture.componentRef.setInput('mode', mode);
    fixture.componentRef.setInput('open', true);
    fixture.detectChanges();
    await flush();
    fixture.detectChanges();
    return fixture.nativeElement as HTMLElement;
  };
  const settle = async () => {
    fixture.detectChanges();
    await flush();
    fixture.detectChanges();
  };
  const ed = () => fixture.debugElement.injector.get(TemplateEditor);
  const text = (e: Element | null | undefined) =>
    (e?.textContent ?? '').replace(/\s+/g, ' ').trim();
  const el = () => fixture.nativeElement as HTMLElement;
  const node = (key: string) =>
    el().querySelector(`mc-template-node[data-node="${key}"]`) as HTMLElement;
  const keys = () =>
    [...el().querySelectorAll('mc-template-node')].map((n) => n.getAttribute('data-node'));
  const button = (label: string) =>
    [...el().querySelectorAll('button')].find((b) => text(b) === label) as HTMLButtonElement;
  const viewport = () => el().querySelector('.viewport') as HTMLElement;
  const centre = (e: Element) => {
    const r = e.getBoundingClientRect();
    return { x: r.left + r.width / 2, y: r.top + r.height / 2 };
  };
  const pointer = (target: Element, type: string, at: { x: number; y: number }) =>
    target.dispatchEvent(
      new PointerEvent(type, {
        bubbles: true,
        cancelable: true,
        clientX: at.x,
        clientY: at.y,
        pointerId: 7,
        pointerType: 'mouse',
        button: 0,
        isPrimary: true,
      }),
    );
  const type = async (input: HTMLInputElement | HTMLTextAreaElement, value: string) => {
    input.value = value;
    input.dispatchEvent(new Event('input', { bubbles: true }));
    await settle();
  };

  it('draws a node per role and Merge, and shows the one selected in the panel', async () => {
    await render(entry());
    expect(keys()).toEqual(['planner', 'writer', 'qa', MERGE_NODE]);
    expect(text(node('planner'))).toContain('Lead');
    expect(text(node('qa'))).toContain('read-only');
    expect(node('writer').getAttribute('aria-label')).toBe('Writer (writer)');
    // Lead → each role, the handoff, and the lead merging by default.
    const edges = [...el().querySelectorAll('[data-edge]')].map((e) => e.getAttribute('data-edge'));
    expect(edges).toContain('h:writer>qa');
    expect(edges).toContain('m:planner');
    expect(el().querySelectorAll('g.edge.assign').length).toBe(2);
    expect(text(el().querySelector('.label'))).toBe('the script');
    // Nothing selected: the template's own fields.
    const name = el().querySelector('input[name="template-name"]') as HTMLInputElement;
    expect(name.value).toBe('Video');

    pointer(node('writer'), 'pointerdown', centre(node('writer')));
    pointer(viewport(), 'pointerup', centre(node('writer')));
    await settle();
    expect(node('writer').getAttribute('aria-pressed')).toBe('true');
    const title = el().querySelector('input[name="role-title"]') as HTMLInputElement;
    expect(title.value).toBe('Writer');
    await type(title, 'Scriptwriter');
    expect(text(node('writer'))).toContain('Scriptwriter');
    expect(ed().template().roles[1].title).toBe('Scriptwriter');

    // A click on empty canvas selects nothing again.
    const empty = viewport().getBoundingClientRect();
    pointer(viewport(), 'pointerdown', { x: empty.right - 5, y: empty.bottom - 5 });
    pointer(viewport(), 'pointerup', { x: empty.right - 5, y: empty.bottom - 5 });
    await settle();
    expect(ed().selection()).toBeNull();
  });

  it('adds a role at a free spot, selected, and writes it into the JSON', async () => {
    await render(entry());
    button('Add role').click();
    await settle();
    expect(keys()).toEqual(['planner', 'writer', 'qa', 'role-4', MERGE_NODE]);
    expect(node('role-4').getAttribute('aria-pressed')).toBe('true');
    const title = el().querySelector('input[name="role-title"]') as HTMLInputElement;
    expect(title.value).toBe('Role 4');
    const t = ed().template();
    const placed = t.layout?.['role-4'];
    expect(placed && placed.x % GRID === 0 && placed.y % GRID === 0).toBe(true);
    for (const k of ['planner', 'writer', 'qa', MERGE_NODE]) {
      const p = t.layout?.[k];
      const apart =
        !p ||
        !placed ||
        placed.x + 184 <= p.x ||
        p.x + 184 <= placed.x ||
        placed.y + 104 <= p.y ||
        p.y + 104 <= placed.y;
      expect(apart, k).toBe(true);
    }
    button('JSON').click();
    await settle();
    const json = el().querySelector('textarea[name="json"]') as HTMLTextAreaElement;
    expect(JSON.parse(json.value).roles[3]).toEqual({
      id: 'role-4',
      title: 'Role 4',
      model: 'sonnet',
      permissionMode: 'default',
    });
  });

  it('connects roles with the panel checkboxes, with a note, and disconnects them', async () => {
    await render(entry());
    ed().select({ kind: 'role', id: 'planner' });
    await settle();
    const next = el().querySelector('input[name="next-writer"]') as HTMLInputElement;
    expect(next.checked).toBe(false);
    next.click();
    await settle();
    expect(ed().template().flow?.at(-1)).toEqual({ from: 'planner', to: 'writer' });
    expect(el().querySelector('[data-edge="h:planner>writer"]')).not.toBeNull();
    // The lead's own "assigns tasks" line to the writer gives way to it.
    expect(el().querySelectorAll('g.edge.assign').length).toBe(1);
    const note = el().querySelector('input[name="note-writer"]') as HTMLInputElement;
    await type(note, 'the outline');
    expect(ed().template().flow?.at(-1)).toEqual({
      from: 'planner',
      to: 'writer',
      note: 'the outline',
    });
    (el().querySelector('input[name="next-writer"]') as HTMLInputElement).click();
    await settle();
    expect(ed().template().flow).toEqual([
      { from: 'writer', to: 'qa', note: 'the script', weight: 2 },
    ]);
    // The role stays selected all along.
    expect(ed().selection()).toEqual({ kind: 'role', id: 'planner' });
  });

  it('draws a handoff from a node’s dot, and who merges by dragging to Merge', async () => {
    await render(entry());
    const from = node('qa').querySelector('[data-handle]') as HTMLElement;
    pointer(from, 'pointerdown', centre(from));
    pointer(viewport(), 'pointermove', centre(node('writer')));
    await settle();
    expect(node('writer').classList).toContain('link-over');
    expect(el().querySelector('g.edge.temp')).not.toBeNull();
    pointer(viewport(), 'pointerup', centre(node('writer')));
    await settle();
    expect(ed().template().flow?.at(-1)).toEqual({ from: 'qa', to: 'writer' });
    // The new handoff is selected, to note what passes along.
    expect(ed().selection()).toEqual({ kind: 'handoff', from: 'qa', to: 'writer' });
    expect(el().querySelector('input[name="handoff-note"]')).not.toBeNull();
    expect(el().querySelector('g.edge.temp')).toBeNull();

    const dot = node('writer').querySelector('[data-handle]') as HTMLElement;
    pointer(dot, 'pointerdown', centre(dot));
    pointer(viewport(), 'pointermove', centre(node(MERGE_NODE)));
    pointer(viewport(), 'pointerup', centre(node(MERGE_NODE)));
    await settle();
    expect(ed().template().integration).toEqual({
      branch: 'main',
      review: true,
      strategy: 'squash',
      mergeBy: 'writer',
    });
    expect(el().querySelector('[data-edge="m:writer"]')).not.toBeNull();
    // Dropped on nothing: nothing changes.
    const before = ed().template();
    pointer(dot, 'pointerdown', centre(dot));
    pointer(viewport(), 'pointermove', { x: centre(dot).x + 3, y: centre(dot).y + 400 });
    pointer(viewport(), 'pointerup', { x: centre(dot).x + 3, y: centre(dot).y + 400 });
    await settle();
    expect(ed().template()).toBe(before);
  });

  it('moves a node by dragging and with the arrow keys, on the grid', async () => {
    await render(entry());
    const start = ed().positions()['writer'];
    const at = centre(node('writer'));
    pointer(node('writer'), 'pointerdown', at);
    pointer(viewport(), 'pointermove', { x: at.x + 70, y: at.y + 45 });
    await settle();
    expect(node('writer').classList).toContain('dragging');
    pointer(viewport(), 'pointerup', { x: at.x + 70, y: at.y + 45 });
    await settle();
    const moved = ed().template().layout?.['writer'];
    expect(moved?.x).toBeGreaterThan(start.x);
    expect(moved?.y).toBeGreaterThan(start.y);
    expect((moved?.x ?? 1) % GRID).toBe(0);
    expect((moved?.y ?? 1) % GRID).toBe(0);
    // Every node's place is stored once one moved.
    expect(Object.keys(ed().template().layout ?? {}).sort()).toEqual(
      [MERGE_NODE, 'planner', 'qa', 'writer'].sort(),
    );

    node('writer').focus();
    node('writer').dispatchEvent(
      new KeyboardEvent('keydown', { key: 'ArrowRight', bubbles: true, cancelable: true }),
    );
    node('writer').dispatchEvent(
      new KeyboardEvent('keydown', {
        key: 'ArrowDown',
        shiftKey: true,
        bubbles: true,
        cancelable: true,
      }),
    );
    await settle();
    expect(ed().template().layout?.['writer']).toEqual({
      x: (moved?.x ?? 0) + GRID,
      y: (moved?.y ?? 0) + GRID * 4,
    });
  });

  it('keeps the JSON and the canvas in step, and a mistake keeps the last good template', async () => {
    await render(entry());
    button('JSON').click();
    await settle();
    const json = el().querySelector('textarea[name="json"]') as HTMLTextAreaElement;
    const t = JSON.parse(json.value) as TeamTemplate;
    expect(t['custom']).toEqual({ keep: true });
    expect('builtin' in t || 'report' in t).toBe(false);
    t.roles[2].title = 'Checker';
    await type(json, JSON.stringify(t, null, 2));
    button('Visual').click();
    await settle();
    expect(text(node('qa'))).toContain('Checker');

    button('JSON').click();
    await settle();
    const area = el().querySelector('textarea[name="json"]') as HTMLTextAreaElement;
    await type(area, '{ "id": "video", "roles": [');
    expect(text(el().querySelector('.notice.error'))).toContain('Not JSON');
    button('Visual').click();
    await settle();
    // The last good template, not editable until the JSON is fixed or dropped.
    expect(text(node('qa'))).toContain('Checker');
    expect(text(el())).toContain('The JSON has a mistake');
    expect(button('Add role')).toBeUndefined();
    expect(el().querySelector('[data-handle]')).toBeNull();
    button('Use this version').click();
    await settle();
    expect(button('Add role')).toBeDefined();
    expect(JSON.parse(ed().text()).roles[2].title).toBe('Checker');
  });

  it('shows a built-in template read-only, with Duplicate to edit', async () => {
    backend.routes.set('/team-templates/video/duplicate', () => ({
      status: 201,
      body: { ...entry(), id: 'video-copy' },
    }));
    await render(entry({ builtin: true }), 'view');
    expect(keys()).toEqual(['planner', 'writer', 'qa', MERGE_NODE]);
    expect(button('Add role')).toBeUndefined();
    expect(button('Auto-arrange')).toBeUndefined();
    expect(el().querySelector('[data-handle]')).toBeNull();
    expect(text(el())).toContain('A built-in template cannot be changed');
    // Still selectable, nothing editable.
    pointer(node('writer'), 'pointerdown', centre(node('writer')));
    pointer(viewport(), 'pointerup', centre(node('writer')));
    await settle();
    expect((el().querySelector('input[name="role-title"]') as HTMLInputElement).readOnly).toBe(
      true,
    );
    expect((el().querySelector('select[name="role-mode"]') as HTMLSelectElement).disabled).toBe(
      true,
    );
    expect(button('Delete')).toBeUndefined();
    node('writer').dispatchEvent(
      new KeyboardEvent('keydown', { key: 'ArrowRight', bubbles: true, cancelable: true }),
    );
    await settle();
    expect(ed().template().layout).toBeUndefined();
    button('JSON').click();
    await settle();
    expect((el().querySelector('textarea[name="json"]') as HTMLTextAreaElement).readOnly).toBe(
      true,
    );
    let duplicated = false;
    fixture.componentInstance.duplicated.subscribe(() => (duplicated = true));
    button('Duplicate to edit').click();
    await flush();
    expect(backend.calls).toContain('POST /team-templates/video/duplicate');
    expect(duplicated).toBe(true);
  });

  it("shows the server's problems on their nodes and edges, and saves every field", async () => {
    backend.routes.set('/team-templates/validate', (_m, body) => {
      const t = body as TeamTemplate;
      const bad = t.roles[1].paths?.[0] === '/etc';
      return {
        status: 200,
        body: bad
          ? {
              ok: false,
              errors: [
                { path: 'roles[1].paths[0]', message: 'A path glob is relative to the project.' },
                { path: 'flow[0].note', message: 'Too long.' },
              ],
              warnings: [{ path: 'integration.gate', message: 'No gate.' }],
            }
          : { ok: true, errors: [], warnings: [] },
      };
    });
    backend.routes.set('/team-templates/video', (method, body) => ({
      status: method === 'PUT' ? 200 : 404,
      body,
    }));
    await render(entry());
    ed().select({ kind: 'role', id: 'writer' });
    await settle();
    const paths = el().querySelector('textarea[name="role-paths"]') as HTMLTextAreaElement;
    await type(paths, '/etc\n');
    // The line being written stays.
    expect(paths.value).toBe('/etc\n');
    button('Check').click();
    await settle();
    expect(text(node('writer').querySelector('.count'))).toBe('1');
    expect(text(node(MERGE_NODE).querySelector('.count'))).toBe('1');
    expect(el().querySelector('g.edge.handoff.bad')).not.toBeNull();
    const issues = [...el().querySelectorAll('.issue')].map((i) => text(i));
    expect(issues).toEqual([
      'Writer: A path glob is relative to the project.',
      'Writer → QA: Too long.',
      'Merge: No gate.',
    ]);
    // A problem leads to what it is about.
    (el().querySelectorAll('.issue')[1] as HTMLButtonElement).click();
    await settle();
    expect(ed().selection()).toEqual({ kind: 'handoff', from: 'writer', to: 'qa' });

    ed().select({ kind: 'role', id: 'writer' });
    await settle();
    await type(el().querySelector('textarea[name="role-paths"]') as HTMLTextAreaElement, 'src/**');
    // Changed since: the old problems are out of date.
    expect(text(el().querySelector('.issue'))).toContain('(at the last check)');
    button('Save').click();
    await settle();
    const saved = backend.bodyOf('PUT /team-templates/video') as TeamTemplate;
    expect(saved['custom']).toEqual({ keep: true });
    expect(saved.roles[0]['future']).toBe(1);
    expect(saved.roles[1].paths).toEqual(['src/**']);
    expect(saved.flow).toEqual([{ from: 'writer', to: 'qa', note: 'the script', weight: 2 }]);
    expect(saved.integration).toEqual({ branch: 'main', review: true, strategy: 'squash' });
    expect(saved.layout).toBeUndefined();
  });

  it('deletes a role once confirmed, with its handoffs; renames one with its references', async () => {
    const ask = vi.spyOn(window, 'confirm').mockReturnValue(true);
    await render(entry({ integration: { mergeBy: 'qa' } }));
    ed().select({ kind: 'role', id: 'writer' });
    await settle();
    const id = el().querySelector('input[name="role-id"]') as HTMLInputElement;
    await type(id, 'Bad id');
    expect(text(el().querySelector('.problem'))).toContain('lower case');
    await type(id, 'author');
    id.dispatchEvent(new Event('change', { bubbles: true }));
    await settle();
    expect(keys()).toEqual(['planner', 'author', 'qa', MERGE_NODE]);
    expect(ed().template().flow?.[0].from).toBe('author');
    expect(ed().selection()).toEqual({ kind: 'role', id: 'author' });

    // An id typed, then another node clicked: the field is left after the
    // click selected that node, and the role it was typed for is renamed.
    const again = el().querySelector('input[name="role-id"]') as HTMLInputElement;
    await type(again, 'scribe');
    pointer(node('planner'), 'pointerdown', centre(node('planner')));
    pointer(viewport(), 'pointerup', centre(node('planner')));
    again.dispatchEvent(new Event('change', { bubbles: true }));
    await settle();
    expect(keys()).toEqual(['planner', 'scribe', 'qa', MERGE_NODE]);
    expect(ed().selection()).toEqual({ kind: 'role', id: 'planner' });

    ed().select({ kind: 'role', id: 'qa' });
    await settle();
    button('Delete').click();
    await settle();
    expect(ask).toHaveBeenCalled();
    expect(keys()).toEqual(['planner', 'scribe', MERGE_NODE]);
    expect(ed().template().flow).toEqual([]);
    expect(ed().template().integration).toEqual({});
    // The lead merges again, drawn as the default.
    expect(el().querySelector('[data-edge="m:planner"]')?.closest('g')?.classList).toContain(
      'dflt',
    );
    // The only lead can't be deleted.
    ed().select({ kind: 'role', id: 'planner' });
    await settle();
    expect(button('Delete').disabled).toBe(true);
  });

  it('starts a new template from a skeleton whose id follows its name', async () => {
    backend.routes.set('/team-templates/validate', () => ({
      status: 200,
      body: { ok: true, errors: [], warnings: [] },
    }));
    backend.routes.set('/team-templates', (method, body) => ({ status: 201, body }));
    await render(null, 'new');
    expect(keys()).toEqual(['lead', 'dev', MERGE_NODE]);
    expect(text(el())).toContain('Drag a role to move it.');
    const name = el().querySelector('input[name="template-name"]') as HTMLInputElement;
    await type(name, 'Weekly newsletter');
    expect(ed().template().id).toBe('weekly-newsletter');
    button('Check').click();
    await settle();
    expect(text(el())).toContain('The template is valid.');
    button('Save').click();
    await settle();
    expect((backend.bodyOf('POST /team-templates') as TeamTemplate).id).toBe('weekly-newsletter');
    // The tip goes for good once hidden.
    (el().querySelector('.hint button') as HTMLButtonElement).click();
    await settle();
    expect(el().querySelector('.hint')).toBeNull();
    expect(localStorage.getItem('mc_template_hint')).toBe('1');
  });
});
