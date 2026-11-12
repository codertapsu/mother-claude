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
  /** How its versions differ from this computer's, and what it can't do
   * until it updates (absent from an older server). */
  skew?: MachineSkew;
  /** The computers it is paired with (their ids), as it last said. */
  pairedWith?: string[];
}

/** Another computer's version against this one's. */
export type VersionOrder = 'older' | 'newer';

/** How a paired computer differs from this one. */
export interface MachineSkew {
  /** Its Mother Claude against this computer's. */
  app?: VersionOrder;
  /** Its Claude Code against this computer's. */
  claude?: VersionOrder;
  /** What this computer does with others that it can't until it updates
   * (capabilities: `moves`, `sync`, `run`, `tasks`, …). */
  lacks?: string[];
}

/** One side of an introduction this computer received: who introduced
 * which computer, and where it stands (docs/PLAN-MACHINES.md §5.12). */
export interface Introduction {
  id: string;
  from: string;
  fromName: string;
  /** `host`: this computer waits for the other; `joiner`: it dials it. */
  role: 'host' | 'joiner';
  other: { id: string; name: string; os: MachineOs | string; key: string };
  at: number;
  expiresAt: number;
  state: 'offered' | 'accepted' | 'declined' | 'withdrawn' | 'expired';
  detail: string | null;
  /** Its pairing runs now. */
  pairing: boolean;
}

/** A computer revoked here and on the others (*Revoke everywhere*). */
export interface Revocation {
  machine: string;
  name: string;
  key: string;
  at: number;
  /** The paired computers still to take it: offline, or too old. */
  pending: { id: string; name: string; online: boolean; tooOld: boolean }[];
  /** Those that took it. */
  done: { id: string; name: string }[];
}

/** `POST /api/machines/introduce`: how each of the two took it. */
export interface IntroduceResult {
  id: string;
  host: IntroduceSide;
  joiner: IntroduceSide;
}

export interface IntroduceSide {
  machine: string;
  name: string;
  /** `already`: the two are paired already; `withdrawn`: taken back
   * because the other could not be told. */
  state: 'offered' | 'already' | 'failed' | 'withdrawn';
  detail: string | null;
}

/** `GET`/`PUT /api/app/autostart`: *Start Mother Claude when you sign in,
 * in the tray*. */
export interface AutostartView {
  /** This system has a login item Mother Claude can set. */
  supported: boolean;
  enabled: boolean;
  /** On, but switched off in the system's own startup apps. */
  overridden: boolean;
  /** This run started at sign-in, in the tray. */
  startedInTray: boolean;
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
  /** An introduced pairing: who introduced which computer (no code). */
  intro?: PairingIntro | null;
  /** Joiner of an introduced pairing: the other computer has not accepted
   * the introduction yet. */
  waitingAccept?: boolean;
}

export interface PairingIntro {
  id: string;
  by: { id: string; name: string };
  other: { id: string; name: string; os: MachineOs | string };
}

/** How a session's code travels when it moves: machine to machine (a git
 * bundle, the default) or through the project's git remote. */
export type CodeTransfer = 'bundle' | 'remote';
/** Where it lands: the project folder when it has no changes of its own
 * (the default), or always a separate worktree. */
export type CodeApply = 'checkout' | 'worktree';

/** The code options every move starts from (D10), the same on every
 * computer. */
export interface CodeSettings {
  transfer: CodeTransfer;
  apply: CodeApply;
  keepRemoteRefs: boolean;
}

export interface MachinesSettings {
  relay: RelaySetting;
  keepAwake: boolean;
  /** Absent from an older server. */
  code?: CodeSettings;
  /** What this computer keeps at most of each other computer's copies
   * (absent from an older server). */
  copiesCapBytes?: number;
}

/** A project's own code options: a field left out follows the defaults. */
export interface CodeOverride {
  transfer?: CodeTransfer;
  apply?: CodeApply;
  keepRemoteRefs?: boolean;
}

/** What to run after a move, per system (asked of Claude in the move
 * note, never run by Mother Claude). */
