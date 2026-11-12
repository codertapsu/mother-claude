// Dev-only: Machines (`/api/machines*`, docs/PLAN-MACHINES.md) for
// `?mock=1`. A pairing runs by itself: as the host, a computer joins a few
// seconds after the code shows; as the joiner, the host answers. Confirming
// pairs it. `?mockmachines=1` starts with Machines on and two computers
// paired, and moves of sessions between them (`mock-moves.ts`). Never
// bundled in production builds.

import {
  AccountsTable,
  CodeSettings,
  FoldersView,
  Grants,
  Introduction,
  MachineAsk,
  MachinesFrame,
  MachinesView,
  PairingView,
  PeerMachine,
  ProjectInput,
  ProjectOptions,
  Readiness,
  Revocation,
  RootEntry,
  RootsChange,
  RootsView,
  SetupCheck,
  SetupView,
  SyncPair,
  defaultCode,
  defaultGrants,
} from '../machines';

const S = 1000;
const MIN = 60 * S;

/** What this build can do with other computers (`protocol::CAPABILITIES`). */
const ALL_CAPABILITIES = [
  'hello',
  'setup',
  'settings',
  'bulk',
  'roots',
  'moves',
  'ignoredFiles',
  'run',
  'tasks',
  'teamRoles',
  'sync',
  'introductions',
  'revokeEverywhere',
];

function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

function err(status: number, code: string, message: string, extra: object = {}): Response {
  return json({ error: { code, message, ...extra } }, status);
}

export class MockMachines {
  private enabled = false;
  private setupStep: number | null = null;
  private selfName = 'MacBook';
  private relay: { mode: 'public' | 'custom' | 'none'; url: string | null } = {
    mode: 'public',
    url: null,
  };
  private keepAwake = true;
  private code: CodeSettings = defaultCode();
  private projects: ProjectOptions[] = [];
  private peers: PeerMachine[] = [];
  private pairing: PairingView | null = null;
  private timer?: ReturnType<typeof setTimeout>;
  private tests: Readiness[] = [];
  private roots: RootEntry[] = [];
  /** What other computers asked this one (shown on the Machines page). */
  private asks: MachineAsk[] = [
    {
      id: 'ask_0000000000000001',
      from: 'f'.repeat(32),
      fromName: 'DESKTOP-PC',
      kind: 'signIn',
      account: 'Work',
      at: Date.now() - 2 * MIN,
      state: 'asked',
    },
  ];
  /** What this computer asked the others. */
  private asked: MachineAsk[] = [];
  /** Introductions received (Phase 5). */
  private introductions: Introduction[] = [];
  /** Computers revoked everywhere (Phase 5). */
  private revocations: Revocation[] = [];
  /** Projects kept in sync (`?mockmachines=1`: one in sync, one changed
   * on both). */
  private syncPairs: SyncPair[] = [];

  constructor(
    private readonly opts: {
      desktop: boolean;
      broadcast: (frame: MachinesFrame) => void;
      /** `?mockmachines=1`: on, with DESKTOP-PC (online) and BUILD-BOX
       * (offline) paired (`mock-moves.ts` serves their sessions). */
      paired?: boolean;
    },
  ) {
    if (opts.paired) {
      this.enabled = true;
      const now = Date.now();
      this.peers = [
        this.pairedPeer('f'.repeat(32), 'DESKTOP-PC', 'windows', true, now),
        this.pairedPeer('e'.repeat(32), 'BUILD-BOX', 'linux', false, now - 40 * MIN),
      ];
      this.syncPairs = [
        this.syncPair('sy_0000000000000001', 'app', {
          state: 'inSync',
          at: now - 2 * MIN,
        }),
        {
          ...this.syncPair('sy_0000000000000002', 'website', {
            state: 'paused',
            reason: 'conflict',
            detail: 'website changed on both computers: choose which one’s to keep.',
            since: now - 5 * MIN,
          }),
          conflict: {
            at: now - 5 * MIN,
            first: false,
            here: {
              files: 3,
              headMoved: false,
              branchSwitched: false,
              branch: 'main',
              head: 'a1b2c3d4e5',
            },
            there: {
              files: 1,
              headMoved: true,
              branchSwitched: false,
              branch: 'main',
              head: 'f6e5d4c3b2',
            },
          },
        },
      ];
    }
  }

