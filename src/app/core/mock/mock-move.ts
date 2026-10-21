// Dev-only: moving data anywhere for `?mock=1` (docs/ACCOUNTS.md §13) —
// storages beside the backup folder (`/backup/storage*`), GET /backup's
// `storage` list, a set's index by storage and set, and POST /data/move with
// the `items` body: the dry run with its skips and warnings, one job per
// source location, and Undo. The old body (`what`) goes on to mock-data.ts.
// Never bundled in production builds.
//
// With `?mockbackup=1` (or any of its variants) there is a second storage,
// "Google Drive", with a set no account is named after ("Shared") that
// holds an archived conversation. `?mockstorage=none` leaves the backup
// folder alone: with only the Default account GET /backup then answers as
// before (goal 7). A device (`?as=device`) may do all of it except what
// needs the dangerous right: a folder's trust, settings and MCP servers.

import { DEFAULT_ACCOUNT, homeOf } from '../accounts';
import { AccountView, JobView, SessionSummary } from '../models';
import {
  BACKUP_STORAGE,
  DataLocation,
  StorageKind,
  readLocation,
  storageKindOf,
} from '../move-data';
import { urlParam } from '../platform';
import { MockRole } from './mock-api';
import { GDRIVE, HOST, JobEnd, JobNames, MockData, OTHER_HOST, THIS_MACHINE } from './mock-data';

const BACKUP_MODE = urlParam('mockbackup');
const STORAGE_MODE = urlParam('mockstorage');
const TRANSFER_MODE = urlParam('mocktransfer');
const CONFIGURED = ['1', 'wrong', 'nokey', 'unfinished', 'tampered'].includes(BACKUP_MODE ?? '');

const M = 60_000;
const H = 60 * M;
const D = 24 * H;
const SHARED_DIR = '/Users/marcus/.claude';
const KIND_LABELS: Record<StorageKind, string> = {
  icloud: 'iCloud Drive',
  gdrive: 'Google Drive',
  onedrive: 'OneDrive',
  dropbox: 'Dropbox',
  folder: 'Folder',
};
/** Copying these needs the dangerous right. */
const DANGEROUS_ITEMS = new Set(['settings', 'mcpServers']);
const CUSTOMIZATION_ITEMS = [
  'memory',
  'settings',
  'skills',
  'agents',
  'commands',
  'outputStyles',
  'keybindings',
  'mcpServers',
  'history',
];

/** What Move data's mock needs from the fixture backend. */
export interface MoveHost {
  readonly role: MockRole;
  /** Every session the mock knows (live list and history). */
  sessions(): SessionSummary[];
  accounts(): AccountView[];
  /** The account a session is recorded as (the Default when none). */
  accountOf(sessionId: string): string;
  reassign(sessionId: string, account: string): void;
}

/** One conversation in a set. */
interface Entry {
  id: string;
  cwd: string;
  title: string;
  bytes: number;
  pushedAt: number;
  newestMtime: number;
  host: string;
  archived?: { at: number; machine: string };
}

interface Store {
  id: string;
  label: string;
  path: string;
  kind: StorageKind;
  available: boolean;
}

/** Something that goes: a conversation of an account or of a set. */
interface Candidate {
  id: string;
  cwd: string;
  title: string;
  bytes: number;
  live: boolean;
  /** Where it comes from. */
  source: DataLocation;
  entry?: Entry;
}

type Direction = 'between' | 'assign' | 'toStorage' | 'fromStorage' | 'storages';

interface Skip {
  id?: string;
  cwd?: string;
  item?: string;
  code: string;
  message: string;
}

interface Planned {
  c: Candidate;
  direction: Direction;
  action: 'move' | 'copy' | 'assign' | 'archive' | 'restore';
}

interface Custom {
  item: string;
  name: string;
  action: 'add' | 'replace' | 'skip' | 'merge';
  reason?: string;
}

export class MockMove {
  private readonly stores: Store[] = [];
  /** Entries per set (`<storage>\u0000<set>`): every entry of a set only
   * Move data knows, and what it added to an account's set in the backup
   * folder. */
  private readonly sets = new Map<string, Entry[]>();
  /** Conversations archived out of their account (id → that account). */
  private readonly gone = new Map<string, string>();

