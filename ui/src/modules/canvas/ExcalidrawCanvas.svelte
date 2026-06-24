<script module lang="ts">
  // The currently-mounted Excalidraw API. Held at MODULE scope (only one canvas
  // is open at a time) so an IN-FLIGHT generate() survives a component remount
  // during the possibly-long agent call — otherwise the instance `excaliApi` is
  // nulled by onDestroy and the resumed generate hits a dead reference (the
  // "null is not an object (s.getSceneElements)" crash).
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  let liveApi: any = null;
</script>

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
  // Excalidraw's `convertToExcalidrawElements`, captured once the module loads.
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  let convert: ((skeleton: any[]) => any[]) | null = null;
  let saveTimer: ReturnType<typeof setTimeout> | null = null;
  let destroyed = false;
  let generating = $state(false);

  /** Agent generation → editable shapes. Asks the agent for a diagram (Mermaid),
   *  converts it to NATIVE Excalidraw elements (mermaid-to-excalidraw), drops them
   *  to the right of any existing content, and auto-fits the view to them. */
  export async function generate(prompt: string): Promise<void> {
    const p = prompt.trim();
    if (!p || generating || !liveApi || !convert) return;
    generating = true;
    try {
      const res = await canvas.assist(p, 'flow');
      // Re-read the LIVE Excalidraw API after the (possibly long) agent call — the
      // component may have remounted while we waited; draw to whatever's mounted.
      const ex = liveApi;
      if (!ex) {
        toasts.error('Canvas was reloaded', 'Open the canvas and try the draw again.');
        return;
      }
      // Prefer the agent's Excalidraw element JSON (true code blocks, icons,
      // frames); fall back to Mermaid → mermaid-to-excalidraw for clean
      // auto-layout flowcharts. Both end as an Excalidraw element SKELETON that
      // convertToExcalidrawElements turns into real, editable elements.
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      const xd = (res as any).excalidraw;
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      let skeleton: any[] | null = null;
      if (xd) {
        skeleton = Array.isArray(xd) ? xd : Array.isArray(xd.elements) ? xd.elements : null;
      }
      if (!skeleton && res.mermaid) {
        const { parseMermaidToExcalidraw } = await import('@excalidraw/mermaid-to-excalidraw');
        const parsed = await parseMermaidToExcalidraw(res.mermaid);
        // eslint-disable-next-line @typescript-eslint/no-explicit-any
        skeleton = parsed.elements as any[];
      }
      if (!skeleton || !skeleton.length) {
        toasts.info('Nothing to draw', res.note || 'The agent did not return a diagram.');
        return;
      }
      const fresh = convert(skeleton);
      if (!fresh.length) {
        toasts.info('Nothing to draw', 'The diagram came back empty.');
        return;
      }
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      const existing = ex.getSceneElements() as any[];
      let dx = 0;
      if (existing.length) {
        const maxX = Math.max(...existing.map((e) => e.x + (e.width ?? 0)));
        const minX = Math.min(...fresh.map((e) => e.x));
        dx = maxX + 100 - minX;
      }
      const placed = dx ? fresh.map((e) => ({ ...e, x: e.x + dx })) : fresh;

      // LIVE BUILD-IN: reveal shapes first (so arrows have something to bind to),
      // then arrows — a few per frame, so the diagram "draws itself" instead of
      // popping in all at once. Frame the FINAL bounds up front so the camera
      // doesn't jump while it builds.
      const arrows = placed.filter((e) => e.type === 'arrow' || e.type === 'line');
      const shapes = placed.filter((e) => e.type !== 'arrow' && e.type !== 'line');
      const ordered = [...shapes, ...arrows];
      ex.updateScene({ elements: [...existing, ordered[0]] });
      ex.scrollToContent(placed, { fitToContent: true, animate: true });
      const step = Math.max(1, Math.ceil(ordered.length / 22));
      for (let i = 1; i < ordered.length; i += step) {
        if (liveApi !== ex) break; // the canvas changed under us
        ex.updateScene({ elements: [...existing, ...ordered.slice(0, i + step)] });
        await new Promise((r) => setTimeout(r, 55));
      }
      ex.updateScene({ elements: [...existing, ...ordered] });
      scheduleSave();
      // Surface the agent's session so "View conversation" can show its work.
      void canvas.refreshSession();
      toasts.success('Drawn on canvas', res.note || 'Editable shapes added.');
    } catch (e) {
      toasts.error('Ask AI failed', e instanceof Error ? e.message : String(e));
    } finally {
      generating = false;
    }
  }
  export function isGenerating(): boolean {
    return generating;
  }

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
    convert = Ex.convertToExcalidrawElements;
    if (destroyed || !host) return;
    root = createRoot(host);
    root.render(
      React.createElement(Ex.Excalidraw, {
        // eslint-disable-next-line @typescript-eslint/no-explicit-any
        excalidrawAPI: (a: any) => {
          excaliApi = a;
          liveApi = a;
        },
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
    if (liveApi === excaliApi) liveApi = null;
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
