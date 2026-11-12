import { KeptCopy } from './copies';
import {
  MachineAsk,
  MachinesView,
  PeerMachine,
  Revocation,
  SetupCheck,
  SetupView,
  SyncPair,
  VersionOrder,
  healthAlarm,
} from './machines';
import { AccountView, JobView, PermissionInfo } from './models';
import { accountSigningIn } from './sign-in';

/**
 * What the Inbox's *To fix* lists (docs/UI-REDESIGN.md §6.4): something
 * stopped working, or works worse, each with its fix — a missing macOS
 * permission, a computer's failing required check or its versions apart, a
 * project changed on both computers, a copy paused for a reason someone
 * can end, a backup or move that failed, a sign-in lost or as someone
 * else, a computer revoked everywhere that some of yours still trust, an
 * update waiting. Pure: `AttentionService` gathers the sources
 * from the services that already read them.
 */

/** How long a failed job, or a failed ask, stays to fix. */
export const FAILED_WITHIN_MS = 24 * 3600_000;

/** The pauses of a copy someone can end; the others pass by themselves
 * (offline, changed while read, lives elsewhere now) or are no one's to
 * fix (an error, said on Computers). */
const FIXABLE_COPY: ReadonlySet<string> = new Set([
  'noGrant',
  'machinesOff',
  'tooLarge',
  'unsupported',
]);

/** ✕ it blocks · ! it works, but worse · ● news. */
export type FixTone = 'danger' | 'warn' | 'info';

interface FixBase {
  /** Unique among the items, and the same while it lasts. */
  id: string;
  tone: FixTone;
  /** The computer it is about (null: this one). */
  machine: string | null;
  /** That computer's name, while it is another one. */
  machineName?: string;
  /** The account it is about. */
  account?: string;
  /** When it happened (ms), when known. */
  at?: number;
}

export type FixItem = FixBase &
  (
    | { kind: 'permission'; permission: PermissionInfo }
    /** A required check failing on this computer or another. */
    | { kind: 'check'; check: SetupCheck; remote: boolean }
    /** Machines' health alarm before its checks are read. */
    | { kind: 'health'; blockers: number }
    | {
        kind: 'signIn';
        accountView: AccountView;
        problem: 'signedOut' | 'authFailed' | 'tokenMissing' | 'identity';
      }
    | { kind: 'skew'; peer: PeerMachine; what: 'app' | 'claude'; order: VersionOrder }
    | { kind: 'sync'; pair: SyncPair }
    | { kind: 'copy'; copy: KeptCopy }
    | { kind: 'job'; job: JobView }
    /** Another computer's ask that ran here and failed. */
    | { kind: 'ask'; ask: MachineAsk }
    /** A computer revoked everywhere, still to be told to some of yours. */
    | { kind: 'revoked'; revocation: Revocation }
    | { kind: 'update'; version: string }
  );

/** What the items are read from. */
export interface FixSources {
  /** This client is the desktop app (permissions and updates are its). */
  desktop: boolean;
  now: number;
  permissions: readonly PermissionInfo[];
  machines: MachinesView | null;
  setup: SetupView | null;
  accounts: readonly AccountView[];
  sync: readonly SyncPair[];
  kept: readonly KeptCopy[];
  jobs: readonly JobView[];
  /** The version of an update waiting to be installed (null: none). */
  update: string | null;
  /** Items this device put away (failed jobs and asks). */
  dismissed: ReadonlySet<string>;
}

/** Everything to fix, the blocking first. */
export function fixItems(s: FixSources): FixItem[] {
  return [
    ...permissionItems(s),
    ...healthItems(s),
    ...signInItems(s),
    ...skewItems(s),
    ...syncItems(s),
    ...copyItems(s),
    ...jobItems(s),
    ...askItems(s),
    ...revokedItems(s),
    ...updateItems(s),
  ];
}

/** A required macOS permission missing (the desktop). */
function permissionItems(s: FixSources): FixItem[] {
  if (!s.desktop) return [];
  return s.permissions
    .filter((p) => p.required && p.granted === false)
    .map((p) => ({
      kind: 'permission',
      id: `permission:${p.id}`,
      tone: 'danger',
      machine: null,
      permission: p,
    }));
}

/**
 * The required checks that fail while Machines is on or being set up (its
 * health dot): this computer's — but its accounts' sign-ins, which the
 * accounts say themselves — and every other computer's. One item says so
 * until the checks are read.
 */
function healthItems(s: FixSources): FixItem[] {
  const v = s.machines;
  if (!healthAlarm(v)) return [];
  const out: FixItem[] = [];
  for (const c of s.setup?.self.checks ?? []) {
    if (c.state !== 'blocker' || c.scope === 'account') continue;
    out.push({
      kind: 'check',
      id: `check:self:${c.id}`,
      tone: 'danger',
      machine: null,
      check: c,
      remote: false,
      at: s.setup?.self.at ?? undefined,
    });
  }
  for (const pc of s.setup?.peers ?? []) {
    for (const c of pc.checks) {
      if (c.state !== 'blocker') continue;
      out.push({
        kind: 'check',
        id: `check:${pc.machine}:${c.id}`,
        tone: 'danger',
        machine: pc.machine,
        machineName: pc.name,
        check: c,
        remote: true,
        at: pc.at ?? undefined,
      });
    }
  }
  if (out.length || !v) return out;
  // Only this computer's account checks fail: the accounts say it.
  if (s.setup?.self.checks.some((c) => c.state === 'blocker')) return out;
  return [
    { kind: 'health', id: 'health', tone: 'danger', machine: null, blockers: v.health.blockers },
  ];
}

