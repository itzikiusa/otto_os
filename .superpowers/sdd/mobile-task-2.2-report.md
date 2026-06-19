# Task 2.2 — PWA manifest, icons, service worker (iOS + Android install)

**Date:** 2026-06-20  
**Branch:** feat/rbac-multiuser (worktree: otto_os-rbac)  
**Status:** DONE — all steps complete, build + check clean, dist verified.

---

## Icons generated

| File | Dimensions | Method |
|------|-----------|--------|
| `ui/public/icons/icon-192.png` | 192×192 | `sips -z 192 192 otto-mark.png` |
| `ui/public/icons/icon-512.png` | 512×512 | `sips -z 512 512 otto-mark.png` |
| `ui/public/icons/icon-512-maskable.png` | 512×512 | Python/Pillow: mark resized to 410px (80% safe-zone), composited onto solid `#111` background, saved as opaque RGB PNG |
| `ui/public/icons/apple-touch-icon-180.png` | 180×180 | Python/Pillow: mark resized to 144px (80%), composited onto solid `#111` background, saved as opaque RGB PNG (no transparency — iOS requirement) |

Source: `ui/public/otto-mark.png` (1024×1024, RGBA).  
Maskable safe-zone: mark occupies inner 80% of canvas (industry standard; leaves 10% padding on each edge).

---

## `ui/public/manifest.webmanifest`

- `name: "Otto"`, `short_name: "Otto"`
- `start_url: "./"` (hash-router root)
- `display: "standalone"`
- `theme_color: "#111111"`, `background_color: "#111111"` (matches `data-scheme="dark"`)
- `icons`: all four icons listed; `icon-512-maskable.png` has `"purpose": "maskable"`

---

## `ui/public/sw.js` — caching policy

- **`/api/*` and `/ws/*`: always pass-through, never cached** (live daemon traffic).
- Everything else: cache-first for the app shell (pre-caches `/` + `/manifest.webmanifest` on install; stale entries cleaned on activate via `clients.claim` + old-cache deletion).
- `skipWaiting()` on install; `clients.claim()` on activate.
- Only caches same-origin, status-200, GET responses.

---

## `ui/index.html` head additions

```html
<link rel="manifest" href="/manifest.webmanifest" />
<meta name="theme-color" content="#111111" />
<meta name="apple-mobile-web-app-capable" content="yes" />
<meta name="apple-mobile-web-app-status-bar-style" content="black-translucent" />
<meta name="apple-mobile-web-app-title" content="Otto" />
<link rel="apple-touch-icon" href="/icons/apple-touch-icon-180.png" />
```

Existing viewport meta and favicon left intact.

---

## `ui/src/main.ts` — SW registration

```ts
if ('serviceWorker' in navigator) {
  window.addEventListener('load', () => {
    navigator.serviceWorker.register('/sw.js').catch(() => {});
  });
}
```

No-op in dev (sw.js isn't served by Vite dev server from `public/` at the same path in module mode); the `.catch()` swallows the failure silently.

---

## Build + check results

- `cd ui && npm run check` → **0 errors, 0 warnings** (490 files checked)
- `cd ui && npm run build` → **success** (built in ~2.4s)

---

## dist verification (after build)

All assets present in `ui/dist/`:

```
ui/dist/manifest.webmanifest      712 B
ui/dist/sw.js                    1930 B
ui/dist/icons/icon-192.png         41 kB
ui/dist/icons/icon-512.png        155 kB
ui/dist/icons/icon-512-maskable.png  85 kB
ui/dist/icons/apple-touch-icon-180.png  21 kB
```

`ui/dist/index.html` contains all 6 PWA head tags (verified via grep).

---

## Files changed / created

| Path | Action |
|------|--------|
| `ui/public/icons/icon-192.png` | created |
| `ui/public/icons/icon-512.png` | created |
| `ui/public/icons/icon-512-maskable.png` | created |
| `ui/public/icons/apple-touch-icon-180.png` | created |
| `ui/public/manifest.webmanifest` | created |
| `ui/public/sw.js` | created |
| `ui/index.html` | modified (added 6 PWA head tags) |
| `ui/src/main.ts` | modified (SW registration) |

---

## Self-review

- SW correctly excludes `/api` and `/ws` paths from caching (deny-by-path guard, not allow-list, so all API sub-paths are covered).
- Apple-touch-icon and maskable icon both have solid `#111` background (no transparency) as required.
- Maskable icon uses 80% safe-zone padding (industry standard for Android adaptive icons).
- SW registration is guarded by `'serviceWorker' in navigator` (no crash on unsupported browsers/environments).
- `npm run check` clean — no TypeScript or Svelte errors introduced.
- No Rust crates modified — this is a pure UI/public asset task; Rust build unaffected.
- Vite copies `ui/public/` verbatim to `ui/dist/` at build time; rust-embed will bake them in on the next `cargo build --features embed-ui`.

## Concerns / notes

- None blocking. The service worker's pre-cache list is minimal (`/`, `/manifest.webmanifest`); the built JS/CSS bundles are served under hashed filenames so they're added to the cache dynamically on first fetch. This is intentionally simple per spec ("pure pass-through fetch listener is acceptable").
- Chrome's `beforeinstallprompt` fires when: HTTPS + manifest + SW + icons + `display: standalone` are all present. All four are now satisfied.
- iOS "Add to Home Screen" is manual (Share sheet); it uses `apple-touch-icon` + `apple-mobile-web-app-*` meta tags — both now present.
