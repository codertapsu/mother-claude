// Machines: one Mother Claude across several of your computers
// (docs/PLAN-MACHINES.md). The wire types of `/api/machines*` and the
// `machines` frame, as `src-tauri/src/machines/` sends them, plus the small
// helpers every Machines screen shares. Labels here are English; they are
// translated where they are shown.

export type MachineOs = 'macos' | 'windows' | 'linux' | 'other';

export type GrantName = 'view' | 'control' | 'move' | 'run' | 'dangerous';

/** A grant on the local network (`lan`) and over the internet. */
export interface GrantPair {
  lan: boolean;
  internet: boolean;
}

/** What one computer lets another do on it. */
export type Grants = Record<GrantName, GrantPair>;

/** How a connection to another computer runs: direct on the local network,
 * direct over the internet, or through the relay. */
export type MachinePath = 'lan' | 'internet' | 'relay';

export interface MachineVersions {
  app: string;
  protocol: number;
  claude: string | null;
}

/** This computer, as `GET /api/machines` describes it. */
export interface ThisMachine {
  id: string;
  name: string;
  os: MachineOs | string;
  osVersion: string | null;
  versions: MachineVersions;
  /** The key's fingerprint, `3F9A C1D2 77E0 9B41`. */
  key: string;
}

export type RelayMode = 'public' | 'custom' | 'none';

export interface RelaySetting {
  mode: RelayMode;
  url: string | null;
}

/** A paired computer. */
export interface PeerMachine {
  id: string;
  name: string;
  os: MachineOs | string;
  osVersion: string | null;
  versions: MachineVersions | null;
  key: string;
  pairedAt: number;
  lastSeen: number | null;
  online: boolean;
  path: MachinePath | null;
  rttMs: number | null;
  /** What it may do on this computer. */
  grants: Grants;
  /** What this computer may do on it, as it last said. */
  theirGrants: Grants | null;
  capabilities: string[];
  relay: RelaySetting | null;
  problem: 'olderApp' | 'newerApp' | null;
  /** Its sessions as this computer last heard them (null: none heard yet;
   * absent from an older server). */
  sessions?: PeerSessions | null;
}

/** How many of a paired computer's sessions this one knows, and how many
 * run. */
export interface PeerSessions {
  total: number;
  running: number;
}

export type PairingRole = 'host' | 'joiner';

export type PairingState =
  'starting' | 'waiting' | 'joining' | 'confirm' | 'waitingOther' | 'paired' | 'failed';

/** The one pairing this computer runs, as host (it shows the code) or
 * joiner (it typed the code). */
export interface PairingView {
  role: PairingRole;
  state: PairingState;
  /** `K7QM-4R2X`, while the host shows it. */
  code: string | null;
  expiresAt: number | null;
  other: { id: string; name: string; os: MachineOs | string; path: MachinePath } | null;
  /** The six digits both screens compare. */
  digits: string | null;
  /** Host: the joiner came over the internet, so its digits are typed. */
  mustType: boolean;
  confirmed: boolean;
  otherConfirmed: boolean;
  error: { code: string; message: string } | null;
  peerId: string | null;
}

export interface MachinesSettings {
  relay: RelaySetting;
  keepAwake: boolean;
}

/** Another computer suggests its relay setting. */
export interface SettingsOffer {
  from: string;
  fromName: string;
  relay: RelaySetting;
  at: number;
}

export type HealthState = 'ok' | 'warning' | 'blocker' | 'unknown';

export interface MachinesHealth {
  state: HealthState;
  blockers: number;
  warnings: number;
  at: number | null;
}

/** `GET /api/machines`. */
export interface MachinesView {
  enabled: boolean;
  self: ThisMachine;
  peers: PeerMachine[];
  pairing: PairingView | null;
  settings: MachinesSettings;
  offer: SettingsOffer | null;
  transport: { running: boolean; relay: { url: string; connected: boolean } | null };
  health: MachinesHealth;
  /** The setup step left at (1–7), null when never started or finished. */
  setupStep: number | null;
}

export type CheckState = 'ok' | 'warning' | 'blocker' | 'unknown' | 'na';
export type FixKind = 'auto' | 'settings' | 'admin' | 'manual';

/** One of the setup's checks (plan §5.14). */
export interface SetupCheck {
  id: string;
  area: string;
  title: string;
  state: CheckState;
  detail: string;
  why: string;
  severity: 'required' | 'recommended';
  fix: { kind: FixKind; label: string } | null;
}

/** One computer's checks. */
export interface MachineChecks {
  machine: string;
  name: string;
  at: number | null;
  checks: SetupCheck[];
  error: string | null;
}

export interface TestLine {
  id: 'connect' | 'transfer' | string;
  state: 'ok' | 'failed' | 'skipped' | string;
  detail: string;
  ms: number | null;
  mibps: number | null;
}

/** The readiness test with one computer. */
export interface Readiness {
  machine: string;
  name: string;
  tests: TestLine[];
  at: number;
}

/** `GET /api/machines/setup`. */
export interface SetupView {
  self: MachineChecks;
  peers: MachineChecks[];
  blockers: number;
  warnings: number;
  tests: Readiness[];
}

export interface NearbyMachine {
  id: string;
  name: string;
  os: MachineOs | string;
}

/** A required check that keeps Machines off (`409 machines_blocked`). */
export interface Blocker {
  id: string;
  title: string;
  detail: string;
}

