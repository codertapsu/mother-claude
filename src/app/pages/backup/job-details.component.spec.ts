import { TestBed } from '@angular/core/testing';

import { FolderPairDraftService } from '../../core/folder-pair-draft.service';
import { FakeBackend, useFakeServer } from '../../core/mock/fake-server.testing';
import { setActiveMock } from '../../core/mock/mock-api';
import { JobView, SessionSummary } from '../../core/models';
import { RealtimeService } from '../../core/realtime.service';
import { JobDetailsComponent } from './job-details.component';

const CONFLICTED = 'd5b3f4a6-7b8c-4d9e-0f1a-2b3c4d5e6f7a';
const NEWER = 'c4a2e3f5-6a7b-4c8d-9e0f-1a2b3c4d5e6f';

const backup = (result: unknown): JobView => ({
  id: 'jb',
  kind: 'backup',
  state: 'done',
  startedAt: 1,
  finishedAt: 2,
  filesDone: 0,
  filesTotal: 0,
  bytesDone: 0,
  bytesTotal: 0,
  sessions: [],
  account: 'default',
  result,
});

describe('JobDetailsComponent', () => {
  beforeEach(() => useFakeServer(new FakeBackend()));
  afterEach(() => setActiveMock(null));

  const render = (job: JobView) => {
    const fixture = TestBed.createComponent(JobDetailsComponent);
    fixture.componentRef.setInput('job', job);
    fixture.detectChanges();
    return fixture.nativeElement as HTMLElement;
  };
  const text = (el: Element | null | undefined) =>
    (el?.textContent ?? '').replace(/\s+/g, ' ').trim();

  it('lists what a backup did not push, what a Pull brings, and what it left out', () => {
    // The app knows one of the sessions by name.
    (
      TestBed.inject(RealtimeService) as unknown as { replaceStore(l: SessionSummary[]): void }
    ).replaceStore([{ id: CONFLICTED, title: 'Stripe webhook retries' } as SessionSummary]);
    const el = render(
      backup({
        archived: ['a'],
        unchanged: 4,
        skippedLive: [],
        failed: [['f1f1f1f1-0000', 'reading the transcript: permission denied']],
        conflicts: [CONFLICTED],
        newerInSet: [NEWER],
        warnings: ['agents/review.md could not be read and is not in the configuration bundle'],
        removed: 2,
        bytes: 1,
      }),
    );
    const heads = [...el.querySelectorAll('.h')].map((h) => text(h));
    expect(heads).toEqual([
      'Changed here and on another Mac, so not backed up — a Pull brings the other copy beside this one:',
      'Newer in the backup (another Mac backed them up) — a Pull brings them:',
      'Could not be backed up:',
    ]);
    const items = [...el.querySelectorAll('li')].map((li) => text(li));
    expect(items).toEqual([
      'd5b3f4a6 Stripe webhook retries',
      'c4a2e3f5',
      'f1f1f1f1 Reading the transcript: permission denied.',
      'agents/review.md could not be read and is not in the configuration bundle.',
    ]);
    expect(text(el.querySelector('.gone'))).toBe('Removed 2 old files the backup no longer names.');
  });

  it('says nothing more about a backup that found nothing to tell', () => {
    const el = render(backup({ archived: ['a'], unchanged: 4, skippedLive: [], failed: [] }));
    expect(text(el)).toBe('');
  });

  it("names what kept another machine's path, offers a pair, and what a pair moved", () => {
    const el = render({
      ...backup({
        restored: ['a1a1a1a1-0000'],
        updated: [],
        conflicts: [],
        skipped: [],
        warnings: [],
        historyLines: 0,
        relocated: [NEWER],
        mapped: [[NEWER, '/Users/marcus/dev/api-server']],
        foreign: [
          {
            folder: 'D:\\dev\\shop',
            machine: '7b9d1f3a5c7e9b1d3f5a7c9e1b3d5f7a',
            host: 'DESKTOP-PC',
            sessions: 1,
          },
        ],
      }),
      kind: 'pull',
    });
    const heads = [...el.querySelectorAll('.h')].map((h) => text(h));
    expect(heads[0]).toBe('Moved into the folder a folder pair maps them to here:');
    expect(heads[1]).toContain("Kept another machine's path — no folder pair maps these projects");
    expect(text(el.querySelector('.foreign li'))).toBe(
      'D:\\dev\\shop from DESKTOP-PC Pair a folder…',
    );
    (el.querySelector('.foreign .pair') as HTMLButtonElement).click();
    expect(TestBed.inject(FolderPairDraftService).draft()).toEqual({
      machine: '7b9d1f3a5c7e9b1d3f5a7c9e1b3d5f7a',
      otherFolder: 'D:\\dev',
    });
  });
});
