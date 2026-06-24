<script lang="ts">
  // The real Excalidraw editor, embedded. Excalidraw is React, so we mount it
  // React-in-Svelte: a host div + `createRoot` + `createElement(Excalidraw)`.
  // React + Excalidraw (~1.2MB) are DYNAMICALLY imported so they only load when
  // a canvas is actually opened, not in the main bundle.
  //
  // Persistence: the open scene's `doc_json` holds Excalidraw's
  // `{elements, appState, files}` (the server stores it opaquely). We debounce-
  // save on change and one last time on unmount. Excalidraw brings its own
  // toolbar, styles panel, shape/icon libraries and PNG/SVG export for free.
  import { onMount, onDestroy } from 'svelte';
  import { canvas } from '../../lib/stores/canvas.svelte';
  import { ui } from '../../lib/stores/ui.svelte';
  import { api } from '../../lib/api/client';
  import { toasts } from '../../lib/toast.svelte';

  interface Props {
    readonly?: boolean;
  }
  let { readonly = false }: Props = $props();

  let host = $state<HTMLDivElement | null>(null);
  // React / Excalidraw runtime handles — loaded dynamically, so untyped here.
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  let root: any = null;
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  let excaliApi: any = null;
  let saveTimer: ReturnType<typeof setTimeout> | null = null;
  let destroyed = false;

  function scheduleSave(): void {
    if (readonly) return;
    if (saveTimer) clearTimeout(saveTimer);
    saveTimer = setTimeout(() => void saveNow(), 700);
  }

  async function saveNow(): Promise<void> {
    const id = canvas.currentId;
    if (!excaliApi || !id) return;
    const elements = excaliApi.getSceneElements();
    const appState = excaliApi.getAppState();
    const files = excaliApi.getFiles?.() ?? {};
    const doc = {
      type: 'excalidraw',
      version: 2,
      source: 'otto',
      elements,
      // Only persist the durable bits of appState — not transient UI/selection.
      appState: {
        viewBackgroundColor: appState.viewBackgroundColor,
        gridSize: appState.gridSize ?? null,
      },
      files,
    };
    try {
      await api.put(`/canvas/scenes/${id}`, { doc });
      canvas.markSaved(doc);
    } catch (e) {
      toasts.error('Canvas save failed', e instanceof Error ? e.message : String(e));
    }
  }

  // Build Excalidraw initialData from the opaque doc. Tolerates our older
  // Scene-shaped docs (no `elements`) by starting empty.
  function initialData() {
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    const raw = canvas.rawDoc as any;
    const elements = Array.isArray(raw?.elements) ? raw.elements : [];
    return {
      elements,
      appState: {
        viewBackgroundColor: raw?.appState?.viewBackgroundColor ?? '#ffffff',
      },
      files: raw?.files ?? {},
      scrollToContent: elements.length > 0,
    };
  }

  onMount(async () => {
    // Point Excalidraw at its bundled fonts/locales. (CDN for now — a later pass
    // self-hosts these for full offline support in the desktop app.)
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    const w = window as any;
    if (!w.EXCALIDRAW_ASSET_PATH) {
      w.EXCALIDRAW_ASSET_PATH = 'https://unpkg.com/@excalidraw/excalidraw@0.18.1/dist/prod/';
    }
    const React = await import('react');
    const { createRoot } = await import('react-dom/client');
    const Ex = await import('@excalidraw/excalidraw');
    await import('@excalidraw/excalidraw/index.css');
    if (destroyed || !host) return;
    root = createRoot(host);
    root.render(
      React.createElement(Ex.Excalidraw, {
        // eslint-disable-next-line @typescript-eslint/no-explicit-any
        excalidrawAPI: (a: any) => (excaliApi = a),
        initialData: initialData(),
        onChange: scheduleSave,
        theme: ui.resolvedScheme,
        name: canvas.scene?.title ?? 'Canvas',
        viewModeEnabled: readonly,
        UIOptions: { canvasActions: { loadScene: false } },
      }),
    );
  });

  onDestroy(() => {
    destroyed = true;
    if (saveTimer) {
      clearTimeout(saveTimer);
      void saveNow();
    }
    try {
      root?.unmount();
    } catch {
      /* ignore */
    }
    root = null;
    excaliApi = null;
  });
</script>

<div class="excali" bind:this={host}></div>

<style>
  .excali {
    width: 100%;
    height: 100%;
    min-height: 0;
    position: relative;
  }
  /* Excalidraw renders its own full chrome — make it fill the host. */
  .excali :global(.excalidraw) {
    height: 100%;
  }
</style>
