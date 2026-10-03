// Module pages, one lazily loaded chunk each (r3-09-01).
//
// The shell (App.svelte) used to import every page statically, so every
// document — the main window, each pop-out and the side-by-side pane — parsed
// the whole app (~8 MB JS) before its first paint. Now the shell chrome is the
// only eager code and each page is its own chunk:
//
// - A pop-out / side pane loads just the module it shows (plus whatever it
//   navigates to).
// - The main window warms the user's top visible pages at idle after its
//   first paint (`prefetchPagesWhenIdle`, planned by lib/prefetchPlan.ts —
//   not on a phone / remote daemon) and any page on sidebar hover/focus
//   (`installNavPrefetch`), so a click normally finds its chunk evaluated.
// - A navigation never shows a blank or a spinner: the router holds the
//   CURRENT route (and so the current page) until the next page's chunk is in
//   (`router.setPrepare`, installed by the shell). The URL moves at once; the
//   page swaps in the same frame its code is ready.
//
// This file must stay tiny and import no page: the entry (App.svelte at the
// root) uses it to start the first page's chunk alongside the shell's.

import type { Component } from 'svelte';
import { SvelteMap } from 'svelte/reactivity';

// eslint-disable-next-line @typescript-eslint/no-explicit-any
type PageComponent = Component<any>;
type Loader = () => Promise<{ default: PageComponent }>;

/** Page key → its chunk. A key is the route's first segment, except the
 *  aliases resolved by {@link pageKeyOf}. Order = idle prefetch order (the
 *  sidebar's default order, most-used first). */
const LOADERS: Record<string, Loader> = {
  // The right activity panel belongs to the Agents page (the only page that
  // shows it) and starts loading with it: its Git/Files/Browser/API tabs pull
  // in CodeMirror, xterm and the git views — too much for the shell's closure,
  // and too much to hold the default route's first paint for (~540 KB). The
  // Drawer shows a placeholder until `loadedRightPanel()` lands.
  agents: () => {
    void ensureRightPanel();
    return import('../modules/agents/AgentsPage.svelte');
  },
  home: () => import('../modules/home/HomePage.svelte'),
  git: () => import('../modules/git/GitPage.svelte'),
  database: () => import('../modules/database/DatabasePage.svelte'),
  vault: () => import('../modules/vault/VaultPage.svelte'),
  api: () => import('../modules/api/ApiPage.svelte'),
  history: () => import('../modules/agents/history/HistoryPage.svelte'),
  assistant: () => import('../modules/assistant/AssistantPage.svelte'),
  'mission-control': () => import('../modules/mission-control/MissionControlPage.svelte'),
  rooms: () => import('./RoomsRoute.svelte'),
  workflows: () => import('../modules/workflows/WorkflowsPage.svelte'),
  'scheduled-tasks': () => import('../modules/scheduled-tasks/ScheduledTasksPage.svelte'),
  'personal-agents': () => import('../modules/personal-agents/PersonalAgentsPage.svelte'),
  loops: () => import('../modules/loops/LoopsPage.svelte'),
  swarm: () => import('../modules/swarm/SwarmPage.svelte'),
  product: () => import('../modules/product/ProductPage.svelte'),
  design: () => import('../modules/design-hall/DesignHallPage.svelte'),
  canvas: () => import('../modules/canvas/CanvasPage.svelte'),
  browser: () => import('../modules/browser/BrowserView.svelte'),
  'run-with-otto': () => import('../modules/run-with-otto/RunWithOttoPage.svelte'),
  mcp: () => import('../modules/mcp/McpPage.svelte'),
  brokers: () => import('../modules/brokers/BrokersPage.svelte'),
  kubernetes: () => import('../modules/kubernetes/KubernetesPage.svelte'),
  aws: () => import('../modules/aws/AwsPage.svelte'),
  insights: () => import('../modules/insights/InsightsPage.svelte'),
  usage: () => import('../modules/usage/UsagePage.svelte'),
  'skills-eval': () => import('../modules/skills-lab/SkillsLabPage.svelte'),
  proof: () => import('../modules/proof/ProofPage.svelte'),
  settings: () => import('../modules/settings/Settings.svelte'),
  walkthroughs: () => import('../modules/help/Walkthroughs.svelte'),
  plugin: () => import('./PluginRoute.svelte'),
  snip: () => import('../modules/snip/SnipEditor.svelte'),
};

let rightPanel = $state<PageComponent | null>(null);
let rightPanelLoad: Promise<void> | null = null;

/** Load (once) the shell's right activity panel (RightPanel.svelte). */
function loadRightPanel(): Promise<void> {
  return (rightPanelLoad ??= import('./RightPanel.svelte').then(
    (m) => {
      rightPanel = m.default;
    },
    (e: unknown) => {
      rightPanelLoad = null;
      throw e;
    },
  ));
}

/** Start (once) the right panel's chunk; never throws — a failed load is
 *  forgotten, so the next call (the shell re-asks while it's shown) retries. */
export function ensureRightPanel(): Promise<void> {
  return loadRightPanel().catch(() => {});
}

/** The right activity panel once loaded (reactive), else null. */
export function loadedRightPanel(): PageComponent | null {
  return rightPanel;
}

/** Routes the shell never renders as a page (the root App handles them). */
const NOT_SHELL = new Set(['s', 'bar', 'tray', 'room', 'room-host']);

/** The page a route shows, or null for a route outside the shell. Mirrors the
 *  shell's old `{#if moduleName === …}` chain: `connections` is the Database
 *  page, a plugin route without a slug and any unknown module are Agents. */
