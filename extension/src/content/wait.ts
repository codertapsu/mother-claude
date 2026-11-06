// `wait` conditions that live in the page: a selector, a text, a quiet
// network. (URL and time waits run in the service worker: a navigation would
// take this script with it.)

import { CommandError } from '../shared/errors.ts';
import { collapse, deepQueryAll, isRendered, windowOf } from './dom.ts';

function now(): number {
  return Date.now();
}

function pageText(doc: Document): string {
  const body = doc.body;
  if (!body) return '';
  const inner = body.innerText;
  return collapse(typeof inner === 'string' ? inner : (body.textContent ?? ''));
}

function stopped(): CommandError {
  return new CommandError('stopped', 'the wait was stopped');
}

/**
 * Resolve when `check()` holds, re-checking on DOM changes (throttled) and on
 * a 100 ms beat; reject with `timeout` at the deadline, `stopped` on abort.
 */
function until(
  doc: Document,
  check: () => boolean,
  deadline: number,
  what: string,
  signal?: AbortSignal,
): Promise<number> {
  const start = now();
  if (signal?.aborted) return Promise.reject(stopped());
  if (check()) return Promise.resolve(0);
  const w = windowOf(doc);
  return new Promise<number>((resolve, reject) => {
    let last = 0;
    let done = false;
    const finish = (ok: boolean, error?: CommandError): void => {
      if (done) return;
      done = true;
      observer?.disconnect();
      clearInterval(beat);
      signal?.removeEventListener('abort', onAbort);
      if (ok) resolve(now() - start);
      else reject(error ?? new CommandError('timeout', `waited ${now() - start} ms for ${what}`));
    };
    const onAbort = (): void => finish(false, stopped());
    signal?.addEventListener('abort', onAbort, { once: true });
    const test = (): void => {
      if (now() >= deadline) finish(false);
      else if (check()) finish(true);
    };
    const observer = w
      ? new w.MutationObserver(() => {
          if (now() - last < 50) return;
          last = now();
          test();
        })
      : null;
    observer?.observe(doc, {
      subtree: true,
      childList: true,
      attributes: true,
      characterData: true,
    });
    const beat = setInterval(test, 100);
  });
}

export function waitForSelector(
  doc: Document,
  selector: string,
  deadline: number,
  signal?: AbortSignal,
): Promise<number> {
  return until(
    doc,
    () => {
      const el = deepQueryAll(doc, selector, 1)[0];
      return !!el && isRendered(el);
    },
    deadline,
    selector,
    signal,
  );
}

export function waitForText(
  doc: Document,
  text: string,
  deadline: number,
  signal?: AbortSignal,
): Promise<number> {
  const wanted = collapse(text).trim();
  if (!wanted) return Promise.resolve(0);
  return until(doc, () => pageText(doc).includes(wanted), deadline, JSON.stringify(wanted), signal);
}

/** Requests still open, as counted by the page hook (−1 when it is not there). */
export type InflightProbe = () => { inflight: number; lastChange: number } | null;

/**
 * A quiet network, heuristically: the document has loaded, the page hook
 * counts no open fetch/XHR, and no resource finished for `quietMs`.
 */
export function waitForNetworkIdle(
  doc: Document,
  deadline: number,
  probe: InflightProbe,
  quietMs = 500,
  signal?: AbortSignal,
): Promise<number> {
  const start = now();
  if (signal?.aborted) return Promise.reject(stopped());
  const w = windowOf(doc);
  let lastActivity = start;
  let observer: PerformanceObserver | null = null;
  try {
    if (w?.PerformanceObserver) {
      observer = new w.PerformanceObserver(() => {
        lastActivity = now();
      });
      observer.observe({ type: 'resource', buffered: false });
    }
  } catch {
    observer = null;
  }
  return new Promise<number>((resolve, reject) => {
    const end = (): void => {
      clearInterval(beat);
      observer?.disconnect();
      signal?.removeEventListener('abort', onAbort);
    };
    const onAbort = (): void => {
      end();
      reject(stopped());
    };
    signal?.addEventListener('abort', onAbort, { once: true });
    const beat = setInterval(() => {
      const t = now();
      const hook = probe();
      if (hook && hook.lastChange > lastActivity) lastActivity = Math.min(t, hook.lastChange);
      // While a request is open (or the page loads) the network is busy now.
      const busy = (hook?.inflight ?? 0) > 0 || doc.readyState !== 'complete';
      if (busy) lastActivity = t;
      if (!busy && t - lastActivity >= quietMs) {
        end();
        resolve(t - start);
      } else if (t >= deadline) {
        end();
        reject(new CommandError('timeout', `the network did not go quiet in ${t - start} ms`));
      }
    }, 50);
  });
}