/** An account signed out, refused, without its token here, or signed in
 * as someone else (none while its sign-in runs). */
function signInItems(s: FixSources): FixItem[] {
  const out: FixItem[] = [];
  for (const a of s.accounts) {
    if (accountSigningIn(a)) continue;
    const base = { id: `signin:${a.id}`, machine: null, account: a.id, accountView: a } as const;
    if (a.identityWarning) {
      out.push({ ...base, kind: 'signIn', tone: 'warn', problem: 'identity' });
    } else if (
      a.attention === 'signedOut' ||
      a.attention === 'authFailed' ||
      a.attention === 'tokenMissing'
    ) {
      out.push({ ...base, kind: 'signIn', tone: 'danger', problem: a.attention });
    }
  }
  return out;
}

/** A paired computer whose Mother Claude or Claude Code is apart from
 * this one's (an older server only says `problem`). */
function skewItems(s: FixSources): FixItem[] {
  const out: FixItem[] = [];
  for (const p of s.machines?.peers ?? []) {
    const app =
      p.skew?.app ??
      (p.problem === 'olderApp' ? 'older' : p.problem === 'newerApp' ? 'newer' : undefined);
    const sides: ['app' | 'claude', VersionOrder | undefined][] = [
      ['app', app],
      ['claude', p.skew?.claude],
    ];
    for (const [what, order] of sides) {
      if (!order) continue;
      out.push({
        kind: 'skew',
        id: `skew:${p.id}:${what}`,
        tone: 'warn',
        machine: p.id,
        machineName: p.name,
        peer: p,
        what,
        order,
      });
    }
  }
  return out;
}

/** A project changed on both computers: nothing syncs until one is kept. */
function syncItems(s: FixSources): FixItem[] {
  return s.sync
    .filter((p) => p.status.state === 'paused' && p.status.reason === 'conflict')
    .map((p) => ({
      kind: 'sync',
      id: `sync:${p.id}`,
      tone: 'warn',
      machine: p.machine,
      machineName: p.machineName,
      pair: p,
      at: p.conflict?.at ?? (p.status.state === 'paused' ? p.status.since : undefined),
    }));
}

/** A copy kept on another computer, paused for a reason someone can end. */
function copyItems(s: FixSources): FixItem[] {
  return s.kept
    .filter((k) => k.status?.state === 'paused' && FIXABLE_COPY.has(k.status.reason ?? ''))
    .map((k) => ({
      kind: 'copy',
      id: `copy:${k.id}`,
      tone: 'warn',
      machine: k.machine,
      machineName: k.machineName,
      copy: k,
      at: k.status?.since,
    }));
}

/** A backup, restore, pull, move, copy or delete that failed in the last
 * day, unless the same one ran through since or this device put it away. */
function jobItems(s: FixSources): FixItem[] {
  return s.jobs
    .filter(
      (j) =>
        j.state === 'failed' &&
        s.now - (j.finishedAt ?? j.startedAt) < FAILED_WITHIN_MS &&
        !s.dismissed.has(`job:${j.id}`) &&
        !s.jobs.some(
          (k) => k !== j && k.state === 'done' && sameJob(j, k) && k.startedAt > j.startedAt,
        ),
    )
    .sort((a, b) => (a.finishedAt ?? a.startedAt) - (b.finishedAt ?? b.startedAt))
    .map((j) => ({
      kind: 'job',
      id: `job:${j.id}`,
      tone: 'warn',
      machine: null,
      account: j.account ?? j.from ?? j.to,
      job: j,
      at: j.finishedAt ?? j.startedAt,
    }));
}

/** Two jobs do the same: the same kind, between the same places. */
function sameJob(a: JobView, b: JobView): boolean {
  return (
    a.kind === b.kind &&
    a.account === b.account &&
    a.from === b.from &&
    a.to === b.to &&
    a.fromStorage === b.fromStorage &&
    a.toStorage === b.toStorage
  );
}

/** What another computer asked, confirmed here, that failed lately. */
function askItems(s: FixSources): FixItem[] {
  return (s.machines?.asks ?? [])
    .filter(
      (a) =>
        a.state === 'failed' &&
        s.now - (a.answeredAt ?? a.at) < FAILED_WITHIN_MS &&
        !s.dismissed.has(`ask:${a.id}`),
    )
    .map((a) => ({
      kind: 'ask',
      id: `ask:${a.id}`,
      tone: 'warn',
      machine: a.from,
      machineName: a.fromName,
      ask: a,
      at: a.answeredAt ?? a.at,
    }));
}

/** A computer revoked everywhere (*Revoke everywhere*) that some of your
 * other computers still have to be told of — offline, or too old to take
 * it: they let it in until they are. It goes once each one took it, or when
 * this device puts it away. */
function revokedItems(s: FixSources): FixItem[] {
  return (s.machines?.revocations ?? [])
    .filter((r) => r.pending.length > 0 && !s.dismissed.has(`revoked:${r.machine}`))
    .map((r) => ({
      kind: 'revoked',
      id: `revoked:${r.machine}`,
      tone: 'warn',
      machine: null,
      revocation: r,
      at: r.at,
    }));
}

/** A new version of the app, ready to install (the desktop). */
function updateItems(s: FixSources): FixItem[] {
  if (!s.desktop || s.update === null) return [];
  return [{ kind: 'update', id: 'update', tone: 'info', machine: null, version: s.update }];
}
