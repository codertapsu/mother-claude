import { ComponentFixture, TestBed } from '@angular/core/testing';
import { provideRouter } from '@angular/router';

import { DataJobsService } from '../core/data-jobs.service';
import {
  FakeBackend,
  asDesktop,
  flush,
  serveAccounts,
  useFakeServer,
} from '../core/mock/fake-server.testing';
import { MOCK_ACCOUNT_WORK, buildAccounts } from '../core/mock/fixtures';
import { setActiveMock } from '../core/mock/mock-api';
import { AccountRef, JobView, TransferWhat } from '../core/models';
import { MoveCopySheetComponent } from './move-copy-sheet.component';

const DEFAULT_REF: AccountRef = {
  id: 'default',
  name: 'Default',
  color: '#8b5cf6',
  kind: 'directory',
};
const WORK_REF: AccountRef = {
  id: MOCK_ACCOUNT_WORK,
  name: 'Work',
  color: '#0ea5e9',
  kind: 'directory',
};

const plan = (over: Record<string, unknown> = {}) => ({
  what: { what: 'session', id: 's1' },
  mode: 'copy',
  sessions: [
    {
      id: 's1',
      cwd: '/Users/me/dev/api',
      files: [
        {
          from: '/Users/me/.claude/projects/x/s1.jsonl',
          to: '/Users/me/.claude-work/projects/x/s1.jsonl',
          size: 2048,
        },
        { from: '/a', to: '/Users/me/.claude-work/file-history/s1/1', size: 1024 },
      ],
    },
  ],
  files: 2,
  bytes: 3072,
  addsProjectEntry: null,
  removesProjectEntry: null,
  historyLines: 0,
  writesShared: false,
  ...over,
});

const job = (over: Partial<JobView> = {}): JobView => ({
  id: 'job1',
  kind: 'copy',
  state: 'running',
  startedAt: Date.now(),
  filesDone: 0,
  filesTotal: 2,
  bytesDone: 0,
  bytesTotal: 3072,
  sessions: ['s1'],
  ...over,
});

