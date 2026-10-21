import {
  AddressContext,
  MAIN_SERVER,
  ServerStatus,
  absorbReply,
  accountInUseText,
  lanBaseOf,
  normalizeDefaults,
  normalizeServer,
  parsePort,
  portProblem,
  remoteServers,
  serverAccount,
  serverAddresses,
  serverErrorText,
  serverNameProblem,
  serverPanel,
  serversOf,
  tunnelServer,
  withoutServer,
} from './api-servers';
import { ApiError } from './api.service';
import { BridgeStatus, DetectedTunnel, RemoteInfo } from './models';

/** The main payload, as `GET /api/bridge` answered before servers. */
const main = (over: Partial<BridgeStatus> = {}): BridgeStatus => ({
  enabled: true,
  running: true,
  port: 5612,
  url: 'http://127.0.0.1:5612',
  startedAt: 100,
  requireToken: false,
  tokenPolicy: 'local',
  configuredTokenPolicy: 'local',
  exposedByTunnel: false,
  hostBuilt: true,
  hostRunning: true,
  messagesApi: true,
  claudeAuthenticated: true,
  claudeAccount: 'me@example.com (Max)',
  activeOperations: 2,
  defaults: { cwd: '/Users/me' },
  threads: [],
  docsUrl: 'http://127.0.0.1:5612/docs/',
  consoleUrl: 'http://127.0.0.1:5612/example/',
  ...over,
});

const work = {
  id: 'work',
  name: 'Work',
  running: true,
  port: 5613,
  url: 'http://127.0.0.1:5613',
  prefix: '/servers/work',
  startedAt: 200,
  requireToken: false,
  tokenPolicy: 'local',
  exposedByTunnel: true,
  tunnelUrl: 'https://work.ngrok.app',
  defaults: { account: 'acc_work', accountOnly: true, cwd: '/Users/me/acme' },
  activeThread: 't-1',
  threads: [
    {
      threadId: 't-1',
      cwd: '/Users/me/acme',
      createdAt: 1,
      permissionMode: 'default',
      persisted: true,
    },
  ],
  docsUrl: 'http://127.0.0.1:5613/docs/',
  consoleUrl: 'http://127.0.0.1:5613/example/',
};

const personal = {
  id: 'personal',
  name: 'Personal',
  running: false,
  port: 5614,
  url: null,
  prefix: '/servers/personal',
  requireToken: false,
  tokenPolicy: 'local',
  exposedByTunnel: false,
  defaults: { account: 'acc_gone', accountOnly: true },
  threads: [],
  accountMissing: true,
};

const withServers = (servers: unknown[], over: Partial<BridgeStatus> = {}) =>
  ({ ...main(over), servers }) as BridgeStatus;