  private syncPair(id: string, name: string, status: SyncPair['status']): SyncPair {
    const now = Date.now();
    return {
      id,
      machine: 'f'.repeat(32),
      machineName: 'DESKTOP-PC',
      machineOs: 'windows',
      online: true,
      folder: `/Users/you/development/projects/${name}`,
      name,
      there: `D:\\development\\projects\\${name}`,
      here: { on: true },
      peer: { on: true, asked: false },
      status,
      last: {
        at: now - 2 * MIN,
        step: 14,
        branch: 'main',
        head: 'a1b2c3d4e5',
        direction: 'in',
        files: 2,
      },
      conflict: null,
      kept: [],
      createdAt: now - 2 * 24 * 60 * MIN,
      updatedAt: now - 2 * MIN,
    };
  }

  /** `/api/machines/sync*` (code sync, docs/PLAN-MACHINES.md §5.15). */
  private syncRoute(path: string, method: string, b: Record<string, unknown>): Response | null {
    const view = () => json({ pairs: this.syncPairs, watching: true });
    if (path === '/machines/sync' && method === 'PUT') {
      const machine = String(b['machine'] ?? '');
      const project = String(b['project'] ?? '');
      const peer = this.peers.find((p) => p.id === machine);
      if (!peer) return err(404, 'machines_unknown', 'Not a paired computer.');
      this.syncPairs = this.syncPairs.filter((p) => p.folder !== project);
      if (b['on'] === true) {
        const name = project.split(/[\\/]/).filter(Boolean).pop() ?? project;
        this.syncPairs.push({
          ...this.syncPair(`sy_${Date.now().toString(16).padStart(16, '0')}`, name, {
            state: 'waiting',
          }),
          folder: project,
          machine: peer.id,
          machineName: peer.name,
          peer: { on: false, asked: true },
          last: null,
          createdAt: Date.now(),
        });
      }
      this.syncFrame();
      return view();
    }
    const one = /^\/machines\/sync\/([^/]+)\/(now|resolve)$/.exec(path);
    if (one && method === 'POST') {
      const p = this.syncPairs.find((x) => x.id === one[1]);
      if (!p) return err(404, 'sync_unknown', 'No such synced project here.');
      if (one[2] === 'resolve') {
        if (!p.conflict) return err(409, 'sync_no_conflict', 'There is nothing to choose.');
        const keepHere = b['keep'] === 'here';
        p.kept = [
          ...p.kept,
          {
            ref: `refs/mother-claude/sync/${p.id}/kept-20261112T101500-3fa2`,
            side: keepHere ? 'there' : 'here',
            commit: null,
            files: keepHere ? p.conflict.there.files : p.conflict.here.files,
            at: Date.now(),
          },
        ];
        p.conflict = null;
      }
      if (!p.conflict && p.status.state !== 'off' && p.status.state !== 'waiting') {
        p.status = { state: 'inSync', at: Date.now() };
      }
      this.syncFrame();
      return json(p);
    }
    return null;
  }

  private syncFrame(): void {
    this.opts.broadcast({ t: 'machines', what: 'sync' });
  }

