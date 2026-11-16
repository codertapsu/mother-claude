import { MockPlugins } from './mock-plugins';

const read = async (r: Response | null) => {
  expect(r).not.toBeNull();
  return { status: r!.status, body: (await r!.json()) as Record<string, unknown> };
};
const q = (s = '') => new URLSearchParams(s);

describe('MockPlugins (?mock=1)', () => {
  it('installs after the command its marketplace runs was accepted, and keeps its own plugin', async () => {
    const m = new MockPlugins(true, Date.now());
    const first = await read(
      m.route('/plugins/install', 'POST', { plugin: 'needs-command@claude-plugins-official' }, q()),
    );
    expect(first.status).toBe(409);
    const error = first.body['error'] as { code: string; sha256: string };
    expect(error.code).toBe('confirm_command');
    const ok = await read(
      m.route(
        '/plugins/install',
        'POST',
        { plugin: 'needs-command@claude-plugins-official', acceptCommand: error.sha256 },
        q(),
      ),
    );
    expect(ok.status).toBe(200);
    const view = await read(m.route('/plugins', 'GET', undefined, q()));
    expect((view.body['installed'] as { id: string }[]).map((p) => p.id)).toContain(
      'needs-command@claude-plugins-official',
    );
    const ours = await read(
      m.route('/plugins/uninstall', 'POST', { plugin: 'mother-claude@mother-claude-local' }, q()),
    );
    expect(ours.status).toBe(409);
    expect(m.route('/plugins/available', 'GET', undefined, q('q=figma'))).not.toBeNull();
  });

  it("reconnects a session's failed server, and a phone changes nothing", async () => {
    const m = new MockPlugins(false, Date.now());
    const how = { owned: true, tmux: false };
    const before = await read(m.routeSession('s1', 'mcp', 'GET', undefined, how));
    const figma = () =>
      (before.body['servers'] as { name: string; status: string }[]).find(
        (s) => s.name === 'figma-mcp-node',
      );
    expect(figma()?.status).toBe('failed');
    const after = await read(
      m.routeSession('s1', 'mcp/reconnect', 'POST', { server: 'figma-mcp-node' }, how),
    );
    expect(
      (after.body['servers'] as { name: string; status: string }[]).find(
        (s) => s.name === 'figma-mcp-node',
      )?.status,
    ).toBe('connected');
    const phone = await read(m.route('/plugins/install', 'POST', { plugin: 'figma@x' }, q()));
    expect(phone.status).toBe(403);
  });
});
