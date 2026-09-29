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

export interface RouteReply {
  status: number;
  body: unknown;
  headers?: Record<string, string>;
}

/** Answers one path; gets the request's JSON body (undefined for none). */
export type Route = (method: string, body?: unknown) => RouteReply | Promise<RouteReply>;

export class FakeBackend implements MockBackend {
  readonly role = 'device' as const;
  readonly token = 'tok';
  readonly sockets: FakeSocket[] = [];
  readonly calls: string[] = [];
  /** Every request's JSON body, in the order of `calls`. */
  readonly bodies: unknown[] = [];
  readonly routes = new Map<string, Route>();

  async fetch(url: string, init?: RequestInit): Promise<Response> {
    const path = new URL(url).pathname.replace(/^\/api/, '');
    const method = init?.method ?? 'GET';
    const sent = parseJson(init?.body);
    this.calls.push(`${method} ${path}`);
    this.bodies.push(sent);
    const route = this.routes.get(path);
    const { status, body, headers } = route
      ? await route(method, sent)
      : ({ status: 404, body: {} } as RouteReply);
    return new Response(JSON.stringify(body), {
      status,
      headers: { 'content-type': 'application/json', ...headers },
    });
  }

  /** The body of the last `METHOD /path` request. */
  bodyOf(call: string): unknown {
    const i = this.calls.lastIndexOf(call);
    return i < 0 ? undefined : this.bodies[i];
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

function parseJson(body: BodyInit | null | undefined): unknown {
  if (typeof body !== 'string') return undefined;
  try {
    return JSON.parse(body) as unknown;
  } catch {
    return body;
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
