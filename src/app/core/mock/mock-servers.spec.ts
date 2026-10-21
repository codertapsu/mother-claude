import { AccountView, BridgeStatus, RemoteInfo } from '../models';
import { accountFixture, buildFixtures } from './fixtures';
import { MockServers, ServersHost } from './mock-servers';

describe('MockServers (the mock backend of API servers)', () => {
  let main: BridgeStatus;
  let accounts: AccountView[];
  let remote: RemoteInfo;
  let mock: MockServers;

  const make = (desktop = true) => {
    const host: ServersHost = {
      desktop,
      main: () => main,
      setMain: (s) => (main = s),
      accounts: () => accounts,
      remote: () => remote,
      accountRef: () => undefined,
    };
    mock = new MockServers(host);
  };

  beforeEach(() => {
    const fx = buildFixtures(Date.now());
    main = fx.bridge;
    remote = fx.remote;
    accounts = [
      accountFixture({ id: 'default', name: 'Default', isDefault: true }),
      accountFixture({ id: 'acc_work', name: 'Work' }),
    ];
    make();
  });

  const call = async (path: string, method = 'GET', body?: unknown) => {
    const res = mock.route(path, method, body);
    return res ? { status: res.status, body: res.status === 204 ? null : await res.json() } : null;
  };
  const code = async (path: string, method: string, body?: unknown) => {
    const r = await call(path, method, body);
    return [r?.status, r?.body?.error?.code];
  };

  it('leaves the main server alone while no other exists', async () => {
    expect(await call('/bridge')).toBeNull();
    expect(await call('/remote')).toBeNull();
    expect(await call('/bridge/start', 'POST', {})).toBeNull();
    expect(await call('/sessions')).toBeNull();
  });

  it('adds, starts, changes, stops and removes a server', async () => {
    const added = await call('/bridge/servers', 'POST', {
      name: ' Work ',
      defaults: { cwd: '/x', account: 'acc_work', accountOnly: true },
    });
    expect(added?.status).toBe(201);
    expect(added?.body).toEqual(
      jasmine.objectContaining({
        id: 'work',
        name: 'Work',
        port: 5613,
        running: false,
        prefix: '/servers/work',
        defaults: { cwd: '/x', account: 'acc_work', accountOnly: true },
      }),
    );
    const listed = (await call('/bridge'))?.body;
    expect(listed.servers.map((s: { id: string }) => s.id)).toEqual(['main', 'work']);
    expect(listed.enabled).toBeTrue();

    const started = await call('/bridge/servers/work/start', 'POST', {
      defaults: { cwd: '/y', account: 'acc_work' },
      openToTunnels: true,
    });
    expect(started?.body).toEqual(
      jasmine.objectContaining({
        running: true,
        tokenPolicy: 'never',
        defaults: { cwd: '/y', account: 'acc_work' },
      }),
    );

    const changed = await call('/bridge/servers/work', 'PATCH', { name: 'Acme', port: 5620 });
    expect(changed?.body).toEqual(
      jasmine.objectContaining({ id: 'work', name: 'Acme', port: 5620 }),
    );

    const thread = await call('/bridge/servers/work/active-thread', 'POST', { threadId: 't-1' });
    expect(thread?.body.activeThread).toBe('t-1');

    const stopped = await call('/bridge/servers/work/stop', 'POST', {});
    expect(stopped?.body).toEqual(
      jasmine.objectContaining({ running: false, tokenPolicy: 'local' }),
    );

    expect((await call('/bridge/servers/work', 'DELETE'))?.status).toBe(204);
    expect(await call('/bridge')).toBeNull();
  });

  it('gives each new server the first free port and a unique id', async () => {
    await call('/bridge/servers', 'POST', { name: 'Lab', port: 5613 });
    const second = (await call('/bridge/servers', 'POST', { name: 'Lab 2!' }))?.body;
    expect(second).toEqual(jasmine.objectContaining({ id: 'lab-2', port: 5614 }));
    const third = (await call('/bridge/servers', 'POST', { name: '¡¡' }))?.body;
    expect(third.id).toBe('server');
  });

  it('refuses as the server does', async () => {
    await call('/bridge/servers', 'POST', { name: 'Work', port: 5613 });
    expect(await code('/bridge/servers', 'POST', { name: 'work' })).toEqual([
      409,
      'server_name_taken',
    ]);
    expect(await code('/bridge/servers', 'POST', { name: 'Main' })).toEqual([
      409,
      'server_name_taken',
    ]);
    expect(await code('/bridge/servers', 'POST', { name: '' })).toEqual([422, 'invalid_field']);
    expect(await code('/bridge/servers', 'POST', { name: 'B', port: 80 })).toEqual([
      422,
      'invalid_field',
    ]);
    const taken = await call('/bridge/servers', 'POST', { name: 'B', port: 5613 });
    expect(taken?.body.error).toEqual(
      jasmine.objectContaining({ code: 'port_unavailable', port: 5613, server: 'work' }),
    );
    expect(await code('/bridge/servers', 'POST', { name: 'B', port: 5612 })).toEqual([
      409,
      'port_unavailable',
    ]);
    expect(
      await code('/bridge/servers', 'POST', { name: 'B', defaults: { account: 'acc_x' } }),
    ).toEqual([422, 'invalid_field']);
    expect(
      await code('/bridge/servers', 'POST', { name: 'B', defaults: { accountOnly: true } }),
    ).toEqual([422, 'invalid_field']);
    expect(await code('/bridge/servers/main', 'PATCH', { name: 'X' })).toEqual([
      409,
      'main_server_fixed',
    ]);
    expect(await code('/bridge/servers/main', 'DELETE')).toEqual([409, 'main_server_fixed']);
    expect(await code('/bridge/servers/nope/start', 'POST', {})).toEqual([404, 'server_not_found']);
  });

  it('keeps open to tunnels and a bypass mode to the desktop', async () => {
    make(false);
    await call('/bridge/servers', 'POST', { name: 'Phone' });
    expect(await code('/bridge/servers/phone/start', 'POST', { openToTunnels: true })).toEqual([
      403,
      'desktop_only',
    ]);
    expect(
      await code('/bridge/servers/phone/start', 'POST', {
        defaults: { permissionMode: 'bypassPermissions' },
      }),
    ).toEqual([403, 'desktop_only']);
    // Anything else a phone may do.
    expect((await call('/bridge/servers/phone/start', 'POST', {}))?.status).toBe(200);
    // The desktop-only Remote page is the fixture backend's to refuse.
    expect(await call('/remote')).toBeNull();
  });

  it('says when its account was removed, and refuses removing one a running server uses', async () => {
    await call('/bridge/servers', 'POST', {
      name: 'Work',
      defaults: { account: 'acc_work', accountOnly: true },
    });
    await call('/bridge/servers/work/start', 'POST', {});
    const refused = await call('/accounts/acc_work', 'DELETE');
    expect(refused?.body.error).toEqual(
      jasmine.objectContaining({
        code: 'account_used_by_bridge',
        server: 'work',
        serverName: 'Work',
      }),
    );
    await call('/bridge/servers/work/stop', 'POST', {});
    // Stopped: the removal is the fixture backend's.
    expect(await call('/accounts/acc_work', 'DELETE')).toBeNull();
    accounts = accounts.filter((a) => a.id !== 'acc_work');
    const listed = (await call('/bridge'))?.body;
    expect(listed.servers[1].accountMissing).toBeTrue();
    const start = await call('/bridge/servers/work/start', 'POST', {});
    expect(start?.body.error).toEqual(
      jasmine.objectContaining({ code: 'account_not_found', field: 'account' }),
    );
    // Choosing another account (here: automatic) lets it start.
    const again = await call('/bridge/servers/work/start', 'POST', { defaults: {} });
    expect(again?.body.running).toBeTrue();
    expect(again?.body.accountMissing).toBeUndefined();
  });

  it('names the server a tunnel points at on the Remote page', async () => {
    await call('/bridge/servers', 'POST', { name: 'Work' });
    const r = (await call('/remote'))?.body as RemoteInfo;
    expect((r.bridge as unknown as { servers: { id: string }[] }).servers.map((s) => s.id)).toEqual(
      ['main', 'work'],
    );
  });
});
