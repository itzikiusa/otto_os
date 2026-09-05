<script lang="ts">
  // DesignBoard — the arena's Excalidraw board (design §4.2). Excalidraw is
  // React, so it mounts React-in-Svelte (host div + createRoot + createElement),
  // the same island pattern as Canvas' ExcalidrawCanvas — but bound to PROPS, not
  // the canvas store: the arena owns the source, the debounce and the conflict
  // handling; this component only turns `source` into a board and emits the
  // full Excalidraw scene on every manual edit (undebounced) via `onchange`.
  //
  //   source → board   parse + normalise. A full saved doc goes through
  //                    restoreElements; the agent's simplified form (no Excalidraw
  //                    internals) is BUILT with the shared canvas builder so labels
  //                    stay centred (the stock converter scatters them to 0,0).
  //   board → source   `onchange(JSON)` of the FULL scene on ANY manual edit.
  import { onMount, onDestroy } from 'svelte';
  import { ui } from '../../../lib/stores/ui.svelte';
  import { buildExcalidrawElements, isSimplified } from '../../canvas/excalidraw-build';

  interface Props {
    source: string;
    readonly?: boolean;
    onchange?: (source: string) => void;
  }
  let { source, readonly = false, onchange }: Props = $props();

  let host = $state<HTMLDivElement | null>(null);
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  let root: any = null;
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  let excaliApi: any = null;
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  let restore: ((els: any[], local: any) => any[]) | null = null;
  let destroyed = false;
  let suppressChange = false;
  /** The last source we loaded OR emitted — a prop echo of our own scene must
   *  not reload the board (it would reset the selection mid-drag). */
  let lastApplied = '';
  let ready = $state(false);
  let loadError = $state<string | null>(null);

  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  function safeRestore(arr: any[]): any[] {
    if (!restore) return arr;
    try {
      return restore(arr, null);
    } catch (err) {
      // eslint-disable-next-line no-console
      console.error('[design] restoreElements failed:', err);
      return arr;
    }
  }

  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  function parse(src: string): any {
    try {
      return src.trim() ? JSON.parse(src) : null;
    } catch {
      return null;
    }
  }

  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  function normalizeScene(raw: any): any[] {
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    const els: any[] = Array.isArray(raw) ? raw : Array.isArray(raw?.elements) ? raw.elements : [];
    if (!els.length) return [];
    if (isSimplified(els)) {
      try {
        return safeRestore(buildExcalidrawElements(els));
      } catch (err) {
        // eslint-disable-next-line no-console
        console.error('[design] buildExcalidrawElements failed:', err);
      }
    }
    return safeRestore(els);
  }

  /** Load a source string into the live editor, replacing the scene + auto-fit. */
  function loadScene(src: string): void {
    const ex = excaliApi;
    if (!ex) return;
    const raw = parse(src);
    const elements = normalizeScene(raw);
    suppressChange = true;
    try {
      ex.updateScene({
        elements,
        appState: raw?.appState?.viewBackgroundColor
          ? { viewBackgroundColor: raw.appState.viewBackgroundColor }
          : undefined,
      });
      if (raw?.files && typeof ex.addFiles === 'function') {
        ex.addFiles(Object.values(raw.files));
      }
      if (elements.length) ex.scrollToContent(elements, { fitToContent: true, animate: false });
    } finally {
      // Excalidraw fires onChange asynchronously after updateScene; give it a beat.
      setTimeout(() => {
        suppressChange = false;
      }, 160);
    }
    lastApplied = src;
  }

  /** Serialise the live scene → the same shape Canvas saves (`type:excalidraw v2`). */
  function serialize(): string {
    const elements = excaliApi.getSceneElements();
    const appState = excaliApi.getAppState();
    const files = excaliApi.getFiles?.() ?? {};
    return JSON.stringify({
      type: 'excalidraw',
      version: 2,
      source: 'otto',
      elements,
      appState: {
        viewBackgroundColor: appState.viewBackgroundColor,
        gridSize: appState.gridSize ?? null,
      },
      files,
    });
  }

  function handleChange(): void {
    if (readonly || suppressChange || !excaliApi || !onchange) return;
    const str = serialize();
    if (str === lastApplied) return;
    lastApplied = str;
    onchange(str);
  }

  // source → board: reload when the PARENT changes the source (agent edit, live
  // update, template). Skips the echo of our own last emit.
  $effect(() => {
    const src = source ?? '';
    if (ready && src !== lastApplied) loadScene(src);
  });

  function initialData() {
    const raw = parse(source ?? '');
    const elements = normalizeScene(raw);
    lastApplied = source ?? '';
    return {
      elements,
      appState: { viewBackgroundColor: raw?.appState?.viewBackgroundColor ?? '#ffffff' },
      files: raw?.files ?? {},
      scrollToContent: elements.length > 0,
    };
  }

  onMount(async () => {
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    const w = window as any;
    if (!w.EXCALIDRAW_ASSET_PATH) {
      w.EXCALIDRAW_ASSET_PATH = 'https://unpkg.com/@excalidraw/excalidraw@0.18.1/dist/prod/';
    }
    try {
      const React = await import('react');
      const { createRoot } = await import('react-dom/client');
      const Ex = await import('@excalidraw/excalidraw');
      await import('@excalidraw/excalidraw/index.css');
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      restore = (Ex as any).restoreElements ?? null;
      if (destroyed || !host) return;
      root = createRoot(host);
      root.render(
        React.createElement(Ex.Excalidraw, {
          // eslint-disable-next-line @typescript-eslint/no-explicit-any
          excalidrawAPI: (a: any) => {
            excaliApi = a;
            ready = true;
          },
          initialData: initialData(),
          onChange: handleChange,
          theme: ui.resolvedScheme,
          name: 'Design board',
          viewModeEnabled: readonly,
          UIOptions: { canvasActions: { loadScene: false } },
        }),
      );
    } catch (e) {
      loadError = e instanceof Error ? e.message : String(e);
    }
  });

  onDestroy(() => {
    destroyed = true;
    try {
      root?.unmount();
    } catch {
      /* ignore */
    }
    root = null;
    excaliApi = null;
  });
</script>

<div class="design-board" class:readonly bind:this={host}>
  {#if loadError}
    <div class="board-err">Board failed to load: {loadError}</div>
  {:else if !ready}
    <div class="board-loading">Loading board…</div>
  {/if}
</div>

<style>
  .design-board {
    width: 100%;
    height: 100%;
    min-height: 0;
    position: relative;
  }
  .design-board :global(.excalidraw) {
    height: 100%;
  }
  .board-loading,
  .board-err {
    position: absolute;
    inset: 0;
    display: grid;
    place-items: center;
    font-size: 12.5px;
    color: var(--text-dim);
    pointer-events: none;
  }
  .board-err {
    color: #ef4444;
  }
</style>
