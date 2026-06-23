import { Injectable, inject, signal } from '@angular/core';

import { ConfigService } from './config.service';
import { PendingInput, ServerEvent, Session, TranscriptEvent } from './models';

export type ConnState = 'connecting' | 'open' | 'closed';

/**
 * Single WebSocket to `/ws`, feeding signals the views react to. Reconnects with
 * backoff. The initial snapshot and every refresh arrive as `sessions` events.
 */
@Injectable({ providedIn: 'root' })
export class RealtimeService {
  private config = inject(ConfigService);

  readonly sessions = signal<Session[]>([]);
  readonly connection = signal<ConnState>('closed');
  /** Latest live transcript delta (session detail merges these). */
  readonly transcriptDelta = signal<{ sessionId: string; events: TranscriptEvent[] } | null>(null);
  /** Latest pending change. */
  readonly pendingDelta = signal<{ sessionId: string; pending?: PendingInput } | null>(null);
  /** Latest hook event (Services / activity feed). */
  readonly hookEvent = signal<unknown>(null);

  private socket?: WebSocket;
  private retry = 0;
  private started = false;

  async start(): Promise<void> {
    if (this.started) return;
    this.started = true;
    await this.config.ensure();
    this.connect();
  }

  private connect(): void {
    const cfg = this.config.config();
    if (!cfg) return;
    this.connection.set('connecting');
    let socket: WebSocket;
    try {
      socket = new WebSocket(cfg.wsUrl);
    } catch {
      this.scheduleReconnect();
      return;
    }
    this.socket = socket;

    socket.onopen = () => {
      this.retry = 0;
      this.connection.set('open');
    };
    socket.onmessage = (ev) => this.dispatch(ev.data);
    socket.onclose = () => {
      this.connection.set('closed');
      this.scheduleReconnect();
    };
    socket.onerror = () => socket.close();
  }

  private scheduleReconnect(): void {
    const delay = Math.min(1000 * 2 ** this.retry, 15000);
    this.retry += 1;
    setTimeout(() => this.connect(), delay);
  }

  private dispatch(raw: string): void {
    let event: ServerEvent;
    try {
      event = JSON.parse(raw) as ServerEvent;
    } catch {
      return;
    }
    switch (event.kind) {
      case 'sessions':
        this.sessions.set(event.data);
        break;
      case 'transcript':
        this.transcriptDelta.set(event.data);
        break;
      case 'pending':
        this.pendingDelta.set(event.data);
        break;
      case 'hook':
        this.hookEvent.set(event.data);
        break;
      case 'notice':
        // surfaced opportunistically; no-op for now
        break;
    }
  }
}