describe('API servers: reading the wire', () => {
  it('lists none while the main server is the only one', () => {
    expect(serversOf(null)).toEqual([]);
    expect(serversOf(main())).toEqual([]);
    expect(serversOf(withServers([{ id: 'main', name: 'Main' }]))).toEqual([]);
    expect(serversOf({ ...main(), servers: 'nope' } as unknown as BridgeStatus)).toEqual([]);
  });

  it('lists the main server first, its state from the top-level payload', () => {
    const list = serversOf(
      withServers([work, { id: 'main', name: 'Main', running: false, port: 9 }, personal]),
    );
    expect(list.map((s) => s.id)).toEqual(['main', 'work', 'personal']);
    // The top-level payload is the main server's status, as before.
    expect(list[0]).toEqual(
      jasmine.objectContaining({ name: 'Main', running: true, port: 5612, prefix: '' }),
    );
    expect(list[1]).toEqual(
      jasmine.objectContaining({ name: 'Work', port: 5613, prefix: '/servers/work' }),
    );
    expect(list[2].accountMissing).toBeTrue();
    // `url: null` reads as no URL.
    expect(list[2].url).toBeUndefined();
  });

  it('skips what is not a server, and a second entry with the same id', () => {
    const list = serversOf(
      withServers([null, 7, { name: 'no id' }, work, { ...work, name: 'Again' }]),
    );
    expect(list.map((s) => s.name)).toEqual(['Main', 'Work']);
  });

  it('reads one server tolerantly', () => {
    const s = normalizeServer({
      id: 'lab',
      tokenPolicy: 'whatever',
      url: 'javascript:alert(1)',
      tunnelUrl: 'https://lab.example/',
      docsUrl: 'http://127.0.0.1:5620/docs/',
      prefix: '/servers/lab/v1/',
      threads: [{ threadId: 'x' }, { nope: 1 }, null],
      accountMissing: 'yes',
    }) as ServerStatus;
    expect(s.name).toBe('lab');
    expect(s.running).toBeFalse();
    expect(s.tokenPolicy).toBe('local');
    expect(s.url).toBeUndefined();
    expect(s.tunnelUrl).toBe('https://lab.example');
    // A docs link keeps its slash.
    expect(s.docsUrl).toBe('http://127.0.0.1:5620/docs/');
    expect(s.prefix).toBe('/servers/lab');
    expect(s.threads.length).toBe(1);
    expect(s.accountMissing).toBeFalse();
    expect(normalizeServer({ name: 'x' })).toBeNull();
    // A prefix that isn't a plain path is made from the id.
    expect(normalizeServer({ id: 'lab', prefix: 'https://evil' })?.prefix).toBe('/servers/lab');
  });

  it("keeps the defaults' known fields only, accountOnly only when true", () => {
    expect(
      normalizeDefaults({
        cwd: '/x',
        model: '',
        thinking: 'maybe',
        settingSources: ['user', 3],
        account: 'acc_1',
        accountOnly: 'true',
        extra: 1,
      }),
    ).toEqual({ cwd: '/x', settingSources: ['user'], account: 'acc_1' });
    expect(normalizeDefaults({ accountOnly: true, thinking: 'off' })).toEqual({
      accountOnly: true,
      thinking: 'off',
    });
    expect(normalizeDefaults(null)).toEqual({});
  });

  it("shows a server's panel as the main payload with that server's fields", () => {
    const status = withServers([work, personal]);
    const [m, w, p] = serversOf(status);
    expect(serverPanel(status, m)).toBe(status);
    expect(serverPanel(status, null)).toBe(status);
    const pw = serverPanel(status, w);
    expect(pw).toEqual(
      jasmine.objectContaining({
        port: 5613,
        url: 'http://127.0.0.1:5613',
        tunnelUrl: 'https://work.ngrok.app',
        exposedByTunnel: true,
        activeThread: 't-1',
        // The shared parts stay the main payload's.
        hostBuilt: true,
        claudeAccount: 'me@example.com (Max)',
        configuredTokenPolicy: 'local',
      }),
    );
    expect(pw.threads.map((t) => t.threadId)).toEqual(['t-1']);
    // A stopped server without a URL: made from its port, docs too.
    const pp = serverPanel(status, p);
    expect(pp.url).toBe('http://127.0.0.1:5614');
    expect(pp.docsUrl).toBe('http://127.0.0.1:5614/docs/');
    expect(pp.running).toBeFalse();
    expect(pp.startedAt).toBeUndefined();
  });
});

describe('API servers: answers to actions', () => {
  it("takes a whole payload, keeping the servers when it lists none (the main server's routes)", () => {
    const before = withServers([work]);
    const after = absorbReply(before, main({ running: false }))!;
    expect(after.running).toBeFalse();
    expect(serversOf(after).map((s) => s.id)).toEqual(['main', 'work']);
    // With no servers before, the payload is taken as it is.
    expect(absorbReply(main(), main({ running: false }))).toEqual(main({ running: false }));
    // Its own servers win.
    const listed = absorbReply(before, withServers([personal]))!;
    expect(serversOf(listed).map((s) => s.id)).toEqual(['main', 'personal']);
  });

  it("replaces one server's entry, or adds it", () => {
    const before = withServers([work, personal]);
    const stopped = absorbReply(before, { ...work, running: false })!;
    expect(serversOf(stopped).find((s) => s.id === 'work')?.running).toBeFalse();
    expect(serversOf(stopped).map((s) => s.id)).toEqual(['main', 'work', 'personal']);
    // The first server besides the main one makes the list.
    const added = absorbReply(main(), work)!;
    expect(serversOf(added).map((s) => s.name)).toEqual(['Main', 'Work']);
    // The main server's own status changes the top-level payload.
    const m = absorbReply(before, { id: MAIN_SERVER, name: 'Main', running: false, port: 5612 })!;
    expect(m.running).toBeFalse();
    expect(m.claudeAccount).toBe('me@example.com (Max)');
    // Nothing usable: unchanged.
    expect(absorbReply(before, null)).toBe(before);
    expect(absorbReply(before, { name: 'x' })).toBe(before);
  });

  it('drops a removed server, and the list with the last one', () => {
    const two = withoutServer(withServers([{ id: 'main' }, work, personal]), 'work')!;
    expect(serversOf(two).map((s) => s.id)).toEqual(['main', 'personal']);
    const none = withoutServer(two, 'personal')!;
    expect('servers' in none).toBeFalse();
    expect(serversOf(none)).toEqual([]);
    expect(withoutServer(main(), 'work')).toEqual(main());
  });
});