export interface AfterMove {
  macos?: string[];
  windows?: string[];
  linux?: string[];
}

/** A project's recipe: commands after a move, and the files git ignores
 * that travel with it (setting those needs the dangerous right). */
export interface Recipe {
  afterMove?: AfterMove;
  carryIgnored?: string[];
}

/** One project's own Machines settings. */
export interface ProjectOptions {
  /** `rt_…/<path in the root>`, or the project's folder. */
  key: string;
  /** Its folder on this computer (null: its root is gone). */
  project: string | null;
  root: { id: string; name: string } | null;
  code: CodeOverride;
  recipe: Recipe | null;
  /** The computers its sessions keep a copy on (§5.9). */
  copies?: string[];
  updatedAt: number;
}

/** `GET`/`PUT /api/machines/settings`. */
export interface MachinesSettingsView extends MachinesSettings {
  projects?: ProjectOptions[];
}

/** One project's settings as sent (the whole of them). */
export interface ProjectInput {
  project: string;
  code: CodeOverride;
  recipe?: Recipe | null;
  /** The computers its sessions keep a copy on; left out, it stays. */
  copies?: string[];
}

/** `PUT /api/machines/settings`. */
export interface MachinesSettingsPatch {
  relay?: RelaySetting;
  keepAwake?: boolean;
  code?: Partial<CodeSettings>;
  projects?: { upsert?: ProjectInput[]; remove?: string[] };
  copiesCapBytes?: number;
}

/** Another computer suggests its relay setting (and code options). */
export interface SettingsOffer {
  from: string;
  fromName: string;
  relay: RelaySetting;
  code?: CodeSettings | null;
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
  /** What other computers asked this one to do (absent from an older
   * server). */
  asks?: MachineAsk[];
  /** Introductions to another computer this one received. */
  introductions?: Introduction[];
  /** Computers revoked here and on the others. */
  revocations?: Revocation[];
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
  /** What a failure blocks when not all of Machines: `account` — that
   * account's sessions (it does not keep Machines off). */
  scope?: string | null;
}

/** A computer's tools and options, beside its checks. */
export interface MachineFacts {
  os: string;
  git: string | null;
  claude: string | null;
  code: CodeSettings | null;
}

export type AskKind = 'fix' | 'signIn' | 'addAccount' | 'clone' | 'sync';
export type AskState = 'asked' | 'running' | 'done' | 'declined' | 'failed';

/** What one computer asked another to do: shown there, done only when
 * someone there confirms it. */
export interface MachineAsk {
  id: string;
  from: string;
  fromName: string;
  kind: AskKind;
  check?: string;
  title?: string;
  fix?: string;
  account?: string;
  origin?: string;
  /** Where a clone goes, or the project here (`sync`). */
  folder?: string;
  /** The code sync's id (`sync`). */
  sync?: string;
  /** The project's folder on the asking computer (`sync`). */
  there?: string;
  at: number;
  state: AskState;
  detail?: string;
  answeredAt?: number;
}

/** One computer's checks. */
export interface MachineChecks {
  machine: string;
  name: string;
  at: number | null;
  checks: SetupCheck[];
  error: string | null;
  facts?: MachineFacts | null;
  /** (Another computer's) what this computer asked of it. */
  asks?: MachineAsk[];
}

export interface TestLine {
  id: 'connect' | 'transfer' | 'tools' | 'code' | 'accounts' | 'folders' | string;
  state: 'ok' | 'warning' | 'failed' | 'skipped' | string;
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
  /** The blockers that keep Machines off (an account not signed in
   * blocks only its sessions); absent from an older server. */
  gating?: number;
  warnings: number;
  tests: Readiness[];
}

// ───────────────────────────────────────────── Accounts and folders (step 5)

export type AccountCellState =
  'signedIn' | 'signedOut' | 'authFailed' | 'tokenMissing' | 'checkFailed' | 'unknown' | 'missing';