  private pairedPeer(
    id: string,
    name: string,
    os: string,
    online: boolean,
    lastSeen: number,
  ): PeerMachine {
    // BUILD-BOX runs an older build (Phase 5's versions and skew).
    const older = os === 'linux';
    return {
      id,
      name,
      os,
      osVersion: os === 'windows' ? 'Windows 11 Pro' : 'Ubuntu 26.04',
      versions: { app: older ? '0.10.9' : '0.11.0', protocol: 1, claude: '2.1.322' },
      key: '8C21 0E5F A9B3 4D67',
      pairedAt: Date.now() - 3 * 24 * 60 * MIN,
      lastSeen,
      online,
      path: online ? 'lan' : null,
      rttMs: online ? 6.1 : null,
      grants: defaultGrants(),
      theirGrants: defaultGrants(),
      capabilities: older
        ? ['hello', 'setup', 'settings', 'bulk', 'moves', 'roots']
        : [...ALL_CAPABILITIES],
      relay: { mode: 'public', url: null },
      problem: null,
      skew: older
        ? {
            app: 'older',
            lacks: ['sync', 'run', 'tasks', 'teamRoles', 'introductions', 'revokeEverywhere'],
          }
        : {},
      pairedWith: [],
    };
  }

  /** `POST /api/machines/introduce`: both offered. */
  private introduce(b: Record<string, unknown>): Response {
    const find = (id: unknown) => this.peers.find((p) => p.id === id);
    const machine = find(b['machine']);
    const other = find(b['with']);
    if (!machine || !other) return err(404, 'machines_unknown', 'Not a paired computer.');
    if (machine.id === other.id) {
      return err(422, 'intro_same', 'Choose two different computers.');
    }
    const tooOld = [machine, other].find((p) => p.skew?.lacks?.includes('introductions'));
    if (tooOld) {
      return err(
        409,
        'machine_too_old',
        `Update Mother Claude on ${tooOld.name} first: it can't be introduced yet.`,
      );
    }
    machine.pairedWith = [...(machine.pairedWith ?? []), other.id];
    other.pairedWith = [...(other.pairedWith ?? []), machine.id];
    this.frame();
    const side = (p: PeerMachine) => ({
      machine: p.id,
      name: p.name,
      state: 'offered',
      detail: null,
    });
    return json({ id: 'in_0123456789abcdef', host: side(other), joiner: side(machine) });
  }

  /** `POST /api/machines/{id}/revoke-everywhere`. */
  private revokeEverywhere(peer: PeerMachine): Response {
    this.peers = this.peers.filter((p) => p.id !== peer.id);
    this.revocations = [
      {
        machine: peer.id,
        name: peer.name,
        key: peer.key,
        at: Date.now(),
        pending: this.peers
          .filter((p) => !p.online)
          .map((p) => ({ id: p.id, name: p.name, online: false, tooOld: false })),
        done: this.peers.filter((p) => p.online).map((p) => ({ id: p.id, name: p.name })),
      },
      ...this.revocations.filter((r) => r.machine !== peer.id),
    ];
    return json(this.view());
  }