  constructor(
    private readonly host: MoveHost,
    private readonly data: MockData,
  ) {
    if (!CONFIGURED || STORAGE_MODE === 'none') return;
    const now = Date.now();
    const drive: Store = {
      id: 'stor_5d2c8e1f0a7b',
      label: 'Google Drive',
      path: GDRIVE,
      kind: 'gdrive',
      available: true,
    };
    this.stores.push(drive);
    this.sets.set(key(drive.id, 'Shared'), [
      {
        id: '0b7e3c1a-9d2f-4e6b-8a5c-1f3d5e7a9b2c',
        cwd: '/Users/marcus/dev/website',
        title: 'Landing page copy review',
        bytes: 188_000,
        pushedAt: now - 3 * D,
        newestMtime: now - 3 * D - 2 * H,
        host: OTHER_HOST,
      },
      {
        id: '1c8f4d2b-0e3a-4f7c-9b6d-2a4e6f8b0c3d',
        cwd: '/Users/marcus/dev/payments',
        title: 'Invoice PDF layout',
        bytes: 264_000,
        pushedAt: now - 2 * D,
        newestMtime: now - 2 * D - 5 * H,
        host: HOST,
      },
      {
        id: '2d9a5e3c-1f4b-4a8d-8c7e-3b5f7a9c1d4e',
        cwd: '/Users/marcus/dev/prototype',
        title: 'Old prototype notes',
        bytes: 92_000,
        pushedAt: now - 12 * D,
        newestMtime: now - 14 * D,
        host: HOST,
        archived: { at: now - 12 * D, machine: THIS_MACHINE },
      },
    ]);
  }

  /** Answers a storage route or a move with `items`, or null to let the data
   * mock answer. */
  async route(
    path: string,
    method: string,
    body: unknown,
    q: URLSearchParams,
  ): Promise<Response | null> {
    const b = obj(body);
    if (path === '/data/move' && method === 'POST' && b && 'items' in b) return this.move(b);
    if (path === '/backup' && method === 'GET') return this.view();
    if (path === '/backup/index' && method === 'GET') {
      return q.has('storage') ? this.storageIndex(q) : this.accountIndex(q);
    }
    if (path === '/backup/storage' && method === 'POST') return this.addStorage(b ?? {});
    const m = /^\/backup\/storage\/([^/]+)$/.exec(path);
    if (m && method === 'PATCH') return this.renameStorage(decodeURIComponent(m[1]), b ?? {});
    if (m && method === 'DELETE') return this.removeStorage(decodeURIComponent(m[1]));
    return null;
  }

  // ───────────────────────────────────────────────────────── Storages

  private async backupView(): Promise<Record<string, unknown>> {
    const res = this.data.route('/backup', 'GET', undefined, new URLSearchParams());
    return res ? ((await (await res).json()) as Record<string, unknown>) : {};
  }

  private async view(): Promise<Response> {
    const v = await this.backupView();
    // Goal 7: only the Default and the backup folder answer as before.
    if (!this.stores.length && this.host.accounts().length <= 1) return json(v);
    return json({ ...v, storage: this.storageList(v) });
  }

  private storageList(v: Record<string, unknown>): Record<string, unknown>[] {
    const out: Record<string, unknown>[] = [];
    const folder = folderOf(v);
    if (folder) {
      const own = accountSets(v);
      const sets: Record<string, unknown>[] = own.map((s) => {
        const mine = this.sets.get(key(BACKUP_STORAGE, s.name)) ?? [];
        const archived = mine.filter((e) => e.archived).length;
        return {
          name: s.name,
          account: s.account,
          exists: s.exists || mine.length > 0,
          ...(s.keyMissing ? { keyMissing: true } : {}),
          ...(s.sessions !== undefined || mine.length
            ? { sessions: (s.sessions ?? 0) + mine.length }
            : {}),
          ...(archived ? { archived } : {}),
        };
      });
      const named = new Set(own.map((s) => s.name));
      for (const name of this.setNames(BACKUP_STORAGE)) {
        if (!named.has(name)) sets.push(this.setWire(BACKUP_STORAGE, name));
      }
      const kind = storageKindOf(folder);
      out.push({
        id: BACKUP_STORAGE,
        // As the server names it.
        label: 'Backup folder',
        path: folder,
        kind,
        primary: true,
        available: true,
        sets,
      });
    }
    for (const st of this.stores) out.push(this.storeWire(st));
    return out;
  }

  private storeWire(st: Store): Record<string, unknown> {
    return {
      id: st.id,
      label: st.label,
      path: st.path,
      kind: st.kind,
      primary: false,
      available: st.available,
      sets: this.setNames(st.id).map((name) => this.setWire(st.id, name)),
    };
  }

  private setWire(storage: string, name: string): Record<string, unknown> & { name: string } {
    const entries = this.sets.get(key(storage, name)) ?? [];
    const archived = entries.filter((e) => e.archived).length;
    return {
      name,
      exists: true,
      sessions: entries.length,
      ...(archived ? { archived } : {}),
    };
  }

  private setNames(storage: string): string[] {
    return [...this.sets.keys()]
      .filter((k) => k.startsWith(`${storage}\u0000`))
      .map((k) => k.slice(storage.length + 1));
  }

  private storagePath(v: Record<string, unknown>, id: string): string | null {
    if (id === BACKUP_STORAGE) return folderOf(v);
    return this.stores.find((s) => s.id === id)?.path ?? null;
  }

