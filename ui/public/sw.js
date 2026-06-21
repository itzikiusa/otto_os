// Otto service worker — PWA shell with SAFE update semantics.
//
// Caching policy:
//   - /api/* and /ws/* are NEVER cached (live daemon traffic).
//   - Navigations / HTML (the app shell) are NETWORK-FIRST so a new deploy is
//     picked up immediately; the cache is only an offline fallback.
//     (The previous cache-FIRST-on-index.html policy served a stale index.html
//     forever → stale hashed-asset references → the whole app stuck on an old
//     build even after redeploys. That was the "nothing is fixed" bug.)
//   - Hashed assets under /assets/* are immutable (content-addressed filenames
//     change every build) so they're safe to serve cache-first.
//
// Bump CACHE_NAME on any policy change so `activate` purges the old cache.

const CACHE_NAME = 'otto-shell-v2';
const PRECACHE_URLS = ['/manifest.webmanifest'];

self.addEventListener('install', (event) => {
  event.waitUntil(
    caches
      .open(CACHE_NAME)
      .then((cache) => cache.addAll(PRECACHE_URLS))
      .then(() => self.skipWaiting()),
  );
});

self.addEventListener('activate', (event) => {
  event.waitUntil(
    caches
      .keys()
      .then((keys) =>
        Promise.all(keys.filter((k) => k !== CACHE_NAME).map((k) => caches.delete(k))),
      )
      .then(() => self.clients.claim()),
  );
});

self.addEventListener('fetch', (event) => {
  const url = new URL(event.request.url);

  // Never intercept API or WebSocket traffic — always the live daemon.
  if (url.pathname.startsWith('/api') || url.pathname.startsWith('/ws')) return;
  if (event.request.method !== 'GET') return;

  // App shell (navigations / HTML): NETWORK-FIRST, cache as offline fallback.
  const isNav =
    event.request.mode === 'navigate' ||
    url.pathname === '/' ||
    url.pathname.endsWith('.html');
  if (isNav) {
    event.respondWith(
      fetch(event.request)
        .then((resp) => {
          const clone = resp.clone();
          caches.open(CACHE_NAME).then((c) => c.put(event.request, clone));
          return resp;
        })
        .catch(() => caches.match(event.request).then((c) => c || caches.match('/'))),
    );
    return;
  }

  // Immutable hashed assets + other static files: cache-first.
  event.respondWith(
    caches.match(event.request).then(
      (cached) =>
        cached ||
        fetch(event.request).then((resp) => {
          if (resp && resp.status === 200 && resp.type === 'basic') {
            const clone = resp.clone();
            caches.open(CACHE_NAME).then((c) => c.put(event.request, clone));
          }
          return resp;
        }),
    ),
  );
});