  route(path: string, method: string, body: unknown): Response | null {
    if (!path.startsWith('/machines') && path !== '/roots') return null;
    const b = (body ?? {}) as Record<string, unknown>;
    const desk = () =>
      this.opts.desktop ? null : err(403, 'desktop_only', 'Only the desktop app can do this.');
    if (path === '/machines' && method === 'GET') return json(this.view());
    if (path === '/machines/setup' && method === 'GET') return json(this.setup());
    if (path === '/machines/settings' && method === 'GET') return json(this.settingsView());
    if (path === '/machines/accounts' && method === 'GET') return json(this.accounts());
    if (path === '/roots' && method === 'GET') return json(this.rootsView());
    if (path === '/machines/sync' && method === 'GET') {
      return json({ pairs: this.syncPairs, watching: true });
    }
    const refused = desk();
    if (refused) return refused;
    const synced = this.syncRoute(path, method, b);
    if (synced) return synced;
    const ask = /^\/machines\/asks\/([^/]+)$/.exec(path);
    if (ask && method === 'POST') {
      const a = this.asks.find((x) => x.id === ask[1]);
      if (!a) return err(404, 'machines_unknown_ask', 'No such request.');
      a.state = b['answer'] === 'confirm' ? 'done' : 'declined';
      a.answeredAt = Date.now();
      a.detail =
        a.state === 'declined'
          ? 'Not now.'
          : 'The sign-in started: finish it in the browser on this computer.';
      return json({ ask: a, next: a.state === 'done' ? '/accounts' : null });
    }
    switch (`${method} ${path}`) {
      case 'GET /machines/folders':
        return json(this.folders());
      case 'PUT /roots':
        return this.changeRoots(b as RootsChange);
      case 'POST /machines/clone':
        return json({
          cloned: true,
          folder: `/Users/you/development/projects/${String(b['rel'])}`,
        });
      case 'POST /machines/setup/remote':
        return json({
          project: b['project'],
          origin: 'https://github.com/you/app.git',
          results: [
            {
              machine: '0'.repeat(32),
              name: this.selfName,
              here: true,
              state: 'ok',
              detail: 'The remote answers without a prompt.',
            },
            ...this.peers.map((p) => ({
              machine: p.id,
              name: p.name,
              here: false,
              state: 'failed',
              detail:
                'The remote asked for a sign-in: sign in to it on this computer (Git Credential Manager, or gh auth login), then try again.',
            })),
          ],
        });
    }
    switch (`${method} ${path}`) {
      case 'PATCH /machines/self':
        this.selfName = String(b['name'] ?? this.selfName).trim() || this.selfName;
        return json(this.view());
      case 'POST /machines/pair':
        this.startHost();
        return json(this.view());
      case 'DELETE /machines/pair':
        clearTimeout(this.timer);
        this.pairing = null;
        return json(this.view());
      case 'POST /machines/join': {
        const code = String(b['code'] ?? '').replace(/[\s-]/g, '');
        if (code.length !== 8) {
          return err(
            422,
            'pair_code_malformed',
            'A code has 8 letters and digits, like K7QM-4R2X.',
          );
        }
        this.startJoiner();
        return json(this.view());
      }
      case 'POST /machines/confirm':
        return this.confirm(b['grants'] as Grants | undefined);
      case 'GET /machines/nearby':
        return json({ machines: [{ id: 'nearby-1', name: 'DESKTOP-PC', os: 'windows' }] });
      case 'POST /machines/setup/check':
      case 'POST /machines/setup/fix':
        if (b['check'] === 'keepAwake') this.keepAwake = true;
        return json(this.setup());
      case 'POST /machines/setup/ask': {
        const kinds: Record<string, MachineAsk['kind']> = {
          signIn: 'signIn',
          addAccount: 'addAccount',
          clone: 'clone',
        };
        const peer = this.peers.find((p) => p.id === b['machine']);
        const made: MachineAsk = {
          id: `ask_${Math.random().toString(16).slice(2, 18).padEnd(16, '0')}`,
          from: '0'.repeat(32),
          fromName: this.selfName,
          kind: kinds[String(b['check'])] ?? 'fix',
          check: kinds[String(b['check'])] ? undefined : String(b['check']),
          account: b['account'] as string | undefined,
          origin: b['origin'] as string | undefined,
          folder: b['rel'] ? `D:\\development\\projects\\${String(b['rel'])}` : undefined,
          at: Date.now(),
          state: 'asked',
        };
        if (peer) this.asked.push(made);
        return json({ asked: true, ask: made });
      }
      case 'PUT /machines/setup/progress':
        this.setupStep = (b['step'] as number | null) ?? null;
        return json(this.view());
      case 'POST /machines/test':
        this.tests = this.peers.map((p) => ({
          machine: p.id,
          name: p.name,
          at: Date.now(),
          tests: [
            {
              id: 'connect',
              state: 'ok',
              detail: 'Direct, on the same network.',
              ms: 6.2,
              mibps: null,
            },
            { id: 'transfer', state: 'ok', detail: '20 MiB at 28.4 MiB/s.', ms: 704, mibps: 28.4 },
            {
              id: 'tools',
              state: 'ok',
              detail: 'git and Claude Code 2.1.322 on both.',
              ms: null,
              mibps: null,
            },
            { id: 'code', state: 'ok', detail: 'Both ways in 1.4 s.', ms: 1400, mibps: null },
            {
              id: 'accounts',
              state: 'warning',
              detail: `Work: not signed in on ${p.name} · Client X: not on ${p.name}`,
              ms: null,
              mibps: null,
            },
            {
              id: 'folders',
              state: this.roots.length ? 'ok' : 'warning',
              detail: this.roots.length
                ? `1 folder resolves on ${p.name} · 4 of 4 projects map.`
                : `4 projects on both are not mapped yet (step 5)`,
              ms: null,
              mibps: null,
            },
          ],
        }));
        return json({ results: this.tests });
      case 'PUT /machines/settings': {
        if (b['relay']) this.relay = b['relay'] as typeof this.relay;
        if (typeof b['keepAwake'] === 'boolean') this.keepAwake = b['keepAwake'];
        if (b['code']) this.code = { ...this.code, ...(b['code'] as Partial<CodeSettings>) };
        const projects = b['projects'] as
          { upsert?: ProjectInput[]; remove?: string[] } | undefined;
        for (const key of projects?.remove ?? []) {
          this.projects = this.projects.filter((p) => p.key !== key && p.project !== key);
        }
        for (const p of projects?.upsert ?? []) {
          this.projects = this.projects.filter((x) => x.project !== p.project);
          this.projects.push({
            key: p.project,
            project: p.project,
            root: null,
            code: p.code,
            recipe: p.recipe ?? null,
            updatedAt: Date.now(),
          });
        }
        return json(this.settingsView());
      }
      case 'POST /machines/settings/offer':
        return json(this.view());
      case 'POST /machines/enable': {
        const on = b['enabled'] === true;
        const blockers = this.blockers();
        if (on && blockers.length) {
          return err(
            409,
            'machines_blocked',
            `${blockers.length} required check is left: fix it first.`,
            {
              blockers: blockers.map((c) => ({ id: c.id, title: c.title, detail: c.detail })),
            },
          );
        }
        this.enabled = on;
        if (on) this.setupStep = null;
        return json(this.view());
      }
    }
    if (method === 'POST' && path === '/machines/introduce') return this.introduce(b);
    const intro = /^\/machines\/introductions\/([^/]+)$/.exec(path);
    if (intro && method === 'POST') {
      const i = this.introductions.find((x) => x.id === intro[1]);
      if (!i) return err(404, 'intro_unknown', 'No such introduction here.');
      i.state = b['accept'] === true ? 'accepted' : 'declined';
      i.detail = i.state === 'declined' ? 'You declined.' : null;
      return json(this.view());
    }
    const everywhere = /^\/machines\/([^/]+)\/revoke-everywhere$/.exec(path);
    if (everywhere && method === 'POST') {
      const peer = this.peers.find((p) => p.id === everywhere[1]);
      if (!peer) return err(404, 'machines_unknown', 'Not a paired computer.');
      return this.revokeEverywhere(peer);
    }
    const m = /^\/machines\/([^/]+)$/.exec(path);
    if (m) {
      const peer = this.peers.find((p) => p.id === m[1]);
      if (!peer) return err(404, 'machines_unknown', 'Not a paired computer.');
      if (method === 'DELETE') {
        this.peers = this.peers.filter((p) => p.id !== peer.id);
        return json(this.view());
      }
      if (method === 'PATCH') {
        if (typeof b['name'] === 'string') peer.name = b['name'] || 'DESKTOP-PC';
        if (b['grants']) peer.grants = b['grants'] as Grants;
        return json(this.view());
      }
    }
    return err(404, 'not_found', `No mock for ${method} ${path}`);
  }

