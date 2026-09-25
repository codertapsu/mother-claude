/*
 * Mother Claude service worker: push notifications ONLY.
 *
 * Deliberately no `fetch` handler and no caching — the app is served live by
 * the desktop app, so a cache could only ever serve a stale build. Registered
 * by the SPA only in a secure context outside the desktop webview.
 *
 * Push payload (server/push.rs): { title, body, sessionId, url: "/#/session/<id>" }
 */
'use strict';

self.addEventListener('install', () => {
  self.skipWaiting();
});

self.addEventListener('activate', (event) => {
  event.waitUntil(self.clients.claim());
});

self.addEventListener('push', (event) => {
  let data = {};
  try {
    data = event.data ? event.data.json() : {};
  } catch {
    data = { body: event.data ? event.data.text() : '' };
  }
  const title = typeof data.title === 'string' && data.title ? data.title : 'Mother Claude';
  const url = typeof data.url === 'string' && data.url.startsWith('/') ? data.url : '/#/';
  const options = {
    body: typeof data.body === 'string' ? data.body : '',
    icon: 'icons/icon-192.png',
    badge: 'icons/icon-192.png',
    // One notification per session: a newer event replaces the older one.
    tag: typeof data.sessionId === 'string' ? `session-${data.sessionId}` : 'mother-claude',
    renotify: true,
    data: { url },
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
