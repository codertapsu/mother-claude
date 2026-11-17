import { TestBed } from '@angular/core/testing';
import { provideRouter } from '@angular/router';

import { ApiClientRoot, ApiConversationRow } from '../../core/claude-api';
import { FakeBackend, asDesktop, flush, useFakeServer } from '../../core/mock/fake-server.testing';
import { setActiveMock } from '../../core/mock/mock-api';
import { ApiClientsComponent } from './api-clients.component';

const usage = {
  inputTokens: 10,
  outputTokens: 5,
  cacheReadInputTokens: 0,
  cacheCreationInputTokens: 0,
};

const conv = (id: string, over: Partial<ApiConversationRow> = {}): ApiConversationRow => ({
  id,
  title: `About ${id}`,
  model: 'claude-sonnet-5-5',
  createdAt: Date.now() - 120_000,
  updatedAt: Date.now() - 60_000,
  messages: 2,
  requests: 1,
  usage,
  seeded: false,
  server: 'main',
  account: 'default',
  busy: false,
  waitingForTools: false,
  live: false,
  ...over,
});

describe('ApiClientsComponent', () => {
  let backend: FakeBackend;
  let roots: ApiClientRoot[];
  let convs: ApiConversationRow[];

  beforeEach(() => {
    backend = new FakeBackend();
    useFakeServer(backend, [provideRouter([])]);
    convs = [conv('c1'), conv('c2'), conv('c3', { busy: true })];
    roots = [
      {
        path: '/data/claude-api',
        default: true,
        servers: [],
        clients: [
          {
            id: '192.168.1.20',
            kind: 'ip',
            label: '192.168.1.20',
            firstSeen: 1,
            lastSeen: Date.now() - 5_000,
            requests: 4,
            folder: '/data/claude-api/192.168.1.20',
            conversations: 3,
            active: 1,
            usage,
          },
        ],
      },
    ];
    backend.routes.set('/bridge/clients', () => ({ status: 200, body: { roots } }));
    backend.routes.set('/bridge/clients/192.168.1.20', (method) => {
      if (method === 'DELETE') {
        roots = [{ ...roots[0], clients: [] }];
        return { status: 200, body: { deleted: ['c1', 'c2'] } };
      }
      return {
        status: 200,
        body: { client: roots[0].clients[0], conversations: convs },
      };
    });
    backend.routes.set('/bridge/api-conversations/delete', (_m, body) => {
      const ids = (body as { ids: string[] }).ids;
      convs = convs.filter((c) => !ids.includes(c.id));
      return { status: 200, body: { deleted: ids, busy: [] } };
    });
  });

  afterEach(() => setActiveMock(null));

  const render = async () => {
    const fixture = TestBed.createComponent(ApiClientsComponent);
    fixture.detectChanges();
    await flush();
    fixture.detectChanges();
    return fixture;
  };
  const button = (el: HTMLElement, text: string) =>
    [...el.querySelectorAll('button')].find((b) => b.textContent?.trim().startsWith(text))!;

  it('lists clients and opens one to its conversations', async () => {
    const fixture = await render();
    const el = fixture.nativeElement as HTMLElement;
    expect(el.textContent).toContain('1 client · 3 conversations');
    (el.querySelector('button.client') as HTMLButtonElement).click();
    await flush();
    fixture.detectChanges();
    expect(backend.urls).toContain('/bridge/clients/192.168.1.20?root=%2Fdata%2Fclaude-api');
    expect(el.textContent).toContain('About c1');
    expect(el.textContent).toContain('answering');
    // A conversation answering a request cannot be picked.
    const boxes = el.querySelectorAll<HTMLInputElement>('.rows input[type="checkbox"]');
    expect(boxes[2].disabled).toBe(true);
  });

  it('deletes the picked conversations after asking, the transcripts only when asked', async () => {
    asDesktop();
    const fixture = await render();
    const el = fixture.nativeElement as HTMLElement;
    (el.querySelector('button.client') as HTMLButtonElement).click();
    await flush();
    fixture.detectChanges();
    const boxes = el.querySelectorAll<HTMLInputElement>('.rows input[type="checkbox"]');
    boxes[0].click();
    boxes[1].click();
    fixture.detectChanges();
    button(el, 'Delete picked (2)').click();
    fixture.detectChanges();
    expect(el.textContent).toContain('Delete 2 conversations?');
    button(el, 'Delete').click();
    await flush();
    fixture.detectChanges();
    expect(backend.bodyOf('POST /bridge/api-conversations/delete')).toEqual({
      ids: ['c1', 'c2'],
      transcripts: false,
    });
    expect(el.textContent).toContain('Deleted 2 conversations.');
  });

  it("deletes all of a client's data", async () => {
    asDesktop();
    const fixture = await render();
    const el = fixture.nativeElement as HTMLElement;
    (el.querySelector('button.client') as HTMLButtonElement).click();
    await flush();
    fixture.detectChanges();
    const transcripts = [...el.querySelectorAll<HTMLInputElement>('label.check input')].pop()!;
    transcripts.click();
    fixture.detectChanges();
    button(el, "Delete all of this client's data").click();
    fixture.detectChanges();
    expect(el.textContent).toContain('Their transcripts go too.');
    button(el, 'Delete').click();
    await flush();
    fixture.detectChanges();
    expect(backend.urls).toContain(
      '/bridge/clients/192.168.1.20?root=%2Fdata%2Fclaude-api&transcripts=true',
    );
    expect(el.textContent).toContain('No client has called the Claude API yet.');
  });
});
