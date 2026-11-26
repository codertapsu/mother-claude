import type { SessionSummary } from './models';

/**
 * Pinned sessions (`server::pins`): the ones someone keeps at the top of
 * Sessions, the same on the desktop and every paired device. `GET
 * /api/pins` answers this view; `{t:'pins'}` says it changed.
 */

/** One pinned session: its id and, for another computer's, that computer. */
export interface Pin {
  id: string;
  machine?: string;
  /** When it was pinned: the *Pinned* group keeps this order. */
  at: number;
}

export interface PinsView {
  pins: Pin[];
  /** This computer's pinned sessions as the server knows them, also ones
   * the session list's frames no longer carry (not active lately). */
  sessions: SessionSummary[];
}

/** `{t:'pins'}`: the pins changed — refetch. */
export interface PinsFrame {
  t: 'pins';
}

/** A pin's identity, the same as a session card's: another computer's
 * session may share an id with one here. */
export function pinKey(id: string, machine?: string | null): string {
  return machine ? `${machine}\u0000${id}` : id;
}

/** Read the server's answer tolerantly (a newer server may add more). */
export function normalizePins(raw: unknown): PinsView {
  const r = (raw && typeof raw === 'object' ? raw : {}) as Record<string, unknown>;
  const pins: Pin[] = [];
  for (const p of Array.isArray(r['pins']) ? r['pins'] : []) {
    if (!p || typeof p !== 'object') continue;
    const x = p as Record<string, unknown>;
    if (typeof x['id'] !== 'string' || !x['id']) continue;
    pins.push({
      id: x['id'],
      ...(typeof x['machine'] === 'string' && x['machine'] ? { machine: x['machine'] } : {}),
      at: typeof x['at'] === 'number' ? x['at'] : 0,
    });
  }
  const sessions = (Array.isArray(r['sessions']) ? r['sessions'] : []).filter(
    (s): s is SessionSummary =>
      !!s && typeof s === 'object' && typeof (s as SessionSummary).id === 'string',
  );
  return { pins, sessions };
}