/** One account on one computer. */
export interface AccountCell {
  state: AccountCellState;
  kind?: string;
  /** Its name there when it differs (the Default under another name). */
  name?: string;
}

/** One account, matched by name across computers. */
export interface AccountRow {
  name: string;
  default: boolean;
  /** Machine id → its cell. */
  cells: Record<string, AccountCell>;
  /** Machine id → what this computer asked of it for this account. */
  asks: Record<string, MachineAsk>;
}

/** `GET /api/machines/accounts`. */
export interface AccountsTable {
  machine: string;
  machines: { id: string; name: string; here: boolean; online: boolean; error: string | null }[];
  accounts: AccountRow[];
}

export interface RepoPair {
  name: string;
  origin: string;
  here: string;
  there: string;
}

/** A root to add (or a computer's folder to add to one), from the
 * repositories it would map. */
export interface RootProposal {
  id: string;
  name: string;
  here: string;
  there: string;
  root?: string;
  repos: RepoPair[];
}

/** A repository on one computer only. */
export interface LoneRepo {
  name: string;
  path: string;
  origin?: string;
  root?: string;
  rel?: string;
  /** Where it would land on the other computer. */
  mapped?: string;
}

/** This computer's repositories matched with one other's. */
export interface PeerFolders {
  machine: string;
  name: string;
  online: boolean;
  error: string | null;
  proposals: RootProposal[];
  matched: RepoPair[];
  mismatched: RepoPair[];
  onlyHere: LoneRepo[];
  onlyThere: LoneRepo[];
}

/** `GET /api/machines/folders`. */
export interface FoldersView {
  machine: string;
  repos: number;
  noOrigin: { name: string; path: string }[];
  peers: PeerFolders[];
}

/** A computer a root may name (`GET /api/roots`). */
export interface RootMachine {
  id: string;
  name: string;
  os?: string;
  here: boolean;
  paired: boolean;
}

/** A root: one folder per computer. */
export interface RootEntry {
  id: string;
  name: string;
  paths: Record<string, string>;
  updatedAt: number;
  by: string;
  pair?: string;
}

/** `GET`/`PUT /api/roots`. */
export interface RootsView {
  machine: string | null;
  machines: RootMachine[];
  roots: RootEntry[];
  max: number;
}

export interface RootsChange {
  upsert?: { id?: string; name: string; paths: Record<string, string> }[];
  remove?: string[];
}

/** One computer's answer to whether a project's remote answers. */
export interface RemoteResult {
  machine: string;
  name: string;
  here: boolean;
  state: 'ok' | 'failed' | 'skipped' | string;
  detail: string;
}

/** `POST /api/machines/setup/remote`. */
export interface RemoteCheckView {
  project: string;
  origin: string | null;
  results: RemoteResult[];
}

