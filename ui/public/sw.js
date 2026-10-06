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
//     change every build) so they're safe to serve cache-first — and ONLY
//     those. Every other static file (theme-boot.js, worklets, icons, the
//     manifest) keeps a stable name across builds, so it is NETWORK-FIRST with
//     the cache as an offline fallback; cache-first pinned them stale until the
//     next CACHE_NAME bump.
//   - The cache name is constant across builds, so each deploy used to add a
//     full set of hashed chunks that were never evicted. When a fetched shell
//     references a different /assets/* set than the last one seen (= a new
//     build), every cached /assets/* entry it doesn't reference is deleted;
//     lazy chunks of the new build re-fetch from the network (HTTP `immutable`)
//     on first use. A reload of the same build prunes nothing.
//
// Bump CACHE_NAME on any policy change so `activate` purges the old cache.

// v3: v2 could poison itself with a stale (or error) shell — see the navigation
// handler below. Bumping the name makes `activate` purge every v2 entry once,
// which is the only way to evict a bad shell from clients already carrying one.
// v4: cache-first narrowed to /assets/*; old-build assets pruned (see above).
// v5: never touch /browser/ (take-over proxy), /plugins/, cross-origin or any
// URL carrying a credential (`token=` / `ticket=`); navigations cache only the
// shell under '/', never each URL. The bump purges anything v4 stored there.
const CACHE_NAME = 'otto-shell-v5';
// Synthetic cache key holding the /assets/* set of the last shell seen.
const SHELL_ASSETS_KEY = '/__otto-shell-assets';
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
  // Only the app's own origin is ours to cache.
  if (url.origin !== self.location.origin) return;
  // Third-party content (the take-over proxy, plugin UIs) and any URL that
  // carries a credential must never land in (or be served from) the cache.
  if (/^\/(browser|plugins)\//.test(url.pathname)) return;
  if (/(^|[?&])(token|ticket|access_token)=/i.test(url.search)) return;
  // Vite dev-server modules (`npm run dev`) are never intercepted. The
  // network-first branch below cached every one of them (each HMR `?t=` URL a
  // new entry) and put the worker on every module fetch, which also hid them
  // from Playwright's `page.route`. A built app never serves these paths, so
  // production caching is unchanged (no CACHE_NAME bump).
  if (/^\/(src|@vite|@fs|@id|node_modules)\//.test(url.pathname)) return;

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
          // Every client route is the same SPA document, so only the shell is
          // stored — under '/' — never the per-URL response.
          const ct = resp ? resp.headers.get('content-type') || '' : '';
          if (resp && resp.ok && resp.type === 'basic' && ct.includes('text/html')) {
            const clone = resp.clone();
            caches.open(CACHE_NAME).then((c) => c.put('/', clone));
            event.waitUntil(resp.clone().text().then(pruneAssets).catch(() => {}));
          }
          return resp;
        })
        .catch(() => caches.match('/')),
    );
    return;
  }

  // Immutable hashed assets: cache-first.
  if (url.pathname.startsWith('/assets/')) {
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
    return;
  }

  // Other (non-hashed) static files: NETWORK-FIRST, cache as offline fallback.
  event.respondWith(
    fetch(event.request)
      .then((resp) => {
        if (resp && resp.status === 200 && resp.type === 'basic') {
          const clone = resp.clone();
          caches.open(CACHE_NAME).then((c) => c.put(event.request, clone));
        }
        return resp;
      })
      .catch(() => caches.match(event.request).then((c) => c || Response.error())),
  );
});

/** Drop cached /assets/* entries the new shell `html` doesn't reference — but
 *  only when its asset set differs from the last shell's (a new build). */
async function pruneAssets(html) {
  const refs = new Set();
  for (const m of html.matchAll(/\/assets\/[^"'\s)>?#]+/g)) refs.add(m[0]);
  if (refs.size === 0) return; // not an app shell — never prune on it
  const sig = [...refs].sort().join('\n');
  const cache = await caches.open(CACHE_NAME);
  const prev = await cache.match(SHELL_ASSETS_KEY);
  if (prev && (await prev.text()) === sig) return; // same build
  const keys = await cache.keys();
  await Promise.all(
    keys.map((req) => {
      const p = new URL(req.url).pathname;
      return p.startsWith('/assets/') && !refs.has(p) ? cache.delete(req) : null;
    }),
  );
  await cache.put(SHELL_ASSETS_KEY, new Response(sig));
}