  private startHost(): void {
    clearTimeout(this.timer);
    this.pairing = {
      role: 'host',
      state: 'waiting',
      code: 'K7QM-4R2X',
      expiresAt: Date.now() + 5 * MIN,
      other: null,
      digits: null,
      mustType: false,
      confirmed: false,
      otherConfirmed: false,
      error: null,
      peerId: null,
    };
    this.timer = setTimeout(() => {
      if (this.pairing?.state !== 'waiting') return;
      this.pairing = {
        ...this.pairing,
        state: 'confirm',
        code: null,
        other: { id: 'f'.repeat(32), name: 'DESKTOP-PC', os: 'windows', path: 'lan' },
        digits: '481270',
      };
      this.frame();
    }, 6 * S);
  }

  private startJoiner(): void {
    clearTimeout(this.timer);
    this.pairing = {
      role: 'joiner',
      state: 'joining',
      code: null,
      expiresAt: Date.now() + 10 * MIN,
      other: null,
      digits: null,
      mustType: false,
      confirmed: false,
      otherConfirmed: false,
      error: null,
      peerId: null,
    };
    this.timer = setTimeout(() => {
      if (this.pairing?.state !== 'joining') return;
      this.pairing = {
        ...this.pairing,
        state: 'confirm',
        other: { id: 'e'.repeat(32), name: 'BUILD-BOX', os: 'linux', path: 'relay' },
        digits: '903512',
      };
      this.frame();
    }, 2 * S);
  }