  private async addStorage(b: Record<string, unknown>): Promise<Response> {
    const raw = typeof b['path'] === 'string' ? b['path'].trim().replace(/\/+$/, '') : '';
    if (!raw) return fail(422, 'invalid_body', "Send the folder's path.");
    if (!raw.startsWith('/') && !raw.startsWith('~')) {
      return fail(422, 'invalid_folder', 'a storage is an absolute folder');
    }
    const path = raw.replace(/^~(?=\/|$)/, '/Users/marcus');
    if (path.split('/').some((p) => p === '.' || p === '..')) {
      return fail(422, 'invalid_folder', 'a storage path has no . or .. in it');
    }
    const inside = this.host
      .accounts()
      .find((a) => a.kind === 'directory' && within(path, a.dir ?? SHARED_DIR));
    if (inside) {
      return fail(422, 'invalid_folder', `${path} is inside ${inside.name}'s directory`);
    }
    const v = await this.backupView();
    const taken = [folderOf(v), ...this.stores.map((s) => s.path)].find(
      (p): p is string => !!p && (within(path, p) || within(p, path)),
    );
    if (taken) return fail(409, 'storage_exists', `${taken} is a storage already`);
    const kind = storageKindOf(path);
    const label =
      (typeof b['label'] === 'string' && b['label'].trim()) ||
      (kind === 'folder' ? (path.split('/').pop() ?? 'Folder') : KIND_LABELS[kind]);
    if ([...label].length > 40)
      return fail(422, 'invalid_storage', 'a label has at most 40 characters');
    const st: Store = { id: `stor_${hex(12)}`, label, path, kind, available: true };
    this.stores.push(st);
    return json(this.storeWire(st), 201);
  }

  private renameStorage(id: string, b: Record<string, unknown>): Response {
    const st = this.stores.find((s) => s.id === id);
    if (id === BACKUP_STORAGE) {
      return fail(422, 'invalid_storage', 'the backup folder is named after where it is');
    }
    if (!st) return fail(404, 'storage_not_found', `no storage ${JSON.stringify(id)}`);
    const label = typeof b['label'] === 'string' ? b['label'].trim() : '';
    if (!label) return fail(422, 'invalid_storage', 'a storage needs a label');
    if ([...label].length > 40)
      return fail(422, 'invalid_storage', 'a label has at most 40 characters');
    st.label = label;
    return json(this.storeWire(st));
  }

  private removeStorage(id: string): Response {
    if (id === BACKUP_STORAGE) {
      return fail(422, 'invalid_storage', 'the backup folder is set on Backup & sync');
    }
    const i = this.stores.findIndex((s) => s.id === id);
    if (i < 0) return fail(404, 'storage_not_found', `no storage ${JSON.stringify(id)}`);
    this.stores.splice(i, 1);
    // Forgotten, never deleted: adding the folder again finds its sets.
    for (const k of [...this.sets.keys()]) if (k.startsWith(`${id}\u0000`)) this.sets.delete(k);
    return new Response(null, { status: 204 });
  }

  /** The entries of a set: an account's set in the backup folder is the
   * data mock's (what it backed up) with what Move data added; any other
   * only Move data's. Null when there is no such set. */
  private async entries(v: Record<string, unknown>, storage: string, set: string) {
    const mine = this.sets.get(key(storage, set));
    if (storage !== BACKUP_STORAGE) return mine ?? null;
    const named = accountSets(v).find((s) => s.name === set);
    if (!named) return mine ?? null;
    const res = this.data.route(
      '/backup/index',
      'GET',
      undefined,
      new URLSearchParams({ account: named.account }),
    );
    const ix = res ? ((await (await res).json()) as Record<string, unknown>) : {};
    const base = (Array.isArray(ix['sessions']) ? ix['sessions'] : [])
      .map(obj)
      .filter((s): s is Record<string, unknown> => !!s && typeof s['id'] === 'string')
      .map((s): Entry => ({
        id: s['id'] as string,
        cwd: String(s['cwd'] ?? ''),
        title: String(s['title'] ?? ''),
        bytes: Number(s['remoteBytes'] ?? 0),
        pushedAt: Number(s['pushedAt'] ?? 0),
        newestMtime: Number(s['newestMtime'] ?? 0),
        host: String(s['host'] ?? HOST),
      }));
    if (!base.length && !mine && !named.exists) return null;
    const added = new Set((mine ?? []).map((e) => e.id));
    return [...(mine ?? []), ...base.filter((e) => !added.has(e.id))];
  }

  /** Why a set can't be read or written now (its key, its check, the
   * passphrase), or null. */
  private setRefusal(v: Record<string, unknown>, storage: string, set: string): Response | null {
    if (storage === BACKUP_STORAGE) {
      const named = accountSets(v).find((s) => s.name === set);
      if (named?.keyMissing) {
        return fail(
          409,
          'set_key_missing',
          `the backup set in ${folderOf(v)}/${set} has no key (identity.age) here`,
        );
      }
      if (BACKUP_MODE === 'tampered' && named?.account === DEFAULT_ACCOUNT) {
        return fail(
          409,
          'backup_unauthenticated',
          `the index of the backup set in ${folderOf(v)}/${set} is not authentic`,
        );
      }
    }
    if (BACKUP_MODE === 'wrong') {
      return fail(403, 'wrong_passphrase', 'the passphrase does not open this backup set');
    }
    return null;
  }

