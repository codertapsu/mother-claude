import { EnvironmentProviders, Provider } from '@angular/core';
import { TestBed } from '@angular/core/testing';

import { AuthService } from '../auth.service';
import { ConfigService } from '../config.service';
import { MeInfo } from '../models';
import { MockBackend, WebSocketLike, setActiveMock } from './mock-api';

/**
 * Test doubles for specs: a scriptable WebSocket and a REST backend plugged
 * in through the mock seam, so the real services run against them.
 */
export class FakeSocket implements WebSocketLike {
  readyState = 0;
  onopen: ((ev: Event) => void) | null = null;
  onmessage: ((ev: MessageEvent) => void) | null = null;
  onclose: ((ev: CloseEvent) => void) | null = null;
  onerror: ((ev: Event) => void) | null = null;
  readonly sent: { t: string }[] = [];

  send(data: string): void {
    this.sent.push(JSON.parse(data));
  }
  close(): void {
    this.readyState = 3;
  }
  open(): void {
    this.readyState = 1;
    this.onopen?.(new Event('open'));
  }
  frame(obj: unknown): void {
    this.onmessage?.(new MessageEvent('message', { data: JSON.stringify(obj) }));
  }
  closeWith(code: number): void {
    this.readyState = 3;
    this.onclose?.(new CloseEvent('close', { code }));
  }
}

export type Route = (
  method: string,
) => { status: number; body: unknown } | Promise<{ status: number; body: unknown }>;

export class FakeBackend implements MockBackend {
  readonly role = 'device' as const;
  readonly token = 'tok';
  readonly sockets: FakeSocket[] = [];
  readonly calls: string[] = [];
  readonly routes = new Map<string, Route>();

  async fetch(url: string, init?: RequestInit): Promise<Response> {
    const path = new URL(url).pathname.replace(/^\/api/, '');
    const method = init?.method ?? 'GET';
    this.calls.push(`${method} ${path}`);
    const route = this.routes.get(path);
    const { status, body } = route ? await route(method) : { status: 404, body: {} };
    return new Response(JSON.stringify(body), {
      status,
      headers: { 'content-type': 'application/json' },
    });
  }

  openSocket(): WebSocketLike {
    const s = new FakeSocket();
    this.sockets.push(s);
    return s;
  }

  get last(): FakeSocket {
    return this.sockets[this.sockets.length - 1];
  }
}

export const me = (over: Partial<MeInfo> = {}): MeInfo => ({
  principal: 'device',
  ingress: 'lan',
  local: false,
  allowDangerous: false,
  ...over,
});

/** Let pending promises and zero-delay timers run. */
export const flush = async (times = 5) => {
  for (let i = 0; i < times; i++) await new Promise((r) => setTimeout(r, 0));
};

/** Configure TestBed as a paired phone talking to `backend`. */
export function useFakeServer(
  backend: FakeBackend,
  providers: (Provider | EnvironmentProviders)[] = [],
): void {
  setActiveMock(backend);
  TestBed.configureTestingModule({ providers });
  TestBed.inject(ConfigService).config.set({
    baseUrl: 'http://mc.test',
    wsUrl: 'ws://mc.test/ws',
    token: backend.token,
    desktop: false,
    mock: true,
  });
  TestBed.inject(AuthService).state.set('paired');
}
