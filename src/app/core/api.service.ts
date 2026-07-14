import { Injectable, inject } from '@angular/core';

import { ConfigService } from './config.service';
import {
  GitOverview,
  LaunchDefaults,
  LaunchOverrides,
  Pairing,
  Session,
  TranscriptEvent,
} from './models';

/**
 * Thin REST client over the embedded axum server. Used identically by the
 * desktop webview and the phone browser.
 */
@Injectable({ providedIn: 'root' })
export class ApiService {
  private config = inject(ConfigService);

  private async req<T>(path: string, init?: RequestInit): Promise<T> {
    const cfg = await this.config.ensure();
    const headers = new Headers(init?.headers);
    if (cfg.token) headers.set('Authorization', `Bearer ${cfg.token}`);
    if (init?.body) headers.set('Content-Type', 'application/json');
    const res = await fetch(`${cfg.baseUrl}/api${path}`, { ...init, headers });
    if (res.status === 401) throw new ApiError('unauthorized', 401);
    if (!res.ok) {
      // Surface the server's message (e.g. "No job matching …") rather than a
      // bare status line, so the UI can explain *why* an action failed.
      const body = (await res.text().catch(() => '')).trim();
      throw new ApiError(body || `${res.status} ${res.statusText}`, res.status);
    }
    const text = await res.text();
    return (text ? JSON.parse(text) : null) as T;
  }

  listSessions(): Promise<Session[]> {
    return this.req<Session[]>('/sessions');
  }

  getSession(id: string): Promise<Session> {
    return this.req<Session>(`/sessions/${encodeURIComponent(id)}`);
  }

  getTranscript(id: string, limit = 400): Promise<TranscriptEvent[]> {
    return this.req<TranscriptEvent[]>(
      `/sessions/${encodeURIComponent(id)}/transcript?limit=${limit}`,
    );
  }

  getDiff(id: string): Promise<GitOverview> {
    return this.req<GitOverview>(`/sessions/${encodeURIComponent(id)}/diff`);
  }

  getFilePatch(id: string, path: string): Promise<{ patch: string | null }> {
    return this.req(
      `/sessions/${encodeURIComponent(id)}/file-patch?path=${encodeURIComponent(path)}`,
    );
  }

  getServices(): Promise<unknown> {
    return this.req('/services');
  }

  /** Launch defaults + available models/efforts (from the user's own settings). */
  getDefaults(): Promise<LaunchDefaults> {
    return this.req<LaunchDefaults>('/defaults');
  }

  getDaemon(): Promise<unknown> {
    return this.req('/daemon');
  }

  getPairing(): Promise<Pairing> {
    return this.req<Pairing>('/pairing');
  }

  // --- Control (endpoints land in the control commits) ---

  spawnSession(
    body: { cwd: string; prompt: string } & LaunchOverrides,
  ): Promise<{ id: string }> {
    return this.req('/sessions', { method: 'POST', body: JSON.stringify(body) });
  }

  sendMessage(id: string, text: string): Promise<void> {
    return this.req(`/sessions/${encodeURIComponent(id)}/message`, {
      method: 'POST',
      body: JSON.stringify({ text }),
    });
  }

  /** Fork a session's conversation into a new owned session we can drive. */
  continueSession(
    id: string,
    prompt?: string,
    overrides?: LaunchOverrides,
  ): Promise<{ id: string }> {
    return this.req(`/sessions/${encodeURIComponent(id)}/continue`, {
      method: 'POST',
      body: JSON.stringify({ prompt: prompt ?? '', ...(overrides ?? {}) }),
    });
  }

  respondPermission(id: string, decision: string, requestId?: string): Promise<void> {
    return this.req(`/sessions/${encodeURIComponent(id)}/permission`, {
      method: 'POST',
      body: JSON.stringify({ decision, requestId }),
    });
  }

  answerQuestion(id: string, answer: string, requestId?: string): Promise<void> {
    return this.req(`/sessions/${encodeURIComponent(id)}/answer`, {
      method: 'POST',
      body: JSON.stringify({ answer, requestId }),
    });
  }

  lifecycle(id: string, action: 'stop' | 'respawn' | 'rm'): Promise<void> {
    return this.req(`/sessions/${encodeURIComponent(id)}/${action}`, { method: 'POST' });
  }

  installHooks(): Promise<unknown> {
    return this.req('/hooks/install', { method: 'POST' });
  }
}

export class ApiError extends Error {
  constructor(
    message: string,
    readonly status: number,
  ) {
    super(message);
  }
}