  private prerequisite(v: Record<string, unknown>, storage: string): Response | null {
    const path = this.storagePath(v, storage);
    if (!path) {
      return storage === BACKUP_STORAGE
        ? fail(409, 'no_backup_folder', 'no backup folder is set')
        : fail(404, 'storage_not_found', `no storage ${JSON.stringify(storage)}`);
    }
    if (v['passphraseSet'] !== true)
      return fail(409, 'no_passphrase', 'no backup passphrase is set');
    return null;
  }

  private async storageIndex(q: URLSearchParams): Promise<Response> {
    const storage = q.get('storage') ?? '';
    const set = q.get('set') ?? '';
    const v = await this.backupView();
    const pre = this.prerequisite(v, storage);
    if (pre) return pre;
    const refused = this.setRefusal(v, storage, set);
    if (refused) return refused;
    const entries = await this.entries(v, storage, set);
    const path = `${this.storagePath(v, storage)}/${set}`;
    if (!entries) return fail(422, 'invalid_backup', `there is no set in ${path}`);
    const named = storage === BACKUP_STORAGE ? accountSets(v).find((s) => s.name === set) : null;
    return json({
      ...(named ? { account: named.account } : {}),
      name: set,
      set: path,
      machines: [{ id: THIS_MACHINE, host: HOST, lastPush: Date.now() - 2 * H }],
      sessions: entries.map((e) => this.indexItem(e)),
      config: [],
    });
  }

  /** The data mock's index of an account's set, with what Move data put in
   * that set (archived entries marked). */
  private async accountIndex(q: URLSearchParams): Promise<Response | null> {
    const res = this.data.route('/backup/index', 'GET', undefined, q);
    if (!res) return null;
    const answer = await res;
    if (!answer.ok) return answer;
    const ix = (await answer.json()) as Record<string, unknown>;
    const v = await this.backupView();
    const set = accountSets(v).find((s) => s.account === ix['account'])?.name;
    const mine = set ? (this.sets.get(key(BACKUP_STORAGE, set)) ?? []) : [];
    if (!mine.length) return json(ix);
    const listed = Array.isArray(ix['sessions'])
      ? (ix['sessions'] as Record<string, unknown>[])
      : [];
    const byId = new Map(mine.map((e) => [e.id, e]));
    const sessions = [
      ...mine.filter((e) => !listed.some((s) => s['id'] === e.id)).map((e) => this.indexItem(e)),
      ...listed.map((s) => {
        const e = byId.get(String(s['id']));
        return e?.archived ? { ...s, archived: e.archived } : s;
      }),
    ];
    return json({ ...ix, sessions });
  }

  private indexItem(e: Entry): Record<string, unknown> {
    const here = this.isLocal(e.id);
    return {
      id: e.id,
      action: here ? 'same' : 'new',
      cwd: e.cwd,
      title: e.title,
      remoteBytes: e.bytes,
      ...(here ? { localBytes: e.bytes } : {}),
      host: e.host,
      pushedAt: e.pushedAt,
      newestMtime: e.newestMtime,
      ...(e.archived ? { archived: e.archived } : {}),
    };
  }

  /** Some account on this Mac has the conversation. */
  private isLocal(id: string): boolean {
    if (this.gone.has(id)) return false;
    return (
      this.host.sessions().some((s) => s.id === id) ||
      this.host.accounts().some((a) => this.data.hasCopy(id, a.id))
    );
  }

  // ─────────────────────────────────────────────────────────── Moves