  private confirm(grants: Grants | undefined): Response {
    const p = this.pairing;
    if (!p || p.state !== 'confirm' || !p.other) {
      return err(409, 'pair_not_ready', 'There is nothing to confirm yet.');
    }
    this.pairing = { ...p, state: 'waitingOther', confirmed: true };
    clearTimeout(this.timer);
    this.timer = setTimeout(() => {
      const q = this.pairing;
      if (!q?.other || q.state !== 'waitingOther') return;
      this.peers = [
        ...this.peers.filter((x) => x.id !== q.other!.id),
        {
          id: q.other.id,
          name: q.other.name,
          os: q.other.os,
          osVersion: q.other.os === 'windows' ? 'Windows 11 Pro' : null,
          versions: { app: '0.11.0', protocol: 1, claude: '2.1.322' },
          key: '8C21 0E5F A9B3 4D67',
          pairedAt: Date.now(),
          lastSeen: Date.now(),
          online: true,
          path: q.other.path,
          rttMs: 6.1,
          grants: grants ?? defaultGrants(),
          theirGrants: defaultGrants(),
          capabilities: ['hello', 'setup', 'settings', 'bulk'],
          relay: { mode: 'public', url: null },
          problem: null,
        },
      ];
      this.pairing = { ...q, state: 'paired', otherConfirmed: true, peerId: q.other.id };
      this.frame();
    }, 1500);
    return json(this.view());
  }