describe('API servers: the account and the Add form', () => {
  const names: Record<string, string> = { acc_work: 'Work' };
  const nameOf = (id: string) => names[id];

  it('says which account a server runs as', () => {
    const [, w, p] = serversOf(withServers([work, personal]));
    expect(serverAccount(w, nameOf)).toEqual({
      id: 'acc_work',
      name: 'Work',
      only: true,
      missing: false,
    });
    expect(serverAccount(p, nameOf)).toEqual(
      jasmine.objectContaining({ name: 'A removed account', missing: true }),
    );
    expect(serverAccount({ defaults: {}, accountMissing: false }, nameOf)).toEqual({
      name: 'Automatic',
      only: false,
      missing: false,
    });
  });

  it('checks a name', () => {
    const list = serversOf(withServers([work, personal]));
    expect(serverNameProblem('  ', list)).toBe('Give it a name.');
    expect(serverNameProblem('x'.repeat(41), list)).toBe('At most 40 characters.');
    expect(serverNameProblem('x'.repeat(40), list)).toBe('');
    expect(serverNameProblem('Tab\there', list)).toBe('Leave out control characters.');
    expect(serverNameProblem('work', list)).toBe('Another API server is called that.');
    expect(serverNameProblem(' WORK ', list, 'work')).toBe('');
    // "Main" is taken, with no other server too.
    expect(serverNameProblem('main', [])).toBe('Another API server is called that.');
    expect(serverNameProblem('Lab', list)).toBe('');
  });

  it('checks a port; empty lets the server pick', () => {
    const list = serversOf(withServers([work, personal]));
    expect(parsePort('')).toBeUndefined();
    expect(parsePort(' 5620 ')).toBe(5620);
    expect(parsePort('56a')).toBeNaN();
    expect(portProblem('', list)).toBe('');
    expect(portProblem('80', list)).toBe('Enter a port from 1024 to 65535, or leave it empty.');
    expect(portProblem('70000', list)).toContain('from 1024 to 65535');
    expect(portProblem('5.5', list)).toContain('from 1024 to 65535');
    expect(portProblem('5613', list)).toBe('Work uses that port.');
    expect(portProblem('5613', list, 'work')).toBe('');
    expect(portProblem('5612', list)).toBe('The main server uses that port.');
    expect(portProblem('5612', [], undefined, 5612)).toBe('The main server uses that port.');
    expect(portProblem('5620', list)).toBe('');
  });
});

