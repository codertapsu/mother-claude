import { CapLimit, ControlCaps, LimitAction, SessionSummary } from '../core/models';

/**
 * Defensive readers for {@link ControlCaps}. The capability fields from the
 * design amendments (limits, rename, terminal, keys, attach, approveVia,
 * wake) may be missing on an older server, so every consumer reads them
 * through here instead of trusting the shape.
 */

/** The session's limitations (falls back to the legacy free-text notes). */
export function capLimits(c: ControlCaps | undefined): CapLimit[] {
  if (!c) return [];
  if (Array.isArray(c.limits) && c.limits.length) {
    return c.limits.filter((l) => l && typeof l.text === 'string' && l.text.trim());
  }
  return (c.notes ?? [])
    .filter((n) => typeof n === 'string' && n.trim())
    .map((text, i) => ({ code: `note-${i}`, text, action: guessAction(text) }));
}

/** Old servers only had text: offer the obvious button anyway. */
function guessAction(text: string): LimitAction | undefined {
  const t = text.toLowerCase();
  if (t.includes('install') && t.includes('control')) return 'install-foreign-control';
  if (t.includes('reload-plugins')) return 'reload-plugins';
  if (t.includes('fork')) return 'fork';
  if (t.includes('continue here')) return 'continue-owned';
  return undefined;
}

export function capAttach(c: ControlCaps | undefined): 'inline' | 'path' | 'none' {
  if (!c) return 'none';
  if (c.attach === 'inline' || c.attach === 'path' || c.attach === 'none') return c.attach;
  if (c.attachImages) return 'inline';
  return c.message === 'relay' ? 'path' : 'none';
}

export function capRename(c: ControlCaps | undefined, owned = false): 'sync' | 'alias' | 'none' {
  if (!c) return 'none';
  if (c.rename === 'sync' || c.rename === 'alias' || c.rename === 'none') return c.rename;
  return owned || c.channel === 'owned' ? 'sync' : 'alias';
}

export function capTerminal(c: ControlCaps | undefined, s?: SessionSummary): boolean {
  if (typeof c?.terminal === 'boolean') return c.terminal;
  return !!s && (!!s.tmuxPane || s.surface === 'tmux' || s.surface === 'background');
}

export function capKeys(c: ControlCaps | undefined, s?: SessionSummary): boolean {
  if (typeof c?.keys === 'boolean') return c.keys;
  return c?.channel === 'tmux' || s?.surface === 'tmux';
}

/** Ultracode can be switched from here (an older server can't: no). */
export function capUltracode(c: ControlCaps | undefined): boolean {
  return c?.setUltracode === true;
}

/** Thinking can be switched from here (an older server can't: no). */
export function capThinking(c: ControlCaps | undefined): boolean {
  return c?.setThinking === true;
}

/** Idle sessions take a relayed message now (else at the next stop). */
export function capWake(c: ControlCaps | undefined): boolean {
  if (typeof c?.wake === 'boolean') return c.wake;
  return c?.channel === 'owned' || c?.channel === 'tmux';
}

/** A limit whose action is one of `actions`, if any. */
export function limitFor(
  c: ControlCaps | undefined,
  ...actions: LimitAction[]
): CapLimit | undefined {
  return capLimits(c).find((l) => l.action && actions.includes(l.action));
}
