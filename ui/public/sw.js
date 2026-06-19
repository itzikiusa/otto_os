// Otto service worker — minimal PWA shell.
// Caching policy:
//   - /api/* and /ws/* are NEVER cached (live daemon traffic).
//   - Everything else uses a cache-first strategy for the app shell
//     (index.html + built JS/CSS) so the PWA loads offline/fast.
//     On install the shell assets are pre-cached; stale entries are
//     cleaned on activate.

const CACHE_NAME = 'otto-shell-v1';

// Assets to pre-cache on install (populated by the build; kept minimal).
const PRECACHE_URLS = ['/', '/manifest.webmanifest'];

self.addEventListener('install', (event) => {
  event.waitUntil(
    caches
      .open(CACHE_NAME)
      .then((cache) => cache.addAll(PRECACHE_URLS))
      .then(() => self.skipWaiting())
  );
});

self.addEventListener('activate', (event) => {
  event.waitUntil(
    caches
      .keys()
      .then((keys) =>
        Promise.all(keys.filter((k) => k !== CACHE_NAME).map((k) => caches.delete(k)))
      )
      .then(() => self.clients.claim())
  );
});

self.addEventListener('fetch', (event) => {
  const url = new URL(event.request.url);

  // Never intercept API or WebSocket traffic — always go to the live daemon.
  if (url.pathname.startsWith('/api') || url.pathname.startsWith('/ws')) {
    return; // pass-through: browser handles it normally
  }

  // Cache-first for everything else (app shell).
  event.respondWith(
    caches.match(event.request).then((cached) => {
      if (cached) return cached;
      return fetch(event.request).then((response) => {
        // Only cache valid same-origin GET responses.
        if (
          !response ||
          response.status !== 200 ||
          response.type !== 'basic' ||
          event.request.method !== 'GET'
        ) {
          return response;
        }
        const clone = response.clone();
        caches.open(CACHE_NAME).then((cache) => cache.put(event.request, clone));
        return response;
      });
    })
  );
});