  private async move(b: Record<string, unknown>): Promise<Response> {
    const mode = b['mode'];
    if (mode !== 'move' && mode !== 'copy') {
      return fail(422, 'invalid_transfer', 'Send mode ("move" or "copy").');
    }
    const items = obj(b['items']) ?? {};
    const ids = strings(items['sessions']);
    const projects = strings(items['projects']).map((p) => p.replace(/\/+$/, '') || '/');
    const all = items['all'] === true;
    const custom = strings(items['customizations']);
    if (!ids.length && !projects.length && !all && !custom.length) {
      return fail(
        422,
        'invalid_transfer',
        'Name what goes: items.sessions, projects, all or customizations.',
      );
    }
    const unknownItem = custom.find((c) => !CUSTOMIZATION_ITEMS.includes(c));
    if (unknownItem) {
      return fail(
        422,
        'invalid_transfer',
        `there is no customization ${JSON.stringify(unknownItem)}`,
      );
    }
    let to = readLocation(b['to']);
    if (!to) return fail(422, 'invalid_transfer', 'Send to.');
    const from = b['from'] === undefined ? null : readLocation(b['from']);
    if (b['from'] !== undefined && !from)
      return fail(422, 'invalid_transfer', 'from is not a place.');
    if (!from && (projects.length || all || custom.length)) {
      return fail(422, 'invalid_transfer', 'from may be left out only for items.sessions');
    }
    const v = await this.backupView();
    for (const l of [from, to]) {
      if (l?.kind === 'account' && !this.account(l.account)) {
        return fail(404, 'not_found', `no account ${JSON.stringify(l.account)}`);
      }
      if (l?.kind === 'storage') {
        const pre = this.prerequisite(v, l.storage);
        if (pre) return pre;
      }
    }
    // A storage target's set: the source account's directory's, by default.
    if (to.kind === 'storage' && !to.set) {
      const owner = this.account(
        from?.kind === 'account' ? this.homeOf(from.account) : DEFAULT_ACCOUNT,
      );
      to = { ...to, set: owner?.name ?? 'Default' };
    }
    if (from && sameLocation(from, to)) {
      return fail(422, 'invalid_transfer', 'from and to are the same place');
    }
    const trust = b['trust'] === true;
    const replace = b['replaceExisting'] === true;
    if (this.host.role !== 'desktop' && (trust || custom.some((c) => DANGEROUS_ITEMS.has(c)))) {
      return fail(
        403,
        'dangerous_blocked',
        "a folder's trust, settings and MCP servers need dangerous actions for this device",
      );
    }
    if (custom.length) {
      if (mode === 'move')
        return fail(422, 'invalid_transfer', 'customizations are only ever copied');
      if (
        from?.kind !== 'account' ||
        to.kind !== 'account' ||
        this.homeOf(from.account) === this.homeOf(to.account)
      ) {
        return fail(
          422,
          'invalid_transfer',
          'customizations are copied only between accounts in different directories',
        );
      }
    }
    if (
      mode === 'copy' &&
      from?.kind === 'account' &&
      to.kind === 'account' &&
      this.homeOf(from.account) === this.homeOf(to.account)
    ) {
      return fail(
        422,
        'same_directory',
        "both accounts use the shared folder: a copy there would be the same conversation; move it instead, which records it as the other account's",
      );
    }
    for (const l of [from, to]) {
      if (l?.kind !== 'storage') continue;
      const refused = this.setRefusal(v, l.storage, l.set);
      if (refused) return refused;
    }

    // What goes, and what does not.
    const skipped: Skip[] = [];
    const candidates = await this.candidates(v, from, ids, projects, all, skipped);
    if (candidates instanceof Response) return candidates;
    const target = to;
    const targetEntries =
      target.kind === 'storage' ? ((await this.entries(v, target.storage, target.set)) ?? []) : [];
    const planned: Planned[] = [];
    for (const c of candidates) {
      const direction = this.direction(c.source, target);
      if (c.live) {
        skipped.push({
          id: c.id,
          code: 'session_live',
          message: `session ${c.id} is running; stop it first`,
        });
      } else if (direction === 'assign' && mode === 'copy') {
        skipped.push({
          id: c.id,
          code: 'same_directory',
          message: `it is in the folder ${this.name(target)} uses already`,
        });
      } else if (this.targetHas(c, target, direction, targetEntries)) {
        skipped.push({
          id: c.id,
          code: 'already_exists',
          message:
            target.kind === 'storage'
              ? 'the set has it already'
              : `${this.name(target)} has it already`,
        });
      } else {
        planned.push({ c, direction, action: action(direction, mode) });
      }
    }
    const customs = custom.length ? this.customizations(custom, replace) : [];
    const intoAccount = target.kind === 'account';
    const cwds = [...new Set(planned.map((p) => p.c.cwd).filter(Boolean))];
    const writesShared = intoAccount && (trust || projects.length > 0 || customs.length > 0);
    if (writesShared && this.liveIn(this.homeOf((target as { account: string }).account))) {
      const a = this.account((target as { account: string }).account);
      return fail(
        409,
        'directory_in_use',
        `a Claude Code process is using ${this.dirOf(a)}; quit its sessions first`,
      );
    }
    if (writesShared && TRANSFER_MODE === 'locked') {
      return fail(
        409,
        'config_locked',
        'Claude Code is writing its settings right now; try again in a few seconds',
      );
    }
    const warnings: string[] = [];
    if (custom.includes('settings')) {
      warnings.push(
        `The copied settings.json runs commands (hooks, statusLine) and its env sets ANTHROPIC_API_KEY, which chooses how Claude signs in: look through it in ${this.name(target)} first.`,
      );
    }
    if (custom.includes('mcpServers')) {
      warnings.push('MCP servers are named only: their env and headers values are never shown.');
    }
    for (const p of planned) {
      if (
        p.direction === 'fromStorage' &&
        p.c.entry &&
        p.c.entry.newestMtime < Date.now() - 30 * D
      ) {
        warnings.push(
          `${p.c.title || p.c.id}: Claude Code will delete it again at its next cleanup unless cleanupPeriodDays (now 30) is raised.`,
        );
      }
    }
    const sources = new Map<string, Planned[]>();
    for (const p of planned) {
      const k = locKey(p.c.source);
      sources.set(k, [...(sources.get(k) ?? []), p]);
    }
    if (!sources.size && from) sources.set(locKey(from), []);
    const files = planned.length * 3 + customs.filter((c) => c.action !== 'skip').length;
    const bytes = planned.reduce((n, p) => n + p.c.bytes + 3_620, 0);

    if (b['dryRun'] === true) {
      return json({
        plan: {
          ...(from ? { from: wire(from) } : {}),
          to: wire(target),
          mode,
          jobs: Math.max(1, sources.size),
          sessions: planned.map((p) => ({
            id: p.c.id,
            cwd: p.c.cwd,
            title: p.c.title,
            files: 3,
            bytes: p.c.bytes + 3_620,
            from: wire(p.c.source),
            action: p.action,
          })),
          skipped,
          projects: intoAccount ? projects : [],
          addsProjectEntries: intoAccount && trust ? (projects.length ? projects : cwds) : [],
          removesProjectEntries:
            intoAccount && trust && mode === 'move' && from?.kind === 'account' ? projects : [],
          historyLines: intoAccount && projects.length ? 3 + planned.length * 4 : 0,
          customizations: customs,
          warnings,
          files,
          bytes,
          writesShared,
        },
      });
    }
    if (TRANSFER_MODE === 'nodata') {
      return fail(409, 'no_data_dir', 'This build has no data directory to keep an undo in.');
    }
    const jobs = [...sources.entries()].map(([, group], i) =>
      this.runGroup(group, i === 0 ? customs : [], i === 0 ? skipped : [], from, target, mode),
    );
    return json({ jobs, job: jobs[0] }, 202);
  }

