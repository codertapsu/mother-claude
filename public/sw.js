/*
 * Mother Claude service worker: push notifications ONLY.
 *
 * Deliberately no `fetch` handler and no caching — the app is served live by
 * the desktop app, so a cache could only ever serve a stale build. Registered
 * by the SPA only in a secure context outside the desktop webview.
 *
 * Push payload (server/push.rs, decrypted JSON; deliberately no URL):
 *   { v, title, body, kind, kinds[], tag, renotify, actionable, pendingIds[],
 *     count, ts, sessionId?, pendingId? }
 * The click target is built here from `sessionId` / `pendingId` after
 * validating them, so a payload can never send the user somewhere else.
 */
'use strict';

const ID = /^[A-Za-z0-9_-]{1,80}$/;

self.addEventListener('install', () => {
  self.skipWaiting();
});

self.addEventListener('activate', (event) => {
  event.waitUntil(self.clients.claim());
});

/** `/#/session/<id>?pending=<pendingId>` for a valid session, else home. */
function targetOf(data) {
  const session =
    typeof data.sessionId === 'string' && ID.test(data.sessionId) ? data.sessionId : '';
  if (!session) return '/#/';
  const pending =
    typeof data.pendingId === 'string' && ID.test(data.pendingId) ? data.pendingId : '';
  return pending ? `/#/session/${session}?pending=${pending}` : `/#/session/${session}`;
}

self.addEventListener('push', (event) => {
  let data = {};
  try {
    data = event.data ? event.data.json() : {};
  } catch {
    data = { body: event.data ? event.data.text() : '' };
  }
  if (!data || typeof data !== 'object') data = {};
  const title = typeof data.title === 'string' && data.title ? data.title : 'Mother Claude';
  const session =
    typeof data.sessionId === 'string' && ID.test(data.sessionId) ? data.sessionId : '';
  const tag =
    typeof data.tag === 'string' && data.tag
      ? data.tag.slice(0, 120)
      : session
        ? `session-${session}`
        : 'mother-claude';
  const options = {
    body: typeof data.body === 'string' ? data.body : '',
    icon: 'icons/icon-192.png',
    badge: 'icons/icon-192.png',
    // A newer event for the same card or session replaces the older one.
    tag,
    renotify: data.renotify !== false,
    timestamp: typeof data.ts === 'number' ? data.ts : Date.now(),
    data: { url: targetOf(data) },
  };
  event.waitUntil(self.registration.showNotification(title, options));
});

self.addEventListener('notificationclick', (event) => {
  event.notification.close();
  const target = new URL(
    (event.notification.data && event.notification.data.url) || '/#/',
    self.location.origin,
  ).href;
  event.waitUntil(
    (async () => {
      const windows = await self.clients.matchAll({ type: 'window', includeUncontrolled: true });
      for (const client of windows) {
        if (new URL(client.url).origin !== self.location.origin) continue;
        try {
          await client.focus();
          if ('navigate' in client) await client.navigate(target);
          return;
        } catch {
          // Fall through to opening a new window.
        }
      }
      await self.clients.openWindow(target);
    })(),
  );
});
