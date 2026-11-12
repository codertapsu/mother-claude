// Fixtures for the Machines specs: the server's answers, as
// `src-tauri/src/machines/` sends them.
import { TestBed } from '@angular/core/testing';

import {
  MachinesView,
  PairingView,
  PeerMachine,
  SetupCheck,
  SetupView,
  defaultGrants,
} from '../../core/machines';
import { MachinesService } from '../../core/machines.service';
import { FakeBackend, me } from '../../core/mock/fake-server.testing';
import { RealtimeService } from '../../core/realtime.service';

export const SELF_ID = '0'.repeat(32);
export const PEER_ID = 'f'.repeat(32);

/** The other computer of a pairing. */
export const OTHER = { id: PEER_ID, name: 'DESKTOP-PC', os: 'windows', path: 'lan' as const };

export function peer(over: Partial<PeerMachine> = {}): PeerMachine {
  return {
    id: PEER_ID,
    name: 'DESKTOP-PC',
    os: 'windows',
    osVersion: 'Windows 11 Pro',
    versions: { app: '0.11.0', protocol: 1, claude: '2.1.322' },
    key: '8C21 0E5F A9B3 4D67',
    pairedAt: Date.now() - 86_400_000,
    lastSeen: Date.now(),
    online: true,
    path: 'lan',
    rttMs: 6.1,
    grants: defaultGrants(),
    theirGrants: null,
    capabilities: ['hello', 'setup', 'settings'],
    relay: { mode: 'public', url: null },
    problem: null,
    ...over,
  };
}

/** A pairing this computer hosts, its code showing. */
export function pairing(over: Partial<PairingView> = {}): PairingView {
  return {
    role: 'host',
    state: 'waiting',
    code: 'K7QM-4R2X',
    expiresAt: Date.now() + 5 * 60_000,
    other: null,
    digits: null,
    mustType: false,
    confirmed: false,
    otherConfirmed: false,
    error: null,
    peerId: null,
    ...over,
  };
}

/** Machines never set up: off, nothing paired. */
export function machinesView(over: Partial<MachinesView> = {}): MachinesView {
  return {
    enabled: false,
    self: {
      id: SELF_ID,
      name: 'MacBook',
      os: 'macos',
      osVersion: 'macOS 27.2',
      versions: { app: '0.11.0', protocol: 1, claude: '2.1.322' },
      key: '3F9A C1D2 77E0 9B41',
    },
    peers: [],
    pairing: null,
    settings: { relay: { mode: 'public', url: null }, keepAwake: true },
    offer: null,
    transport: { running: false, relay: null },
    health: { state: 'unknown', blockers: 0, warnings: 0, at: null },
    setupStep: null,
    ...over,
  };
}

export function check(over: Partial<SetupCheck> = {}): SetupCheck {
  return {
    id: 'app',
    area: 'app',
    title: 'Mother Claude',
    state: 'ok',
    detail: 'Version 0.11.0 · protocol 1 · Claude Code 2.1.322',
    why: 'Every computer needs a version that speaks the same protocol.',
    severity: 'required',
    fix: null,
    ...over,
  };
}

/** The setup's checks: this computer's, none failing, no test run yet. */
export function setupView(over: Partial<SetupView> = {}): SetupView {
  return {
    self: { machine: SELF_ID, name: 'MacBook', at: Date.now(), checks: [check()], error: null },
    peers: [],
    blockers: 0,
    warnings: 0,
    tests: [],
    ...over,
  };
}

/**
 * Connect live updates as the desktop shell does (Machines' summary
 * started), and return what sends a `machines` frame.
 */
export async function machinesFrames(backend: FakeBackend): Promise<(what?: string) => void> {
  TestBed.inject(MachinesService).start();
  await TestBed.inject(RealtimeService).start();
  backend.last.open();
  backend.last.frame({
    t: 'hello',
    v: 2,
    me: me({ principal: 'desktop', ingress: 'loopback', local: true, allowDangerous: true }),
    serverTime: Date.now(),
  });
  return (what = 'pairing') => backend.last.frame({ t: 'machines', what });
}
