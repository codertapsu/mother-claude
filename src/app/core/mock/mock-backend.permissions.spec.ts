import { PendingItem, TranscriptPage } from '../models';
import { PermissionsView } from '../permission-rules';
import { createMockBackend } from './mock-backend';

/** The dev mock as `?mock=1` serves it (the desktop): answers and their
 * permissions, end to end. */
describe('the mock backend: remembering answers', () => {
  const backend = createMockBackend() as ReturnType<typeof createMockBackend> & {
    ask(session: string, command: string): void;
  };

  const call = async (path: string, method = 'GET', body?: unknown) => {
    const res = await backend.fetch(`http://mock.test/api${path}`, {
      method,
      headers: { Authorization: 'Bearer mock-token' },
      ...(body === undefined ? {} : { body: JSON.stringify(body) }),
    });
    const text = await res.text();
    return { status: res.status, body: (text ? JSON.parse(text) : null) as unknown };
  };
  const inbox = async () =>
    ((await call('/inbox')).body as { items: PendingItem[] }).items.map((i) => i.id);
  const rows = async (session: string) =>
    ((await call(`/sessions/${session}/transcript`)).body as TranscriptPage).events
      .filter((e) => e.kind === 'decision')
      .map((e) => e.text);

  it('offers to remember a Bash command in a project, and a dangerous one never', async () => {
    const items = ((await call('/inbox')).body as { items: PendingItem[] }).items;
    const migrate = items.find((i) => i.id === 'req-auth-1');
    expect(migrate?.grant).toEqual(
      expect.objectContaining({ kind: 'tool', tool: 'Bash', patterns: 'command' }),
    );
    expect(migrate?.grant?.projectName).toBe('api-server');
    const publish = items.find((i) => i.id === 'req-deploy-1');
    expect(publish?.grant?.blocked).toBeTruthy();
  });

  it('keeps the answer, and allows what it covers without a card', async () => {
    const bad = await call('/sessions/s-auth/respond', 'POST', {
      requestId: 'req-auth-1',
      decision: 'allow',
      remember: { scope: 'project', patterns: ['npm run migrate && rm -rf /'] },
    });
    expect(bad.status).toBe(422);
    expect(await inbox()).toContain('req-auth-1');

    const ok = await call('/sessions/s-auth/respond', 'POST', {
      requestId: 'req-auth-1',
      decision: 'allow',
      remember: { scope: 'project', patterns: ['npm run migrate'] },
    });
    expect(ok.status).toBe(204);
    expect(await inbox()).not.toContain('req-auth-1');
    // The project's Bash permission was there already: it grew.
    const view = (await call('/permissions')).body as PermissionsView;
    const kept = view.rules.find((r) => r.patterns.includes('npm run migrate'));
    expect(kept).toEqual(
      expect.objectContaining({
        id: 'pr_0000000000a1',
        scope: 'project',
        projectName: 'api-server',
        patterns: ['npm test', 'npm run lint', 'npm run migrate'],
      }),
    );
    const rule = 'Bash commands like npm test, npm run lint, npm run migrate in api-server';
    expect((await rows('s-auth')).at(-1)).toBe(`Allowed by the Mac · remembered: ${rule}`);

    // The same command again: no card, the timeline says which permission.
    const uses = kept?.uses ?? 0;
    backend.ask('s-auth', 'npm run migrate -- --dry-run');
    expect((await inbox()).some((id) => id.startsWith('req-ask-'))).toBe(false);
    expect((await rows('s-auth')).at(-1)).toBe(`Allowed by your permission · ${rule}`);
    const after = (await call('/permissions')).body as PermissionsView;
    expect(after.rules.find((r) => r.id === kept?.id)?.uses).toBe(uses + 1);
    // Anything else still asks.
    backend.ask('s-auth', 'npm run seed');
    expect((await inbox()).some((id) => id.startsWith('req-ask-'))).toBe(true);
  });
});
