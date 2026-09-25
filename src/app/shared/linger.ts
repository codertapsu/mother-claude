import { DestroyRef, Signal, computed, effect, inject, signal, untracked } from '@angular/core';

export interface Lingering<T> {
  item: T;
  /** It left the source list; shown a moment longer so the card can say
   * how it was settled instead of vanishing. */
  gone: boolean;
}

/**
 * A list that keeps removed entries for `ms` (flagged `gone`). Call in an
 * injection context (it uses an effect).
 */
export function lingerList<T extends { id: string }>(
  source: () => readonly T[],
  ms = 2500,
): Signal<Lingering<T>[]> {
  const gone = signal<ReadonlyMap<string, T>>(new Map());
  const timers = new Map<string, ReturnType<typeof setTimeout>>();
  let prev = new Map<string, T>();

  effect(() => {
    const now = source();
    untracked(() => {
      const ids = new Set(now.map((i) => i.id));
      const next = new Map(gone());
      let changed = false;
      for (const [id, item] of prev) {
        if (!ids.has(id) && !next.has(id)) {
          next.set(id, item);
          changed = true;
          timers.set(
            id,
            setTimeout(() => {
              timers.delete(id);
              gone.update((m) => {
                const n = new Map(m);
                n.delete(id);
                return n;
              });
            }, ms),
          );
        }
      }
      for (const id of [...next.keys()]) {
        if (ids.has(id)) {
          next.delete(id);
          clearTimeout(timers.get(id));
          timers.delete(id);
          changed = true;
        }
      }
      if (changed) gone.set(next);
      prev = new Map(now.map((i) => [i.id, i]));
    });
  });
  inject(DestroyRef).onDestroy(() => timers.forEach((t) => clearTimeout(t)));

  return computed(() => [
    ...source().map((item) => ({ item, gone: false })),
    ...[...gone().values()].map((item) => ({ item, gone: true })),
  ]);
}