export function pageKeyOf(parts: readonly string[]): string | null {
  const m = parts[0] ?? '';
  if (NOT_SHELL.has(m)) return null;
  if (m === 'connections') return 'database';
  if (m === 'plugin' && !parts[1]) return 'agents';
  return m in LOADERS ? m : 'agents';
}

/** Resolved pages (reactive: the shell re-renders when one lands). */
const resolved = new SvelteMap<string, PageComponent>();
/** Pages whose chunk failed to load (reactive, for the inline error). */
const failed = new SvelteMap<string, string>();
/** One promise per page, so prefetch, hover and navigation share a request. */
const pending = new Map<string, Promise<PageComponent>>();

/** The page component when its chunk is in, else undefined. */
export function loadedPage(key: string): PageComponent | undefined {
  return resolved.get(key);
}

/** Why the page's chunk failed to load, if it did. */
export function pageError(key: string): string | undefined {
  return failed.get(key);
}

/** Load (once) and return a page's component. A failed load is forgotten so
 *  a later call (Retry, the next hover) tries again. */
export function loadPage(key: string): Promise<PageComponent> {
  const hit = resolved.get(key);
  if (hit) return Promise.resolve(hit);
  let p = pending.get(key);
  if (!p) {
    const load = LOADERS[key];
    if (!load) return Promise.reject(new Error(`no page "${key}"`));
    failed.delete(key);
    p = load().then(
      (m) => {
        resolved.set(key, m.default);
        pending.delete(key);
        return m.default;
      },
      (e: unknown) => {
        pending.delete(key);
        failed.set(key, e instanceof Error ? e.message : String(e));
        throw e;
      },
    );
    pending.set(key, p);
  }
  return p;
}

/** Warm a route's page; never throws. */
export function preloadRoute(parts: readonly string[]): Promise<void> {
  const key = pageKeyOf(parts);
  return key ? loadPage(key).then(() => undefined, () => undefined) : Promise.resolve();
}

/** The router's prepare hook: null when the route can render now, else the
 *  promise the router waits on before it switches (see router.setPrepare). */
export function prepareRoute(parts: readonly string[]): Promise<void> | null {
  const key = pageKeyOf(parts);
  if (!key || resolved.has(key)) return null;
  return preloadRoute(parts);
}

// ── background prefetch ─────────────────────────────────────────────────────

type IdleWindow = Window & {
  requestIdleCallback?: (cb: () => void, opts?: { timeout: number }) => number;
  cancelIdleCallback?: (id: number) => void;
};

/** Input-quiet window the timer fallback waits for (perf F9). */
const INPUT_QUIET_MS = 800;
/** `performance.now()` of the last click / key / wheel in this document. */
let lastInput = -Infinity;
if (typeof window !== 'undefined') {
  const note = (): void => {
    lastInput = performance.now();
  };
  for (const type of ['pointerdown', 'keydown', 'wheel'] as const) {
    window.addEventListener(type, note, { capture: true, passive: true });
  }
}

/** Run `cb` when the main thread is idle. WKWebView has no
 *  requestIdleCallback, so it falls back to a short timer — deferred while
 *  the person is clicking or typing, so a page chunk's parse/evaluate never
 *  competes with their first interactions after boot. */
function whenIdle(cb: () => void, timeout: number): () => void {
  const w = window as IdleWindow;
  if (w.requestIdleCallback) {
    const id = w.requestIdleCallback(cb, { timeout });
    return () => w.cancelIdleCallback?.(id);
  }
  let t: ReturnType<typeof setTimeout>;
  const attempt = (): void => {
    const quiet = performance.now() - lastInput;
    if (quiet < INPUT_QUIET_MS) {
      t = setTimeout(attempt, INPUT_QUIET_MS - quiet);
      return;
    }
    cb();
  };
  t = setTimeout(attempt, 120);
  return () => clearTimeout(t);
}

/**
 * Warm `keys` (see lib/prefetchPlan.ts — the visible modules, capped) after
 * the first paint, one chunk per idle slot so a prefetch never lands as one
 * long frame. Returns a stop function. The main window only: a pop-out or
 * side pane shows one module, so it loads just that one (hover prefetch still
 * covers anything it links to).
 */
export function prefetchPagesWhenIdle(keys: readonly string[], startDelayMs = 400): () => void {
  const queue = keys.filter((k) => k in LOADERS && k !== 'snip' && k !== 'plugin');
  let cancel: (() => void) | null = null;
  let stopped = false;
  const next = (): void => {
    if (stopped) return;
    const key = queue.shift();
    if (!key) return;
    if (resolved.has(key)) return next();
    cancel = whenIdle(() => {
      void loadPage(key)
        .catch(() => {})
        .then(() => next());
    }, 2000);
  };
  const t = setTimeout(next, startDelayMs);
  return () => {
    stopped = true;
    clearTimeout(t);
    cancel?.();
  };
}

/** The route id a sidebar/bottom-nav control navigates to (`data-nav-id`). */
function navIdAt(target: EventTarget | null): string | null {
  const el = (target as Element | null)?.closest?.('[data-nav-id]');
  return el ? (el as HTMLElement).dataset.navId ?? null : null;
}

/** Hover/focus on any `[data-nav-id]` control starts that page's chunk, so a
 *  click ~100 ms later finds it ready even before the idle prefetch got there. */
export function installNavPrefetch(): () => void {
  const warm = (e: Event): void => {
    const id = navIdAt(e.target);
    if (id) void preloadRoute(id.split('/'));
  };
  document.addEventListener('pointerover', warm, { passive: true });
  document.addEventListener('focusin', warm);
  return () => {
    document.removeEventListener('pointerover', warm);
    document.removeEventListener('focusin', warm);
  };
}
