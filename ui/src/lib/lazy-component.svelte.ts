// Lazy-loaded Svelte components that render SYNCHRONOUSLY once loaded.
//
// `{#await import('./X.svelte') then m}` builds a fresh promise every time the
// block re-renders, so even with the module cached each mount shows the pending
// branch for a tick (a flash on every Query↔Structure switch). This loader keeps
// ONE promise per component and parks the resolved module in reactive state:
// the first open waits for the chunk, every later one renders in the same frame.
//
//   const Structure = lazyComponent(() => import('./StructureView.svelte'));
//   {#if Structure.component}<Structure.component />{:else}…loading / error…{/if}
//
// Reading `.component` starts the load. `prefetch()` warms the chunk on idle so
// the first open is instant too. A failed load (offline, a stale chunk after an
// update) surfaces as `.error`; `retry()` clears it and loads again.
import type { Component } from 'svelte';

export interface LazyComponent<T extends Component<any>> {
  /** The loaded component, or null while loading / after a failure. Reading starts the load. */
  readonly component: T | null;
  /** Human message of the last failed load, or null. */
  readonly error: string | null;
  /** Start loading without rendering (idle prefetch). */
  prefetch(): void;
  /** Clear a failure and load again. */
  retry(): void;
}

// eslint-disable-next-line @typescript-eslint/no-explicit-any
export function lazyComponent<T extends Component<any>>(loader: () => Promise<{ default: T }>): LazyComponent<T> {
  let mod = $state.raw<T | null>(null);
  let error = $state<string | null>(null);
  let pending: Promise<void> | null = null;
  function load(): void {
    if (mod || pending) return;
    pending = loader().then(
      (m) => {
        mod = m.default;
      },
      (e: unknown) => {
        pending = null;
        error = e instanceof Error ? e.message : String(e);
      },
    );
  }
  return {
    get component() {
      // Starting a load from a template read is safe: the state writes land in a
      // later microtask, never during the render that read it.
      if (!mod && !pending && error === null) queueMicrotask(load);
      return mod;
    },
    get error() {
      return error;
    },
    prefetch: load,
    retry() {
      error = null;
      load();
    },
  };
}

/** Run `fn` when the main thread is idle (requestIdleCallback, else a timeout). */
export function whenIdle(fn: () => void, timeout = 2000): () => void {
  const w = globalThis as typeof globalThis & {
    requestIdleCallback?: (cb: () => void, o?: { timeout: number }) => number;
    cancelIdleCallback?: (h: number) => void;
  };
  if (w.requestIdleCallback) {
    const h = w.requestIdleCallback(fn, { timeout });
    return () => w.cancelIdleCallback?.(h);
  }
  const t = setTimeout(fn, 200);
  return () => clearTimeout(t);
}
