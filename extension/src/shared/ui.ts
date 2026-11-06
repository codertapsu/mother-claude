// Messages between the service worker and the extension's own pages (side
// panel, options, lease placeholder). Never sent to the desktop.

import type { PendingItem, Settings, SiteMode, TabGroupColor } from './protocol.ts';

export const UI_PORT = 'mc-ui';

export type ConnectionStateName =
  'unpaired' | 'connecting' | 'connected' | 'disconnected' | 'revoked' | 'auth_failed';

export interface UiLease {
  lease: string;
  /** The Claude Code session working in the lease's tab group. */
  session: string;
  title: string;
  color: TabGroupColor;
  tabs: number;
  activity: string | null;
  activitySince: number | null;
  last: string | null;
  lastAt: number | null;
  stoppedUntil: number;
}

export interface UiStatus {
  connection: {
    state: ConnectionStateName;
    error: string;
    appVersion: string;
    retryAt: number;
    host: string;
    port: number;
    pingMs: number;
  };
  paired: boolean;
  browserName: string;
  browserId: string;
  leases: UiLease[];
  pending: PendingItem[];
  siteModes: Record<string, SiteMode>;
  settings: Settings;
  granted: string[];
  debuggerAttached: number;
  extensionVersion: string;
  extensionId: string;
}

export interface TabInfo {
  tabId: number;
  url: string;
  title: string;
  origin: string | null;
  mode: SiteMode;
  /** The user let the extension into this origin (host permission). */
  hostGranted: boolean;
  /** The lease whose group the tab is in. */
  lease: string | null;
  shared: boolean;
  /** The user clicked the toolbar button on this tab (Chrome's activeTab). */
  activeGrant: boolean;
  dialog: { kind: string; message: string } | null;
  /** The origin has its own site-mode entry (not just the default). */
  ownEntry: boolean;
  /** On the desktop's sensitive list: only the desktop raises it to act. */
  sensitive: boolean;
}

/** A Claude Code session on this computer, as the desktop lists it. */
export interface UiSession {
  id: string;
  name: string;
  /** Its project folder's name, and the folder. */
  project: string;
  cwd: string;
  /** Where it runs: "VS Code", "Terminal", "Mother Claude", … */
  surface: string;
  /** working | waiting | idle | background | starting */
  status: string;
  lastActivityAt: number | null;
  owned: boolean;
  /** Mother Claude can send it what to do (directly, or relayed at its next step). */
  canTell: boolean;
  tellHow: 'direct' | 'relay' | null;
  /** Why it can't be told, when it can't. */
  whyNot: string | null;
  /** It has Mother Claude's browser tools (seen listing them, or owned). */
  hasTools: boolean;
  /** Its lease in this browser, when connected here. */
  lease: string | null;
}

export type UiRequest =
  | { op: 'status' }
  | { op: 'pair'; code: string; name: string }
  | { op: 'forget' }
  | { op: 'setConfig'; host: string; port: number; browserName?: string }
  | { op: 'stop'; lease: string }
  | { op: 'resume'; lease: string }
  /** `mode: null` removes the site's own entry (back to the default). */
  | { op: 'setSiteMode'; origin: string; mode: SiteMode | null }
  | { op: 'answer'; pending: string; allow: boolean }
  | { op: 'tabInfo'; tabId: number }
  | { op: 'share'; tabId: number }
  | { op: 'unshare' }
  | { op: 'addToLease'; tabId: number; lease: string }
  | { op: 'reconnect' }
  | { op: 'permissionsChanged' }
  | { op: 'leaseInfo'; lease: string }
  /** The live Claude Code sessions on this computer (from the desktop). */
  | { op: 'sessions' }
  /** Connect a session to this browser with this tab; tell it `message` if given. */
  | { op: 'connect'; session: string; tabId: number; message?: string }
  /** Tell a session connected here what to do. */
  | { op: 'tell'; lease: string; message: string };

export type UiResponse =
  ({ ok: true } & Record<string, unknown>) | { ok: false; code: string; message: string };

export interface UiMessage {
  type: 'mc.ui';
  req: UiRequest;
}

export function isUiMessage(m: unknown): m is UiMessage {
  return typeof m === 'object' && m !== null && (m as { type?: unknown }).type === 'mc.ui';
}