  private checks(): SetupCheck[] {
    const awake: SetupCheck = {
      id: 'keepAwake',
      area: 'power',
      title: 'Stays awake while used',
      state: this.keepAwake ? 'ok' : 'warning',
      detail: this.keepAwake
        ? 'Kept awake while another computer uses it.'
        : 'It may sleep while another computer uses it.',
      why: 'A computer asleep cannot answer the others.',
      severity: 'recommended',
      fix: this.keepAwake ? null : { kind: 'auto', label: 'Keep awake while used' },
    };
    return [
      {
        id: 'app',
        area: 'app',
        title: 'Mother Claude',
        state: 'ok',
        detail: 'Version 0.11.0 · Machines protocol 1 · Claude Code 2.1.322',
        why: 'Every computer needs a version that speaks the same Machines protocol.',
        severity: 'required',
        fix: null,
      },
      awake,
      {
        id: 'localNetwork',
        area: 'network',
        title: 'Local Network access',
        state: 'warning',
        detail:
          'macOS does not let Mother Claude reach your local network. Allow it when macOS asks, or switch it on in System Settings › Privacy & Security › Local Network.',
        why: 'Without it this Mac reaches your other computers only through the relay, even at home.',
        severity: 'recommended',
        fix: { kind: 'settings', label: 'Open Local Network settings' },
      },
      {
        id: 'relay',
        area: 'relay',
        title: 'The relay',
        state: 'ok',
        detail: 'Connected to aps1-1.relay.n0.iroh.link.',
        why: 'Across networks, your computers meet through the relay.',
        severity: 'required',
        fix: null,
      },
      this.peers.length
        ? {
            id: 'pairing',
            area: 'pairing',
            title: 'Paired computers',
            state: 'ok',
            detail: `Paired with ${this.peers.length} computer${this.peers.length === 1 ? '' : 's'}.`,
            why: 'Machines works between computers you paired.',
            severity: 'required',
            fix: null,
          }
        : {
            id: 'pairing',
            area: 'pairing',
            title: 'Paired computers',
            state: 'blocker',
            detail: 'No other computer is paired yet.',
            why: 'Machines works between computers you paired.',
            severity: 'required',
            fix: { kind: 'manual', label: 'Pair a machine' },
          },
    ];
  }

  private blockers(): SetupCheck[] {
    return this.checks().filter((c) => c.state === 'blocker' && !c.scope);
  }

  private settingsView(): object {
    return {
      relay: this.relay,
      keepAwake: this.keepAwake,
      code: this.code,
      projects: this.projects,
    };
  }

  private accounts(): AccountsTable {
    const me = '0'.repeat(32);
    const machines = [
      { id: me, name: this.selfName, here: true, online: true, error: null },
      ...this.peers.map((p) => ({
        id: p.id,
        name: p.name,
        here: false,
        online: p.online,
        error: null,
      })),
    ];
    const cells = (here: AccountsTable['accounts'][number]['cells'][string], there: typeof here) =>
      Object.fromEntries(machines.map((m) => [m.id, m.here ? here : there]));
    return {
      machine: me,
      machines,
      accounts: [
        {
          name: 'Default',
          default: true,
          cells: cells(
            { state: 'signedIn', kind: 'default' },
            { state: 'signedIn', kind: 'default' },
          ),
          asks: {},
        },
        {
          name: 'Work',
          default: false,
          cells: cells(
            { state: 'signedIn', kind: 'directory' },
            { state: 'signedOut', kind: 'directory' },
          ),
          asks: Object.fromEntries(
            this.asked
              .filter((a) => a.kind === 'signIn' && a.account === 'Work')
              .map((a) => [this.peers[0]?.id ?? '', a]),
          ),
        },
        {
          name: 'Client X',
          default: false,
          cells: cells({ state: 'signedIn', kind: 'token' }, { state: 'missing' }),
          asks: {},
        },
      ],
    };
  }

  private folders(): FoldersView {
    const me = '0'.repeat(32);
    return {
      machine: me,
      repos: 4,
      noOrigin: [{ name: 'notes', path: '/Users/you/notes' }],
      peers: this.peers.map((p) => ({
        machine: p.id,
        name: p.name,
        online: p.online,
        error: null,
        proposals: this.roots.length
          ? []
          : [
              {
                id: 'pr_0123456789ab',
                name: 'projects',
                here: '/Users/you/development/projects',
                there: 'D:\\development\\projects',
                repos: ['mother-claude', 'codex-http-bridge', 'tray-tool', 'site'].map((n) => ({
                  name: n,
                  origin: `https://github.com/you/${n}.git`,
                  here: `/Users/you/development/projects/${n}`,
                  there: `D:\\development\\projects\\${n}`,
                })),
              },
            ],
        matched: [],
        mismatched: [],
        onlyHere: [
          {
            name: 'side-project',
            path: '/Users/you/development/projects/side-project',
            origin: 'https://github.com/you/side-project',
            ...(this.roots.length
              ? {
                  root: this.roots[0].id,
                  rel: 'side-project',
                  mapped: 'D:\\development\\projects\\side-project',
                }
              : {}),
          },
        ],
        onlyThere: [
          {
            name: 'win-tools',
            path: 'D:\\development\\projects\\win-tools',
            origin: 'https://github.com/you/win-tools',
          },
        ],
      })),
    };
  }