describe('API servers: addresses', () => {
  const list = serversOf(withServers([work, personal]));
  const [m, w, p] = list;
  const ctx = (over: Partial<AddressContext> = {}): AddressContext => ({
    desktop: true,
    main: { port: 5612, running: true },
    lanBase: 'https://192.168.1.23:6725',
    ...over,
  });

  it('leads with this Mac on the desktop', () => {
    const a = serverAddresses(w, ctx());
    expect(a.map((x) => [x.key, x.url])).toEqual([
      ['own', 'http://127.0.0.1:5613/v1'],
      ['viaMain', 'http://127.0.0.1:5612/servers/work/v1'],
      ['lan', 'https://192.168.1.23:6725/servers/work/v1'],
      ['tunnel', 'https://work.ngrok.app/v1'],
    ]);
    expect(a[0].label).toBe('On this Mac');
    expect(a[0].hint).toBe('A token is required while a tunnel points at the port.');
    expect(a[2].hint).toContain("paired device's token");
  });

  it("leads with what a phone can reach, and labels the Mac's loopback", () => {
    const a = serverAddresses(
      w,
      ctx({
        desktop: false,
        main: { port: 5612, running: true, tunnelUrl: 'https://main.ngrok.app' },
      }),
    );
    expect(a.map((x) => x.key)).toEqual(['lan', 'tunnel', 'mainTunnel', 'own', 'viaMain']);
    expect(a.find((x) => x.key === 'mainTunnel')?.url).toBe(
      'https://main.ngrok.app/servers/work/v1',
    );
    const own = a.find((x) => x.key === 'own')!;
    expect(own.label).toBe('On the Mac');
    expect(own.hint).toContain('this device cannot reach it');
    expect(a.find((x) => x.key === 'viaMain')?.label).toBe('On the Mac, through the main port');
  });

  it('shows the shape of the network address when it is not known here', () => {
    const lan = serverAddresses(p, ctx({ lanBase: null, dashboardPort: 6725 })).find(
      (x) => x.key === 'lan',
    )!;
    expect(lan.url).toBeNull();
    expect(lan.pattern).toBe("https://<the Mac's address>:6725/servers/personal/v1");
  });

  it('says the main port works only while the main server runs', () => {
    const via = serverAddresses(p, ctx({ main: { port: 5612, running: false } })).find(
      (x) => x.key === 'viaMain',
    )!;
    expect(via.hint).toContain('Only while the main server runs.');
    // No main API port (MOTHER_CLAUDE_BRIDGE_PORT=0): no way through it.
    expect(serverAddresses(p, ctx({ main: { port: 0, running: true } })).map((x) => x.key)).toEqual(
      ['own', 'lan'],
    );
  });

  it('gives the main server its own port, the network and its tunnel', () => {
    const tunnelled = { ...m, tunnelUrl: 'https://main.ngrok.app', tokenPolicy: 'never' as const };
    const a = serverAddresses(
      tunnelled,
      ctx({ main: { port: 5612, running: true, tunnelUrl: 'https://main.ngrok.app' } }),
    );
    expect(a.map((x) => [x.key, x.url])).toEqual([
      ['own', 'http://127.0.0.1:5612/v1'],
      ['lan', 'https://192.168.1.23:6725/v1'],
      ['tunnel', 'https://main.ngrok.app/v1'],
    ]);
    expect(a[2].hint).toBe('Open to tunnels: no token needed.');
  });

  it("finds Mother Claude's LAN address", () => {
    const remote = {
      lan: { enabled: true, addresses: [{ ip: '10.0.0.5', url: 'https://10.0.0.5:6725/' }] },
    } as unknown as RemoteInfo;
    expect(lanBaseOf(remote, 'http://127.0.0.1:6725')).toBe('https://10.0.0.5:6725');
    const off = { lan: { enabled: false, addresses: [] } } as unknown as RemoteInfo;
    expect(lanBaseOf(off, 'https://192.168.1.23:6725')).toBeNull();
    // A phone that opened the dashboard over the LAN: its own origin.
    expect(lanBaseOf(null, 'https://192.168.1.23:6725')).toBe('https://192.168.1.23:6725');
    expect(lanBaseOf(null, 'https://100.101.102.103:6725')).toBe('https://100.101.102.103:6725');
    expect(lanBaseOf(null, 'https://mac.local:6725')).toBe('https://mac.local:6725');
    // Through a tunnel, or plain http: not known here.
    expect(lanBaseOf(null, 'https://abc.ngrok-free.app')).toBeNull();
    expect(lanBaseOf(null, 'http://192.168.1.23:6725')).toBeNull();
    expect(lanBaseOf(null, 'not a url')).toBeNull();
  });
});

describe('API servers: tunnels on the Remote page', () => {
  const remote = (servers?: unknown) =>
    ({ bridge: { port: 5612, running: true, ...(servers ? { servers } : {}) } }) as RemoteInfo;
  const listed = remoteServers(
    remote([
      { id: 'main', name: 'Main', port: 5612, running: true, exposed: false },
      { id: 'work', name: 'Work', port: 5613, running: true, exposed: true },
      { id: 'work', name: 'Dup' },
      { nope: true },
    ]),
  );
  const tunnel = (over: Partial<DetectedTunnel> & { server?: string }) =>
    ({ provider: 'ngrok', url: 'https://x.ngrok.app', ...over }) as DetectedTunnel;

  it('reads the servers, none while only the main one exists', () => {
    expect(listed.map((s) => s.name)).toEqual(['Main', 'Work']);
    expect(listed[1]).toEqual({
      id: 'work',
      name: 'Work',
      port: 5613,
      running: true,
      exposed: true,
    });
    expect(remoteServers(remote())).toEqual([]);
    expect(remoteServers(remote([{ id: 'main', name: 'Main' }]))).toEqual([]);
    expect(remoteServers(null)).toEqual([]);
  });

  it('names the server a tunnel points at', () => {
    expect(tunnelServer(tunnel({ target: 'bridge', server: 'work' }), listed)?.name).toBe('Work');
    // No `server`: the main one.
    expect(
      tunnelServer(tunnel({ target: 'bridge', upstream: 'http://127.0.0.1:5612' }), listed)?.name,
    ).toBe('Main');
    // By the port it forwards to.
    expect(
      tunnelServer(tunnel({ target: 'other', upstream: 'localhost:5613' }), listed)?.name,
    ).toBe('Work');
    expect(tunnelServer(tunnel({ target: 'tunnel', upstreamPort: 6726 }), listed)).toBeNull();
    // A server the list doesn't know yet keeps its id.
    expect(tunnelServer(tunnel({ target: 'bridge', server: 'lab' }), listed)?.name).toBe('lab');
    // Only the main server: nothing to name.
    expect(tunnelServer(tunnel({ target: 'bridge' }), [])).toBeNull();
  });
});