describe('MoveCopySheetComponent', () => {
  let backend: FakeBackend;
  let fixture: ComponentFixture<MoveCopySheetComponent>;

  beforeEach(() => {
    backend = new FakeBackend();
    useFakeServer(backend, [provideRouter([])]);
    serveAccounts(backend, buildAccounts(Date.now(), true));
    asDesktop();
    TestBed.inject(DataJobsService).pollMs = 1;
  });

  afterEach(() => setActiveMock(null));

  const open = async (
    what: TransferWhat = { what: 'session', id: 's1' },
    source: AccountRef = DEFAULT_REF,
    live = false,
  ) => {
    fixture = TestBed.createComponent(MoveCopySheetComponent);
    fixture.componentRef.setInput('what', what);
    fixture.componentRef.setInput('source', source);
    fixture.componentRef.setInput('live', live);
    fixture.componentRef.setInput('open', true);
    fixture.detectChanges();
    await flush();
    fixture.detectChanges();
    return fixture.nativeElement as HTMLElement;
  };
  const settle = async (times = 3) => {
    for (let i = 0; i < times; i++) {
      await flush();
      fixture.detectChanges();
    }
  };
  const text = (el: Element | null | undefined) =>
    (el?.textContent ?? '').replace(/\s+/g, ' ').trim();
  const button = (el: HTMLElement, label: string) =>
    [...el.querySelectorAll<HTMLButtonElement>('button')].find((b) => text(b).startsWith(label));
  const targets = (el: HTMLElement) =>
    [...el.querySelectorAll('[role="radiogroup"][aria-label="To"] .opt')].map((o) =>
      text(o.querySelector('.oname')),
    );

  it('offers only the accounts with another directory', async () => {
    let el = await open();
    // A token account shares the Default's directory: nothing to move there.
    expect(targets(el)).toEqual(['Work']);
    fixture.destroy();
    el = await open({ what: 'session', id: 's2' }, WORK_REF);
    expect(targets(el)).toEqual(['Default']);
    expect(text(el.querySelector('[role="radiogroup"] .opt'))).toContain(
      'Shared with Marcus (Pro)',
    );
  });

  it('checks first, then copies with progress, and undoes on a second tap', async () => {
    let polls = 0;
    backend.routes.set('/data/move', (_m, body) =>
      (body as { dryRun?: boolean }).dryRun
        ? { status: 200, body: { plan: plan() } }
        : { status: 202, body: { job: job() } },
    );
    backend.routes.set('/data/jobs/job1', () => ({
      status: 200,
      body:
        ++polls < 2
          ? job({ bytesDone: 1024, filesDone: 1 })
          : job({
              state: 'done',
              bytesDone: 3072,
              filesDone: 2,
              undoUntil: Date.now() + 7 * 86_400_000,
            }),
    }));
    backend.routes.set('/data/jobs/job1/undo', () => ({
      status: 200,
      body: job({ state: 'undone', finishedAt: Date.now() }),
    }));
    const el = await open();

    button(el, 'Check what it does')?.click();
    await settle();
    expect(backend.bodyOf('POST /data/move')).toEqual({
      what: 'session',
      id: 's1',
      from: 'default',
      to: MOCK_ACCOUNT_WORK,
      mode: 'copy',
      trust: false,
      dryRun: true,
    });
    expect(text(el.querySelector('.planline'))).toBe(
      "Copies 1 conversation (2 files, 3.0 KB) into Work's directory.",
    );
    expect(text(el.querySelector('.psess'))).toContain('2 files · 3.0 KB');
    expect(text(el.querySelector('.effects'))).toContain('can be undone for seven days');

    button(el, 'Copy 1 conversation')?.click();
    await settle();
    expect((backend.bodyOf('POST /data/move') as { dryRun: boolean }).dryRun).toBeFalse();
    await new Promise((r) => setTimeout(r, 20));
    await settle();
    const row = el.querySelector('mc-data-job');
    expect(row?.getAttribute('data-state')).toBe('done');
    expect(text(row)).toContain('Copy to Work');
    expect(text(row)).toContain('Copied 1 session.');

    button(el, 'Undo')?.click();
    fixture.detectChanges();
    expect(backend.calls).not.toContain('POST /data/jobs/job1/undo');
    button(el, 'Tap again to undo')?.click();
    await settle();
    expect(backend.calls).toContain('POST /data/jobs/job1/undo');
    expect(el.querySelector('mc-data-job')?.getAttribute('data-state')).toBe('undone');
    expect(button(el, 'Undo')).toBeUndefined();
  });

  it("moves a project with the folder's trust when ticked, saying what that writes", async () => {
    backend.routes.set('/data/move', () => ({
      status: 200,
      body: {
        plan: plan({
          what: { what: 'project', cwd: '/Users/me/dev/api' },
          mode: 'move',
          addsProjectEntry: '/Users/me/dev/api',
          removesProjectEntry: '/Users/me/dev/api',
          historyLines: 7,
          writesShared: true,
        }),
      },
    }));
    const el = await open({ what: 'project', cwd: '/Users/me/dev/api' });
    expect(text(el)).toContain('Every conversation of ~/dev/api');
    button(el, 'Move')?.click();
    fixture.detectChanges();
    const trust = el.querySelector('.check input') as HTMLInputElement;
    expect(text(el.querySelector('.check'))).toContain('Also trust this folder for Work');
    expect(text(el.querySelector('.check'))).toContain(
      'A trusted folder runs its project hooks and MCP servers without asking.',
    );
    trust.checked = true;
    trust.dispatchEvent(new Event('change'));
    fixture.detectChanges();
    button(el, 'Check what it does')?.click();
    await settle();
    expect(backend.bodyOf('POST /data/move')).toEqual({
      what: 'project',
      cwd: '/Users/me/dev/api',
      from: 'default',
      to: MOCK_ACCOUNT_WORK,
      mode: 'move',
      trust: true,
      dryRun: true,
    });
    const effects = text(el.querySelector('.effects'));
    expect(effects).toContain("Adds the folder's entry, with its trust, to Work's settings.");
    expect(effects).toContain("Removes the folder's entry from Default's settings.");
    expect(effects).toContain('Adds 7 lines of prompt history to Work.');
    expect(effects).toContain('waits until no Claude Code session uses that directory');
    expect(text(el.querySelector('.planline'))).toContain("out of Default's");
  });

  it("shows the server's refusal and stays where it was", async () => {
    backend.routes.set('/data/move', () => ({
      status: 409,
      body: {
        error: {
          code: 'directory_in_use',
          message: 'a Claude Code process is using /Users/me/.claude-work; quit its sessions first',
        },
      },
    }));
    const el = await open();
    button(el, 'Check what it does')?.click();
    await settle();
    expect(text(el.querySelector('.notice.error'))).toBe(
      'A Claude Code process is using /Users/me/.claude-work; quit its sessions first.',
    );
    expect(button(el, 'Check what it does')).toBeDefined();
  });

  it('says a running session has to stop first', async () => {
    const el = await open({ what: 'session', id: 's1' }, DEFAULT_REF, true);
    expect(text(el.querySelector('.notice.warn'))).toContain('once it has stopped');
    expect(button(el, 'Check what it does')?.disabled).toBeTrue();
  });
});
