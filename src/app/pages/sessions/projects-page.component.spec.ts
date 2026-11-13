import { TestBed } from '@angular/core/testing';
import { provideRouter } from '@angular/router';

import {
  FakeBackend,
  flush,
  serveAccounts,
  useFakeServer,
} from '../../core/mock/fake-server.testing';
import { buildAccounts } from '../../core/mock/fixtures';
import { setActiveMock } from '../../core/mock/mock-api';
import { ProjectsPageComponent } from './projects-page.component';

const text = (e: Element | null | undefined) => (e?.textContent ?? '').replace(/\s+/g, ' ').trim();

describe('Sessions › Projects', () => {
  let backend: FakeBackend;

  beforeEach(() => {
    backend = new FakeBackend();
    useFakeServer(backend, [provideRouter([])]);
    serveAccounts(backend, buildAccounts(Date.now(), false));
    backend.routes.set('/data/jobs', () => ({ status: 200, body: { jobs: [] } }));
  });

  afterEach(() => setActiveMock(null));

  it('is a tab of Sessions: the head, its tabs and every project on disk', async () => {
    backend.routes.set('/data/projects', () => ({
      status: 200,
      body: {
        projects: [
          {
            account: 'default',
            cwd: '/Users/me/dev/api',
            name: 'api',
            folder: '/Users/me/.claude/projects/-Users-me-dev-api',
            sessions: 3,
            live: 0,
            bytes: 2048,
            lastActivityAt: Date.now() - 60_000,
          },
        ],
      },
    }));
    const fixture = TestBed.createComponent(ProjectsPageComponent);
    fixture.detectChanges();
    await flush();
    fixture.detectChanges();
    const el = fixture.nativeElement as HTMLElement;
    const head = el.querySelector('mc-sessions-head')!;
    expect(text(head.querySelector('h1'))).toBe('Sessions');
    expect([...head.querySelectorAll('nav.hub-tabs a')].map((a) => text(a))).toEqual([
      'Sessions',
      'Agents',
      'Projects',
    ]);
    // The head's one action is New session; the projects' Select is theirs.
    expect([...head.querySelectorAll('.page-head .btn')].map((b) => text(b))).toEqual([
      'New session',
    ]);
    expect(text(el.querySelector('mc-projects-view .pname'))).toBe('api');
    expect(text(el.querySelector('mc-projects-view .toolbar'))).toContain('Select');
    fixture.destroy();
  });
});
