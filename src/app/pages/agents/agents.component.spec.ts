import { TestBed } from '@angular/core/testing';
import { provideRouter } from '@angular/router';

import { FakeBackend, flush, useFakeServer } from '../../core/mock/fake-server.testing';
import { setActiveMock } from '../../core/mock/mock-api';
import { ObservedEvent } from '../../core/teams';
import { AgentsComponent } from './agents.component';

const text = (e: Element | null | undefined) => (e?.textContent ?? '').replace(/\s+/g, ' ').trim();

describe('Sessions › Agents › Teammates', () => {
  let backend: FakeBackend;

  beforeEach(() => {
    backend = new FakeBackend();
    useFakeServer(backend, [provideRouter([])]);
    backend.routes.set('/agents', () => ({ status: 200, body: { sessions: [] } }));
  });

  afterEach(() => setActiveMock(null));

  const render = async () => {
    const fixture = TestBed.createComponent(AgentsComponent);
    fixture.detectChanges();
    await flush();
    fixture.detectChanges();
    return fixture;
  };

  it("lists what Claude Code's own agent teams reported, under Agents", async () => {
    const events: ObservedEvent[] = [
      {
        at: Date.now() - 60_000,
        sessionId: 's1',
        event: 'TaskCreated',
        teammate: 'researcher',
        subject: 'Compare the three vendors',
      },
    ];
    backend.routes.set('/teams/events', () => ({ status: 200, body: { events } }));
    const fixture = await render();
    const el = fixture.nativeElement as HTMLElement;
    expect(text(el.querySelector('mc-sessions-head h1'))).toBe('Sessions');
    const section = el.querySelector('section.teammates')!;
    expect(text(section.querySelector('h2'))).toBe('Teammates');
    const row = section.querySelector('.event')!;
    expect(text(row.querySelector('.badge'))).toBe('TaskCreated');
    expect(text(row.querySelector('.grow'))).toBe('researcher · Compare the three vendors');
    fixture.destroy();
  });

  it('shows no Teammates when no agent team reported anything, or the server has none', async () => {
    backend.routes.set('/teams/events', () => ({ status: 404, body: {} }));
    const fixture = await render();
    const el = fixture.nativeElement as HTMLElement;
    expect(el.querySelector('section.teammates')).toBeNull();
    expect(text(el.querySelector('.empty-state'))).toContain('No agents running');
    fixture.destroy();
  });
});
