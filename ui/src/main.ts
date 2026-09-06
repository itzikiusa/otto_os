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
import { setToken, getToken, baseUrl } from './lib/api/client';

if (mockEnabled()) {
  setupMock();
  // mock auth: ensure a token exists so the shell loads straight away
  if (!getToken()) setToken('mock-token');
}

// OS text prediction / autocorrect (the macOS WebKit candidate bubble) is
// noise in an app where people type paths, flags, identifiers and slash
// commands — switch it off on every text field as it gains focus, so new
// inputs anywhere in the app inherit the rule without per-field attributes.
document.addEventListener(
  'focusin',
  (e) => {
    const el = e.target;
    if (!(el instanceof HTMLTextAreaElement) && !(el instanceof HTMLInputElement)) return;
    if (el instanceof HTMLInputElement && !['text', 'search', 'url', 'email', ''].includes(el.type)) return;
    if (el.dataset.keepAutocorrect === 'true') return;
    el.setAttribute('autocorrect', 'off');
    el.setAttribute('autocapitalize', 'off');
    el.spellcheck = false;
    if (!el.getAttribute('autocomplete')) el.setAttribute('autocomplete', 'off');
  },
  true,
);

// ── Last-resort self-heal ────────────────────────────────────────────────────
// Svelte's `effect_update_depth_exceeded` (an effect that keeps re-scheduling
// itself) throws out of the scheduler and ABORTS the reactive flush: stores
// keep updating but nothing re-renders, every button looks dead, and the only
// way out was a manual reload. Seen live twice on the chat view, both times
// right after a connectivity blip (daemon unreachable → sockets dropped →
// reconnect storm), never reproduced in dev mode. Until the looping effect is
// found: record it (daemon log + localStorage), then reload ONCE — the hash
// route, composer drafts (sessionStorage) and view choices (localStorage) all
// survive a reload, so the user lands back where they were with a live UI.
// A second hit within RELOAD_GUARD_MS is NOT reloaded (no reload loops); it is
// still reported.
const CRASH_KEY = 'otto_ui_last_crash';
const RELOAD_STAMP = 'otto_ui_crash_reload_at';
const RELOAD_GUARD_MS = 60_000;
let healing = false;

function crashKind(msg: string): string | null {
  if (/effect_update_depth_exceeded/.test(msg)) return 'effect_loop';
  if (/Maximum call stack size exceeded/.test(msg)) return 'stack_overflow';
  return null;
}

function reportFatal(kind: string, message: string, stack: string): void {
  const route = location.hash || '';
  let lastReload = 0;
  try {
    lastReload = Number(sessionStorage.getItem(RELOAD_STAMP) ?? 0);
  } catch {
    /* storage unavailable */
  }
  const reload = !healing && Date.now() - lastReload > RELOAD_GUARD_MS;
  const action = reload ? 'reloaded' : 'reload_suppressed';
  const record = { at: new Date().toISOString(), kind, message, stack, route, action };
  try {
    localStorage.setItem(CRASH_KEY, JSON.stringify(record));
  } catch {
    /* quota / private mode */
  }
  console.error(`[otto] fatal UI error (${kind}) — ${action}`, record);
  // keepalive: the report must survive the reload that follows.
  const token = getToken();
  void fetch(`${baseUrl()}/api/v1/client/errors`, {
    method: 'POST',
    keepalive: true,
    headers: { 'Content-Type': 'application/json', ...(token ? { Authorization: `Bearer ${token}` } : {}) },
    body: JSON.stringify({ kind, message: message.slice(0, 2000), stack: stack.slice(0, 4000), route, action }),
  }).catch(() => {});
  if (!reload) return;
  healing = true;
  try {
    sessionStorage.setItem(RELOAD_STAMP, String(Date.now()));
  } catch {
    /* ignore */
  }
  // Let the report leave and the console line land before the page goes.
  setTimeout(() => window.location.reload(), 250);
}

window.addEventListener('error', (e) => {
  const err = e.error as { message?: string; stack?: string } | undefined;
  const msg = String(err?.message ?? e.message ?? '');
  const kind = crashKind(msg);
  if (kind) reportFatal(kind, msg, String(err?.stack ?? ''));
});
window.addEventListener('unhandledrejection', (e) => {
  const r = e.reason as { message?: string; stack?: string } | undefined;
  const msg = String(r?.message ?? r ?? '');
  const kind = crashKind(msg);
  if (kind) reportFatal(kind, msg, String(r?.stack ?? ''));
});

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