/** `{t:'machines', what}`: refetch `/api/machines` (and the setup). */
export interface MachinesFrame {
  t: 'machines';
  what: string;
}

/**
 * `{t:'remote', machine, frame}`: another computer's frame, republished by
 * this one's link to it (docs/PLAN-MACHINES.md §5.6). Its sessions, pending
 * items and their answers reach everyone; a session's transcript, stream and
 * agents reach whoever subscribed to it (`sub` with `machine`). The link adds
 * its own `online`, `offline{since}` and `forgotten` (revoked).
 */
export interface RemoteFrame {
  t: 'remote';
  machine: string;
  frame: unknown;
}

// ─────────────────────────────────────────────────────────────── Helpers

export const GRANT_NAMES: readonly GrantName[] = ['view', 'control', 'move', 'run', 'dangerous'];

/** Each grant's row in the matrix, and its short name in a summary line. */
export const GRANT_LABELS: Record<GrantName, { row: string; short: string }> = {
  view: { row: 'See sessions, transcripts, requests', short: 'See' },
  control: { row: 'Drive them: send, answer, stop, model', short: 'Drive' },
  move: { row: 'Move sessions here and away', short: 'Move' },
  run: { row: 'Run commands and tasks here', short: 'Run' },
  dangerous: { row: 'Dangerous actions', short: 'Dangerous' },
};

/** Grants a computer gives at pairing: see, drive and move; never run or
 * dangerous (plan §5.4). */
export function defaultGrants(): Grants {
  const on = (): GrantPair => ({ lan: true, internet: true });
  const off = (): GrantPair => ({ lan: false, internet: false });
  return { view: on(), control: on(), move: on(), run: off(), dangerous: off() };
}

export function copyGrants(g: Grants): Grants {
  return {
    view: { ...g.view },
    control: { ...g.control },
    move: { ...g.move },
    run: { ...g.run },
    dangerous: { ...g.dangerous },
  };
}

/** What a grant needs first: driving needs seeing; moving, running and
 * dangerous actions need driving (the server drops a grant without its
 * base: `Grants::normalized`). */
const BASE: Partial<Record<GrantName, GrantName>> = {
  control: 'view',
  move: 'control',
  run: 'control',
  dangerous: 'control',
};

/** `g` with one cell changed, keeping the dependencies: switching a grant on
 * switches its bases on, switching one off switches what needs it off. */
export function setGrant(g: Grants, name: GrantName, column: keyof GrantPair, on: boolean): Grants {
  const next = copyGrants(g);
  next[name][column] = on;
  if (on) {
    for (let b = BASE[name]; b; b = BASE[b]) next[b][column] = true;
  } else {
    for (const other of GRANT_NAMES) {
      for (let b = BASE[other]; b; b = BASE[b]) {
        if (b === name) next[other][column] = false;
      }
    }
  }
  return next;
}

export function sameGrants(a: Grants | null | undefined, b: Grants | null | undefined): boolean {
  if (!a || !b) return a === b;
  return GRANT_NAMES.every((n) => a[n].lan === b[n].lan && a[n].internet === b[n].internet);
}

/** `macOS`, `Windows`, `Linux` (names), else `Other`. */
export function osLabel(os: string): string {
  switch (os) {
    case 'macos':
      return 'macOS';
    case 'windows':
      return 'Windows';
    case 'linux':
      return 'Linux';
    default:
      return 'Other';
  }
}

/** A connection's path in words (English; translate where shown). */
export function pathLabel(path: MachinePath | null | undefined): string {
  switch (path) {
    case 'lan':
      return 'Direct (same network)';
    case 'internet':
      return 'Direct (internet)';
    case 'relay':
      return 'Through the relay (slower)';
    default:
      return '';
  }
}

/** Whether `p` lets this computer do `grant` for the way the two are joined
 * now (the local network, or the internet); null while it hasn't said. */
export function grantedByPeer(p: PeerMachine | null | undefined, grant: GrantName): boolean | null {
  const pair = p?.theirGrants?.[grant];
  if (!p || !pair) return null;
  if (p.path === 'lan') return pair.lan;
  if (p.path === 'internet' || p.path === 'relay') return pair.internet;
  return pair.lan || pair.internet;
}

/** A computer's id, short, until its name is known. */
export function shortMachineId(id: string): string {
  return id.slice(0, 8);
}

/** The pairing has not reached an end yet. */
export function pairingLive(p: PairingView | null | undefined): boolean {
  return !!p && p.state !== 'paired' && p.state !== 'failed';
}

/** `481270` → `481 270`. */
export function groupDigits(d: string | null | undefined): string {
  return (d ?? '').replace(/^(\d{3})(\d{3})$/, '$1 $2');
}

/** Blockers first, then warnings, then the rest (a check list's order). */
export function byUrgency(checks: readonly SetupCheck[]): SetupCheck[] {
  const rank: Record<CheckState, number> = { blocker: 0, warning: 1, unknown: 2, ok: 3, na: 4 };
  return [...checks].sort((a, b) => (rank[a.state] ?? 5) - (rank[b.state] ?? 5));
}

/** The health dot: a required check fails while Machines is on or being set
 * up. */
export function healthAlarm(v: MachinesView | null | undefined): boolean {
  return !!v && v.health.state === 'blocker' && (v.enabled || v.setupStep !== null);
}