/** `POST /api/machines/setup/ask`: a check's fix, or an asked action. */
export interface AskBody {
  check: string;
  account?: string;
  origin?: string;
  root?: string;
  rel?: string;
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

// ─────────────────────────────────────────────────────────── Code sync

/** Why a synced project waits (docs/PLAN-MACHINES.md §5.15). */
export type SyncReason =
  'conflict' | 'offline' | 'machinesOff' | 'noGrant' | 'moveRunning' | 'error';

/** Where a synced project stands. */
export type SyncStatus =
  | { state: 'off'; detail: string | null }
  | { state: 'waiting' }
  | { state: 'inSync'; at: number }
  | { state: 'syncing'; since: number }
  | { state: 'paused'; reason: SyncReason; detail: string; since: number };

/** What changed on one side since the project was last synced. */
export interface SyncChanges {
  files: number;
  headMoved: boolean;
  branchSwitched: boolean;
  branch: string | null;
  head: string | null;
}

/** A side's state a resolution kept, as a ref in that computer's
 * repository. */
export interface SyncKept {
  ref: string;
  side: 'here' | 'there';
  commit: string | null;
  files: number;
  at: number;
}

/** One project kept in sync with another computer. */
export interface SyncPair {
  id: string;
  machine: string;
  machineName: string;
  machineOs: string;
  online: boolean;
  /** The project's folder here, its name, and its folder there. */
  folder: string;
  name: string;
  there: string;
  here: { on: boolean };
  /** `asked`: this computer asked and the other has not answered yet. */
  peer: { on: boolean; asked: boolean };
  status: SyncStatus;
  last: {
    at: number;
    step: number;
    branch: string | null;
    head: string | null;
    direction: 'out' | 'in' | 'both';
    files: number;
  } | null;
  conflict: { at: number; first: boolean; here: SyncChanges; there: SyncChanges } | null;
  kept: SyncKept[];
  createdAt: number;
  updatedAt: number;
}

/** `GET /api/machines/sync`. */
export interface SyncView {
  pairs: SyncPair[];
  /** The app watches the projects (steps run on their own). */
  watching: boolean;
}

/** `{t:'machines', what}`: refetch `/api/machines` (and the setup);
 * `what: 'sync'`: refetch `/api/machines/sync`. */
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

/** The code options every computer starts with. */
export function defaultCode(): CodeSettings {
  return { transfer: 'bundle', apply: 'checkout', keepRemoteRefs: false };
}

export function sameCode(
  a: CodeSettings | null | undefined,
  b: CodeSettings | null | undefined,
): boolean {
  const x = a ?? defaultCode();
  const y = b ?? defaultCode();
  return x.transfer === y.transfer && x.apply === y.apply && x.keepRemoteRefs === y.keepRemoteRefs;
}

export function sameRelay(
  a: RelaySetting | null | undefined,
  b: RelaySetting | null | undefined,
): boolean {
  if (!a || !b) return a === b;
  const url = (u: string | null) => (u ?? '').trim().replace(/\/+$/, '').toLowerCase();
  return a.mode === b.mode && (a.mode !== 'custom' || url(a.url) === url(b.url));
}

/** The required checks that keep Machines off (an older server counts every
 * blocker). */
export function gatingOf(s: SetupView | null | undefined): number {
  return s ? (s.gating ?? s.blockers) : 0;
}

/** An ask still waiting for someone, or running. */
export function askOpen(a: MachineAsk | null | undefined): boolean {
  return !!a && (a.state === 'asked' || a.state === 'running');
}

// ───────────────────────────────────── More than two computers (Phase 5)

/** Whether two paired computers are paired with each other, as each last
 * said (`pairedWith`); false while neither said. */
export function pairedTogether(a: PeerMachine, b: PeerMachine): boolean {
  return !!(a.pairedWith?.includes(b.id) || b.pairedWith?.includes(a.id));
}

/** The other paired computers `machine` could be introduced to: those not
 * paired with it yet, as far as this computer knows. */
export function introducibleTo(peers: readonly PeerMachine[], machine: string): PeerMachine[] {
  const m = peers.find((p) => p.id === machine);
  if (!m) return [];
  return peers.filter((p) => p.id !== machine && !pairedTogether(m, p));
}

/** Every pair of paired computers not paired with each other (each pair
 * once, in the order they are listed). */
export function unpairedPairs(peers: readonly PeerMachine[]): [PeerMachine, PeerMachine][] {
  const out: [PeerMachine, PeerMachine][] = [];
  peers.forEach((a, i) => {
    for (const b of peers.slice(i + 1)) {
      if (!pairedTogether(a, b)) out.push([a, b]);
    }
  });
  return out;
}

/** What a capability lets a computer do, in a few words (English;
 * translated where shown): the Machines page names those an older computer
 * lacks. */
export const CAPABILITY_LABELS: Record<string, string> = {
  moves: 'move sessions',
  ignoredFiles: 'take files git ignores with a move',
  roots: 'share folders',
  sync: 'keep code in sync',
  run: 'run commands for your other computers',
  tasks: 'take tasks',
  teamRoles: 'run a team’s roles',
  introductions: 'be introduced to your other computers',
  revokeEverywhere: 'take Revoke everywhere',
};