  /** The conversations a request names, from `from` (each from where it is
   * when it is left out); what can't be found goes into `skipped`. */
  private async candidates(
    v: Record<string, unknown>,
    from: DataLocation | null,
    ids: string[],
    projects: string[],
    all: boolean,
    skipped: Skip[],
  ): Promise<Candidate[] | Response> {
    if (from?.kind === 'storage') {
      const entries = await this.entries(v, from.storage, from.set);
      if (!entries) return fail(422, 'invalid_backup', `there is no set ${from.set} there`);
      const pick = all
        ? entries
        : entries.filter((e) => ids.includes(e.id) || projects.includes(e.cwd));
      for (const id of ids) {
        if (!entries.some((e) => e.id === id)) {
          skipped.push({ id, code: 'not_found', message: `the set has no conversation ${id}` });
        }
      }
      return pick.map((e) => ({
        id: e.id,
        cwd: e.cwd,
        title: e.title,
        bytes: e.bytes,
        live: false,
        source: from,
        entry: e,
      }));
    }
    const here = (s: SessionSummary): Candidate => ({
      id: s.id,
      cwd: s.cwd,
      title: s.title ?? s.name ?? '',
      bytes: 40_000 + s.messageCount * 2_600,
      live: s.live,
      source: { kind: 'account', account: this.host.accountOf(s.id) },
    });
    const known = this.host.sessions().filter((s) => !this.gone.has(s.id));
    if (!from) {
      const out: Candidate[] = [];
      for (const id of ids) {
        const s = known.find((x) => x.id === id);
        if (s) out.push(here(s));
        else
          skipped.push({ id, code: 'not_found', message: `there is no conversation ${id} here` });
      }
      return out;
    }
    const mine = known.filter((s) => this.host.accountOf(s.id) === from.account);
    for (const id of ids) {
      if (!mine.some((s) => s.id === id)) {
        skipped.push({
          id,
          code: 'not_found',
          message: `${this.name(from)} has no conversation ${id}`,
        });
      }
    }
    return mine.filter((s) => all || ids.includes(s.id) || projects.includes(s.cwd)).map(here);
  }

  private direction(from: DataLocation, to: DataLocation): Direction {
    if (from.kind === 'account' && to.kind === 'account') {
      return this.homeOf(from.account) === this.homeOf(to.account) ? 'assign' : 'between';
    }
    if (from.kind === 'account') return 'toStorage';
    return to.kind === 'account' ? 'fromStorage' : 'storages';
  }

  private targetHas(c: Candidate, to: DataLocation, dir: Direction, entries: Entry[]): boolean {
    if (to.kind === 'storage') return entries.some((e) => e.id === c.id && !e.archived);
    if (dir === 'assign') return this.host.accountOf(c.id) === to.account;
    if (dir === 'between') return this.data.hasCopy(c.id, to.account);
    // Out of a set: the account's directory has it already.
    return (
      this.data.hasCopy(c.id, to.account) ||
      this.host
        .sessions()
        .some(
          (s) =>
            s.id === c.id &&
            !this.gone.has(s.id) &&
            this.homeOf(this.host.accountOf(s.id)) === this.homeOf(to.account),
        )
    );
  }

