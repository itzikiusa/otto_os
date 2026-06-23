import { mount } from 'svelte';
import './lib/tokens.css';
import './app.css';
// Svelte Flow base styles for the Canvas Studio module (loaded once, globally).
import '@xyflow/svelte/dist/style.css';
// Cousine: a monospace (Croscore) font with proper Hebrew glyphs. Used as the
// terminal's Hebrew fallback so RTL text renders crisp & aligned instead of
// falling back to a non-mono system font. latin.css enables it as a full
// primary option in the terminal font picker.
import '@fontsource/cousine/latin.css';
import '@fontsource/cousine/hebrew.css';
import App from './App.svelte';
import { mockEnabled, setupMock } from './lib/api/mock';
import { setToken, getToken } from './lib/api/client';

if (mockEnabled()) {
  setupMock();
  // mock auth: ensure a token exists so the shell loads straight away
  if (!getToken()) setToken('mock-token');
}

const app = mount(App, {
  target: document.getElementById('app')!,
});

// Register the PWA service worker (no-op in dev if sw.js isn't served).
// When a NEW service worker takes control (after a deploy), reload once so the
// fresh app shell is shown immediately — otherwise a cached SW can keep serving
// a stale build until the user manually clears site data.
if ('serviceWorker' in navigator) {
  let reloadingForSw = false;
  navigator.serviceWorker.addEventListener('controllerchange', () => {
    if (reloadingForSw) return;
    reloadingForSw = true;
    window.location.reload();
  });
  window.addEventListener('load', () => {
    navigator.serviceWorker
      .register('/sw.js')
      .then((reg) => {
        // Proactively check for an updated SW on each load.
        void reg.update().catch(() => {});
      })
      .catch(() => {});
  });
}

export default app;