describe('API servers: refusals in words', () => {
  const err = (status: number, code: string, extra: Record<string, unknown> = {}, message = 'm') =>
    new ApiError(message, status, 'http', code, undefined, undefined, extra);
  const servers = [
    { id: 'main', name: 'Main' },
    { id: 'work', name: 'Work' },
  ];

  it('says whose a port is', () => {
    expect(
      serverErrorText(err(409, 'port_unavailable', { port: 5613, server: 'work' }), { servers }),
    ).toBe("Port 5613 is Work's. Pick another, or leave the port empty for the first free one.");
    expect(
      serverErrorText(err(409, 'port_unavailable', { port: 6725 }), { saving: true }),
    ).toContain(
      "Port 6725 is taken: the main server, Mother Claude's own ports or another program has it.",
    );
    expect(
      serverErrorText(err(409, 'port_unavailable', { port: 5614 }), { server: 'Personal' }),
    ).toBe(
      'Port 5614 is in use by another program, so Personal cannot listen on it. Stop that program, or give Personal another port.',
    );
  });

  it("sends the main server's port to the environment", () => {
    expect(
      serverErrorText(err(409, 'port_unavailable', { port: 5612 }), {
        server: 'the API',
        main: true,
      }),
    ).toBe(
      'Port 5612 is in use by another program, so the API cannot listen on it. Stop that program, or set MOTHER_CLAUDE_BRIDGE_PORT to another port.',
    );
    // A sentence starts with a capital, whatever the server is called.
    expect(serverErrorText(err(403, 'account_not_served'), { server: 'the API' })).toMatch(
      /^The API runs only as its own account/,
    );
  });

  it('puts the account refusals in words', () => {
    expect(serverErrorText(err(409, 'account_not_found'), { server: 'Personal' })).toBe(
      "Personal's account was removed. Choose another account (or Automatic) for it, then start it.",
    );
    expect(
      serverErrorText(err(403, 'account_not_served'), { server: 'Work', account: 'Work' }),
    ).toBe("Work runs only as Work: another account's conversations can't be used through it.");
    expect(
      serverErrorText(err(409, 'account_used_by_bridge', { server: 'work', serverName: 'Work' })),
    ).toContain('The API server Work is running');
    expect(serverErrorText(err(404, 'server_not_found'))).toContain('no longer exists');
    expect(serverErrorText(err(409, 'server_name_taken'))).toContain('Another API server');
  });

  it("keeps the server's own words for the rest, as before", () => {
    expect(
      serverErrorText(
        new ApiError('Unknown effort', 422, 'http', 'invalid_field', undefined, 'effort'),
      ),
    ).toBe('Unknown effort (effort)');
    expect(serverErrorText(new Error('boom'))).toBe('boom');
    expect(serverErrorText(new ApiError('x', 0, 'network'))).toContain("Can't reach");
  });

  it('names the server that uses an account', () => {
    expect(
      accountInUseText(err(409, 'account_used_by_bridge', { serverName: 'Work' }), 'Work'),
    ).toBe(
      'The API server Work is running under Work. Stop it, or start it with another account, first.',
    );
    expect(accountInUseText(err(409, 'account_used_by_bridge'), 'Work')).toBe(
      'The API is running with Work as the account its new conversations start under. Stop it, or start it again with another account, first.',
    );
  });
});