  /** What copying each category would do (made up, as the demo's). */
  private customizations(items: string[], replace: boolean): Custom[] {
    const has = (name: string, reason = 'the target has its own'): Custom =>
      replace ? { item: '', name, action: 'replace' } : { item: '', name, action: 'skip', reason };
    const out: Custom[] = [];
    for (const item of items) {
      const add = (list: Custom[]) => out.push(...list.map((c) => ({ ...c, item })));
      switch (item) {
        case 'memory':
          add([has('CLAUDE.md')]);
          break;
        case 'settings':
          add([has('settings.json')]);
          break;
        case 'skills':
          add([
            { item, name: 'review', action: 'add' },
            { item, name: 'release-notes', action: 'add' },
            has('commit'),
          ]);
          break;
        case 'agents':
          add([{ item, name: 'code-reviewer', action: 'add' }]);
          break;
        case 'commands':
          add([{ item, name: 'ship', action: 'add' }]);
          break;
        case 'outputStyles':
          add([{ item, name: 'terse', action: 'add' }]);
          break;
        case 'keybindings':
          add([{ item, name: 'keybindings.json', action: 'add' }]);
          break;
        case 'mcpServers':
          add([
            { item, name: 'github', action: 'merge' },
            { item, name: 'linear', action: 'add' },
          ]);
          break;
        default:
          add([{ item, name: 'history.jsonl', action: 'merge' }]);
      }
    }
    return out;
  }

  /** One job: what goes from one source, done after a while; Undo puts it
   * back (a copy into a storage is a backup: nothing to undo). */
  private runGroup(
    group: Planned[],
    customs: Custom[],
    skipped: Skip[],
    from: DataLocation | null,
    to: DataLocation,
    mode: 'move' | 'copy',
  ): JobView {
    const source = group[0]?.c.source ?? from ?? to;
    const names: JobNames = {
      ...(source.kind === 'account' ? { from: source.account } : { fromStorage: source.storage }),
      ...(to.kind === 'account' ? { to: to.account } : { toStorage: to.storage }),
      ...(source.kind === 'storage'
        ? { set: source.set }
        : to.kind === 'storage'
          ? { set: to.set }
          : {}),
    };
    const ids = group.map((p) => p.c.id);
    const files = group.length * 3 + customs.length;
    const bytes = group.reduce((n, p) => n + p.c.bytes + 3_620, 0);
    return this.data.runJob(mode, names, files, bytes, ids, 1_600, () =>
      this.apply(group, customs, skipped, to, mode),
    );
  }

  private apply(
    group: Planned[],
    customs: Custom[],
    skipped: Skip[],
    to: DataLocation,
    mode: 'move' | 'copy',
  ): JobEnd {
    const undo: (() => void)[] = [];
    const done = {
      moved: [] as string[],
      copied: [] as string[],
      assigned: [] as string[],
      archived: [] as string[],
      restored: [] as string[],
    };
    const now = Date.now();
    for (const { c, direction, action: what } of group) {
      const before = this.host.accountOf(c.id);
      if (direction === 'between' || direction === 'assign') {
        if (mode === 'move') {
          this.host.reassign(c.id, (to as { account: string }).account);
          undo.push(() => this.host.reassign(c.id, before));
        } else {
          this.data.copyForContinue(c.id, (to as { account: string }).account);
          undo.push(() => this.data.dropCopy(c.id, (to as { account: string }).account));
        }
      } else if (direction === 'toStorage') {
        const t = to as { storage: string; set: string };
        const entry: Entry = {
          id: c.id,
          cwd: c.cwd,
          title: c.title,
          bytes: c.bytes,
          pushedAt: now,
          newestMtime: now - H,
          host: HOST,
          ...(mode === 'move' ? { archived: { at: now, machine: THIS_MACHINE } } : {}),
        };
        const list = (this.sets.get(key(t.storage, t.set)) ?? []).filter((e) => e.id !== c.id);
        this.sets.set(key(t.storage, t.set), [entry, ...list]);
        if (mode === 'move') {
          this.gone.set(c.id, before);
          undo.push(() => {
            this.gone.delete(c.id);
            // The set keeps its copy; the archived mark is cleared.
            const now2 = this.sets.get(key(t.storage, t.set)) ?? [];
            this.sets.set(
              key(t.storage, t.set),
              now2.map((e) => (e.id === c.id ? { ...e, archived: undefined } : e)),
            );
          });
        }
      } else {
        // Out of a set: into an account (a restore), or into another set.
        const s = c.source as { storage: string; set: string };
        if (direction === 'fromStorage') {
          const account = (to as { account: string }).account;
          const left = this.gone.get(c.id);
          if (left) this.gone.delete(c.id);
          this.data.copyForContinue(c.id, account);
          undo.push(() => {
            this.data.dropCopy(c.id, account);
            if (left) this.gone.set(c.id, left);
          });
        } else {
          const t = to as { storage: string; set: string };
          const list = this.sets.get(key(t.storage, t.set)) ?? [];
          this.sets.set(key(t.storage, t.set), [{ ...c.entry!, archived: undefined }, ...list]);
          undo.push(() =>
            this.sets.set(
              key(t.storage, t.set),
              (this.sets.get(key(t.storage, t.set)) ?? []).filter((e) => e.id !== c.id),
            ),
          );
        }
        if (mode === 'move' && c.entry) {
          const k = key(s.storage, s.set);
          const entry = c.entry;
          this.sets.set(
            k,
            (this.sets.get(k) ?? []).filter((e) => e.id !== c.id),
          );
          undo.push(() => this.sets.set(k, [entry, ...(this.sets.get(k) ?? [])]));
        }
      }
      const bucket =
        what === 'move'
          ? done.moved
          : what === 'copy'
            ? done.copied
            : what === 'assign'
              ? done.assigned
              : what === 'archive'
                ? done.archived
                : done.restored;
      bucket.push(c.id);
    }
    const ids = group.map((p) => p.c.id);
    const backupOnly = to.kind === 'storage' && mode === 'copy';
    const changed = group.length > 0 || customs.some((c) => c.action !== 'skip');
    return {
      result: {
        ...Object.fromEntries(Object.entries(done).filter(([, list]) => list.length)),
        skipped,
        customizations: customs,
      },
      sessions: ids,
      undo: changed && !backupOnly,
      onUndo: () => undo.reverse().forEach((fn) => fn()),
    };
  }