  private rootsView(): RootsView {
    const me = '0'.repeat(32);
    return {
      machine: me,
      machines: [
        { id: me, name: this.selfName, os: 'macos', here: true, paired: false },
        ...this.peers.map((p) => ({ id: p.id, name: p.name, os: p.os, here: false, paired: true })),
      ],
      roots: this.roots,
      max: 64,
    };
  }

  private changeRoots(c: RootsChange): Response {
    for (const id of c.remove ?? []) this.roots = this.roots.filter((r) => r.id !== id);
    for (const u of c.upsert ?? []) {
      const existing = u.id ? this.roots.find((r) => r.id === u.id) : undefined;
      if (existing) {
        existing.name = u.name;
        existing.paths = u.paths;
        existing.updatedAt = Date.now();
      } else {
        this.roots.push({
          id: `rt_${Math.random().toString(16).slice(2, 18).padEnd(16, '0')}`,
          name: u.name,
          paths: u.paths,
          updatedAt: Date.now(),
          by: '0'.repeat(32),
        });
      }
    }
    return json(this.rootsView());
  }

  private setup(): SetupView {
    const self = this.checks();
    const peers = this.peers.map((p) => ({
      machine: p.id,
      name: p.name,
      at: Date.now() - 2 * MIN,
      checks: [
        {
          id: 'firewall',
          area: 'network',
          title: 'The firewall',
          state: 'warning' as const,
          detail: 'No firewall rule lets your other computers reach Mother Claude.',
          why: 'Without a rule, your other computers reach this PC only through the relay.',
          severity: 'recommended' as const,
          fix: { kind: 'admin' as const, label: 'Allow through the firewall' },
        },
      ],
      error: null,
      asks: this.asked,
    }));
    const all = [...self, ...peers.flatMap((p) => p.checks)];
    return {
      self: {
        machine: '0'.repeat(32),
        name: this.selfName,
        at: Date.now() - MIN,
        checks: self,
        error: null,
      },
      peers,
      blockers: all.filter((c) => c.state === 'blocker').length,
      gating: all.filter((c) => c.state === 'blocker' && !c.scope).length,
      warnings: all.filter((c) => c.state === 'warning').length,
      tests: this.tests,
    };
  }

  private view(): MachinesView {
    const s = this.setup();
    return {
      enabled: this.enabled,
      self: {
        id: '0'.repeat(32),
        name: this.selfName,
        os: 'macos',
        osVersion: 'macOS 27.2',
        versions: { app: '0.11.0', protocol: 1, claude: '2.1.322' },
        key: '3F9A C1D2 77E0 9B41',
      },
      peers: this.peers,
      pairing: this.pairing,
      settings: { relay: this.relay, keepAwake: this.keepAwake, code: this.code },
      offer: null,
      transport: {
        running: true,
        relay: { url: 'https://aps1-1.relay.n0.iroh.link./', connected: true },
      },
      health: {
        state: s.blockers ? 'blocker' : s.warnings ? 'warning' : 'ok',
        blockers: s.blockers,
        warnings: s.warnings,
        at: Date.now() - MIN,
      },
      setupStep: this.setupStep,
      asks: this.asks,
      introductions: this.introductions,
      revocations: this.revocations,
    };
  }

  private frame(): void {
    this.opts.broadcast({ t: 'machines', what: 'pairing' });
  }
}
