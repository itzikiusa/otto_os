// Pre-paint appearance (review 06 F8). Loaded as a classic, render-blocking
// script from <head> so the FIRST paint already has the user's theme, light/
// dark scheme and text direction — without it a light-scheme user saw a dark
// flash and an RTL user saw LTR until the JS entry ran ui.applyTheme().
// An external file, not an inline <script>: the desktop CSP is script-src
// 'self'. Mirrors the keys + resolution in lib/stores/ui.svelte.ts
// (applyTheme stays the full pass: accent, ambient, transparency). Keep it
// tiny and stable — the service worker serves static files cache-first.
(function () {
  try {
    var ls = window.localStorage;
    var theme = ls.getItem('otto_theme') || 'native';
    var scheme = ls.getItem('otto_scheme') || 'auto';
    var dir = ls.getItem('otto_direction') === 'rtl' ? 'rtl' : 'ltr';
    var resolved =
      theme === 'pro-dark'
        ? 'dark'
        : scheme === 'light' || scheme === 'dark'
          ? scheme
          : window.matchMedia && window.matchMedia('(prefers-color-scheme: dark)').matches
            ? 'dark'
            : 'light';
    var el = document.documentElement;
    el.setAttribute('data-theme', theme);
    el.setAttribute('data-scheme', resolved);
    el.dir = dir;
    var meta = document.querySelector('meta[name="theme-color"]');
    if (meta) meta.setAttribute('content', resolved === 'light' ? '#f5f5f7' : '#111111');
  } catch (e) {
    /* storage blocked: keep the static defaults */
  }
})();
