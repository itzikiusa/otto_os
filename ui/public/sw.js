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

// v3: v2 could poison itself with a stale (or error) shell — see the navigation
// handler below. Bumping the name makes `activate` purge every v2 entry once,
// which is the only way to evict a bad shell from clients already carrying one.
const CACHE_NAME = 'otto-shell-v3';
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
          // ONLY cache a good shell. The daemon restarts on every deploy, and a
          // request landing in that window can come back 502/503 — caching that
          // pins a broken or outdated shell that later loads happily from
          // cache-first hashed assets, so the app silently keeps running an old
          // build across reloads. Anything non-OK is passed through uncached.
          if (resp && resp.ok) {
            const clone = resp.clone();
            caches.open(CACHE_NAME).then((c) => c.put(event.request, clone));
          }
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
