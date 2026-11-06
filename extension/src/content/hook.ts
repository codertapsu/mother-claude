// Runs in the page's own JavaScript world (MAIN), injected into lease tabs
// only. It reports JavaScript dialogs *before* they block the page — so a
// command that triggered one can answer at once instead of hanging — and
// counts open fetch/XHR requests for the network-idle heuristic. It changes
// nothing a page can observe except through these wrappers, which are
// Proxies of the originals (so they still print as native code).
//
// Passed to chrome.scripting.executeScript as `func`: it must stay
// self-contained (no imports, no outer variables).

export function installPageHook(nonce: string): boolean {
  const d = document;
  const ask = `mc-q-${nonce}`;
  const answer = `mc-a-${nonce}`;
  let present = false;
  const probe = (): void => {
    present = true;
  };
  d.addEventListener(answer, probe);
  d.dispatchEvent(new CustomEvent(ask));
  d.removeEventListener(answer, probe);
  if (present) return false;

  let inflight = 0;
  let lastChange = Date.now();
  const emit = (type: string, detail: unknown): void => {
    d.dispatchEvent(new CustomEvent(`mc-${type}-${nonce}`, { detail: JSON.stringify(detail) }));
  };
  const settled = (): void => {
    inflight = Math.max(0, inflight - 1);
    lastChange = Date.now();
  };
  const w = window as unknown as Record<string, unknown>;

  for (const kind of ['alert', 'confirm', 'prompt'] as const) {
    const original = window[kind] as (...args: unknown[]) => unknown;
    try {
      w[kind] = new Proxy(original, {
        apply(target, thisArg: unknown, args: unknown[]) {
          emit('dialog', { kind, message: String(args[0] ?? '') });
          try {
            return Reflect.apply(target, thisArg, args) as unknown;
          } finally {
            emit('dialog-closed', { kind });
          }
        },
      });
    } catch {
      // a page that froze window.alert keeps it
    }
  }

  try {
    const originalFetch = window.fetch;
    w['fetch'] = new Proxy(originalFetch, {
      apply(target, thisArg: unknown, args: unknown[]) {
        inflight++;
        lastChange = Date.now();
        let p: Promise<Response>;
        try {
          p = Reflect.apply(target, thisArg, args) as Promise<Response>;
        } catch (e) {
          settled();
          throw e;
        }
        p.then(settled, settled);
        return p;
      },
    });
  } catch {
    // not writable
  }

  try {
    const proto = XMLHttpRequest.prototype;
    const send = proto.send;
    proto.send = new Proxy(send, {
      apply(target, xhr: XMLHttpRequest, args: unknown[]) {
        inflight++;
        lastChange = Date.now();
        xhr.addEventListener('loadend', settled, { once: true });
        try {
          return Reflect.apply(target, xhr, args) as unknown;
        } catch (e) {
          settled();
          throw e;
        }
      },
    }) as typeof send;
  } catch {
    // not writable
  }

  d.addEventListener(ask, () => emit('a', { inflight, lastChange }));
  return true;
}
