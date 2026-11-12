// Dev-only: copies kept current between computers (docs/PLAN-MACHINES.md
// §5.9) for `?mock=1&mockmachines=1`: one copy held here from DESKTOP-PC
// (online) and one from BUILD-BOX (offline, so *Continue here* shows), and
// a session's *Keep a copy on …*. Never bundled in production builds.

import type { CopiesView, CopyStatus, HeldCopy, KeptCopy, SessionCopiesView } from '../copies';
import type { PeerMachine } from '../machines';

const MIN = 60_000;

function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

function err(status: number, code: string, message: string): Response {
  return json({ error: { code, message } }, status);
}

export class MockCopies {
  /** Session → the computers it keeps a copy on (its own choice). */
  private kept = new Map<string, string[]>();
  private held: HeldCopy[] = [];
  private capBytes = 2 * 1024 ** 3;

  constructor(
    private readonly peers: () => PeerMachine[],
    private readonly frame: () => void,
    paired: boolean,
  ) {
    if (!paired) return;
    const now = Date.now();
    this.held = [
      {
        id: `held.${'f'.repeat(32)}.d0ee0000-1111-4222-8333-444455556666`,
        machine: 'f'.repeat(32),
        machineName: 'DESKTOP-PC',
        online: true,
        session: 'd0ee0000-1111-4222-8333-444455556666',
        title: 'Tray icon on Windows',
        cwd: 'D:\\development\\projects\\app',
        account: 'Default',
        bytes: 18_400_000,
        files: 41,
        copiedAt: now - 3 * MIN,
        turnAt: now - 3 * MIN,
        createdAt: now - 2 * 24 * 60 * MIN,
        complete: true,
        livesHere: false,
        canContinue: false,
      },
      {
        id: `held.${'e'.repeat(32)}.d1ee0000-1111-4222-8333-444455556666`,
        machine: 'e'.repeat(32),
        machineName: 'BUILD-BOX',
        online: false,
        session: 'd1ee0000-1111-4222-8333-444455556666',
        title: 'Release build',
        cwd: '/home/you/projects/app',
        account: 'Default',
        bytes: 2_100_000,
        files: 9,
        copiedAt: now - 41 * MIN,
        turnAt: now - 41 * MIN,
        createdAt: now - 3 * 60 * MIN,
        complete: true,
        livesHere: false,
        canContinue: true,
      },
    ];
  }

  private status(machine: PeerMachine): CopyStatus {
    return machine.online
      ? {
          state: 'copied',
          since: Date.now() - 2 * MIN,
          copiedAt: Date.now() - 2 * MIN,
          turnAt: Date.now() - 2 * MIN,
          bytes: 4_200_000,
          files: 12,
        }
      : {
          state: 'paused',
          reason: 'offline',
          detail: `${machine.name} can't be reached.`,
          since: Date.now() - 5 * MIN,
          bytes: 0,
          files: 0,
        };
  }

  private sessionView(session: string): SessionCopiesView {
    const on = this.kept.get(session) ?? [];
    return {
      session,
      explicit: this.kept.has(session),
      project: [],
      livesOn: null,
      enabled: true,
      machines: this.peers().map((p) => ({
        id: p.id,
        name: p.name,
        os: p.os,
        online: p.online,
        on: on.includes(p.id),
        from: on.includes(p.id) ? 'session' : null,
        inProject: false,
        supported: true,
        status: on.includes(p.id) ? this.status(p) : null,
      })),
    };
  }

  private view(): CopiesView {
    const kept: KeptCopy[] = [];
    for (const [session, machines] of this.kept) {
      for (const id of machines) {
        const p = this.peers().find((x) => x.id === id);
        if (!p) continue;
        kept.push({
          id: `kept.${id}.${session}`,
          machine: id,
          machineName: p.name,
          online: p.online,
          session,
          title: null,
          from: 'session',
          status: this.status(p),
        });
      }
    }
    const used: Record<string, number> = {};
    for (const h of this.held) used[h.machine] = (used[h.machine] ?? 0) + h.bytes;
    return { enabled: true, held: this.held, kept, capBytes: this.capBytes, used };
  }

  /** `/sessions/{id}/copies`, `/machines/copies*`, or null. */
  route(path: string, method: string, body: Record<string, unknown>): Response | null {
    const one = /^\/sessions\/([^/]+)\/copies$/.exec(path);
    if (one) {
      const session = decodeURIComponent(one[1]);
      if (method === 'PUT') {
        const machines = body['machines'];
        if (Array.isArray(machines)) this.kept.set(session, machines.map(String));
        else this.kept.delete(session);
        this.frame();
      }
      return json(this.sessionView(session));
    }
    if (path === '/machines/copies' && method === 'GET') return json(this.view());
    const del = /^\/machines\/copies\/([^/]+)$/.exec(path);
    if (del && method === 'DELETE') {
      const id = decodeURIComponent(del[1]);
      const [kind, machine, session] = id.split('.');
      if (kind === 'held') {
        if (!this.held.some((h) => h.id === id)) return err(404, 'copy_not_found', 'No such copy.');
        this.held = this.held.filter((h) => h.id !== id);
      } else {
        const rest = (this.kept.get(session) ?? []).filter((m) => m !== machine);
        this.kept.set(session, rest);
      }
      this.frame();
      return json(this.view());
    }
    return null;
  }

  /** A take over from a copy held here: it goes. */
  takenOver(session: string): boolean {
    const had = this.held.some((h) => h.session === session);
    this.held = this.held.filter((h) => h.session !== session);
    if (had) this.frame();
    return had;
  }
}
