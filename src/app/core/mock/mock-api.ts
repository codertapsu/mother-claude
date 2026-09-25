import { isDevMode } from '@angular/core';

import { storageGet, urlParam } from '../platform';

/**
 * The seam between the core services and the dev-only mock backend.
 *
 * In production builds `mock-loader.ts` is replaced (angular.json
 * fileReplacements) by a stub that returns null, so the mock backend is not
 * even bundled; `mockRequested()` is also false outside dev mode.
 */

/** The subset of the browser WebSocket the realtime service relies on. */
export interface WebSocketLike {
  readonly readyState: number;
  onopen: ((ev: Event) => void) | null;
  onmessage: ((ev: MessageEvent) => void) | null;
  onclose: ((ev: CloseEvent) => void) | null;
  onerror: ((ev: Event) => void) | null;
  send(data: string): void;
  close(code?: number, reason?: string): void;
}

export type MockRole = 'desktop' | 'device' | 'unpaired';

export interface MockBackend {
  readonly role: MockRole;
  /** Token the mock accepts (empty for the unpaired role until paired). */
  readonly token: string;
  fetch(url: string, init?: RequestInit): Promise<Response>;
  openSocket(url: string): WebSocketLike;
}

let active: MockBackend | null = null;

export function activeMock(): MockBackend | null {
  return active;
}

export function setActiveMock(backend: MockBackend | null): void {
  active = backend;
}

/** Dev only: `?mock=1` (real query or hash query) or localStorage `mc_mock=1`. */
export function mockRequested(): boolean {
  if (!isDevMode()) return false;
  return urlParam('mock') === '1' || storageGet('mc_mock') === '1';
}

/** `?as=desktop|device|unpaired` picks who the mock treats you as (default desktop). */
export function mockRole(): MockRole {
  const as = urlParam('as') ?? storageGet('mc_mock_as');
  return as === 'device' || as === 'unpaired' ? as : 'desktop';
}
