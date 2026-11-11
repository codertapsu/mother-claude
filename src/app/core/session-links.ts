import { InjectionToken } from '@angular/core';

/**
 * The other computer a page's session runs on (its id), provided by that
 * computer's scope (`machine-scope.ts`: another computer's session page, or
 * the services that answer its requests); null everywhere else, which is this
 * computer's own (docs/PLAN-MACHINES.md §6.2).
 */
export const SESSION_MACHINE = new InjectionToken<string | null>('mc.sessionMachine', {
  providedIn: 'root',
  factory: () => null,
});

/** A session page's route: this computer's (`/session/<id>`), or another
 * computer's (`/session/<id>/on/<machine>`). */
export function sessionLink(id: string, machine?: string | null): string[] {
  return machine ? ['/session', id, 'on', machine] : ['/session', id];
}

/** The page of one of a session's agents, wherever the session runs. */
export function agentLink(id: string, agent: string, machine?: string | null): string[] {
  return machine
    ? ['/session', id, 'agent', agent, 'on', machine]
    : ['/session', id, 'agent', agent];
}