  // ───────────────────────────────────────────────────────── Helpers

  private account(id: string): AccountView | undefined {
    return this.host.accounts().find((a) => a.id === id);
  }

  private homeOf(id: string): string {
    const a = this.account(id);
    return a ? homeOf(a) : DEFAULT_ACCOUNT;
  }

  private dirOf(a: AccountView | undefined): string {
    return a?.kind === 'directory' ? (a.dir ?? SHARED_DIR) : SHARED_DIR;
  }

  private name(l: DataLocation): string {
    if (l.kind === 'account') return this.account(l.account)?.name ?? l.account;
    const st = this.stores.find((s) => s.id === l.storage);
    return `${st?.label ?? (l.storage === BACKUP_STORAGE ? 'the backup folder' : 'the storage')} · ${l.set}`;
  }

  private liveIn(home: string): boolean {
    return this.host
      .sessions()
      .some((s) => s.live && this.homeOf(this.host.accountOf(s.id)) === home);
  }
}

function action(d: Direction, mode: 'move' | 'copy'): Planned['action'] {
  switch (d) {
    case 'assign':
      return 'assign';
    case 'toStorage':
      return mode === 'move' ? 'archive' : 'copy';
    case 'fromStorage':
      return 'restore';
    default:
      return mode;
  }
}

function sameLocation(a: DataLocation, b: DataLocation): boolean {
  return locKey(a) === locKey(b);
}

function locKey(l: DataLocation): string {
  return l.kind === 'account' ? `a:${l.account}` : `s:${l.storage}:${l.set}`;
}

function wire(l: DataLocation): unknown {
  return l.kind === 'account' ? l.account : { storage: l.storage, set: l.set };
}

function key(storage: string, set: string): string {
  return `${storage}\u0000${set}`;
}

function folderOf(v: Record<string, unknown>): string | null {
  const f = obj(v['settings'])?.['folder'];
  return typeof f === 'string' && f ? f : null;
}

/** The sets of the accounts' own directories in the data mock's view. */
function accountSets(
  v: Record<string, unknown>,
): { account: string; name: string; exists: boolean; keyMissing: boolean; sessions?: number }[] {
  return (Array.isArray(v['sets']) ? v['sets'] : [])
    .map(obj)
    .filter((s): s is Record<string, unknown> => !!s && !s['sharedWith'])
    .map((s) => ({
      account: String(s['account']),
      name: String(s['name']),
      exists: s['exists'] === true,
      keyMissing: s['keyMissing'] === true,
      ...(typeof s['sessions'] === 'number' ? { sessions: s['sessions'] } : {}),
    }));
}

function within(path: string, dir: string): boolean {
  const d = dir.replace(/\/+$/, '');
  return path === d || path.startsWith(`${d}/`);
}

function obj(v: unknown): Record<string, unknown> | null {
  return v && typeof v === 'object' && !Array.isArray(v) ? (v as Record<string, unknown>) : null;
}

function strings(v: unknown): string[] {
  return Array.isArray(v) ? v.filter((x): x is string => typeof x === 'string' && !!x) : [];
}

function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

function fail(status: number, code: string, message: string): Response {
  return json({ error: { code, message } }, status);
}

function hex(n: number): string {
  let out = '';
  for (let i = 0; i < n; i++) out += Math.floor(Math.random() * 16).toString(16);
  return out;
}
