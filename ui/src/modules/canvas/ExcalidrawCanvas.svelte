<script module lang="ts">
  // The currently-mounted Excalidraw API, held at MODULE scope (only one canvas
  // is open) so an in-flight generate() survives a brief component remount.
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  let liveApi: any = null;
</script>

<script lang="ts">
  // Plain embedded Excalidraw. Excalidraw is React, so we mount it React-in-Svelte
  // (host div + createRoot + createElement). The scene's SOURCE is a per-scene
  // `canvas.json` Excalidraw scene the agent edits; here we:
  //   source → board   parse + normalise — a full saved doc goes through
  //                    restoreElements (rescuing collapsed labels + routing
  //                    id-only arrows); the agent's simplified form is BUILT
  //                    ourselves (buildExcalidrawElements) with controlled
  //                    geometry + centred labels — never the stock converter,
  //                    which scattered labels to (0,0).
  //   board → source   on ANY manual edit, autosave the FULL Excalidraw scene
  //                    straight back to `canvas.json` (the same file the agent
  //                    edits) — so the user's hand edits update the json too.
  // Agent edits arrive live over `canvas_updated` and reload in place.
  import { onMount, onDestroy } from 'svelte';
  import { canvas } from '../../lib/stores/canvas.svelte';
  import { canvasDocBus } from '../../lib/events.svelte';
  import { ui } from '../../lib/stores/ui.svelte';
  import { api } from '../../lib/api/client';
  import { toasts } from '../../lib/toast.svelte';
  import type { CanvasDoc } from './types';
  import { buildExcalidrawElements, isSimplified } from './excalidraw-build';

  interface Props {
    readonly?: boolean;
  }
  let { readonly = false }: Props = $props();

  // The scene THIS editor is mounted for. Captured once (the component is keyed by
  // currentId, so it remounts per scene). Saves target THIS id — never
  // canvas.currentId, which on a scene switch already points at the NEXT scene and
  // would write this Excalidraw doc into a Mermaid scene (corruption).
  const sceneId = canvas.currentId;

  let host = $state<HTMLDivElement | null>(null);
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  let root: any = null;
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  let excaliApi: any = null;
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  let restore: ((els: any[], local: any) => any[]) | null = null;
  let saveTimer: ReturnType<typeof setTimeout> | null = null;
  let destroyed = false;
  let generating = $state(false);
  let suppressSave = false;
  let lastApplied = '';

  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  function center(e: any): { x: number; y: number } {
    return {
      x: (Number(e.x) || 0) + (Number(e.width) || 0) / 2,
      y: (Number(e.y) || 0) + (Number(e.height) || 0) / 2,
    };
  }

  // Route arrows that the agent left as id-only (`start:{id}`/`end:{id}` with no
  // geometry) — without this they (and their labels) collapse onto (0,0) when the
  // scene goes through restoreElements (which doesn't auto-route). Give each such
  // arrow explicit x/y + points between the two nodes' centres + bindings, and pull
  // any bound text onto the arrow midpoint.
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  function routeArrows(els: any[]): any[] {
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    const byId = new Map<string, any>();
    for (const e of els) if (e && typeof e.id === 'string') byId.set(e.id, e);
    for (const a of els) {
      if (!a || a.type !== 'arrow') continue;
      const last = Array.isArray(a.points) ? a.points[a.points.length - 1] : null;
      const hasGeom = last && (Math.abs(last[0]) > 1 || Math.abs(last[1]) > 1);
      const sid = a.start?.id ?? a.startBinding?.elementId;
      const eid = a.end?.id ?? a.endBinding?.elementId;
      if (hasGeom || !sid || !eid) continue;
      const s = byId.get(sid);
      const t = byId.get(eid);
      if (!s || !t) continue;
      const sc = center(s);
      const tc = center(t);
      a.x = Math.round(sc.x);
      a.y = Math.round(sc.y);
      a.points = [
        [0, 0],
        [Math.round(tc.x - sc.x), Math.round(tc.y - sc.y)],
      ];
      a.width = Math.abs(tc.x - sc.x);
      a.height = Math.abs(tc.y - sc.y);
      a.startBinding = { elementId: sid, focus: 0, gap: 4 };
      a.endBinding = { elementId: eid, focus: 0, gap: 4 };
      delete a.start;
      delete a.end;
    }
    // RESCUE bound text (labels) that collapsed to ~(0,0): re-centre every bound
    // text on its container (arrow midpoint, or shape centre). Fixes legacy full
    // docs that an earlier converter scattered.
    for (const e of els) {
      if (!e || e.type !== 'text' || typeof e.containerId !== 'string') continue;
      const c = byId.get(e.containerId);
      if (!c) continue;
      const w = Number(e.width) || 0;
      const h = Number(e.height) || 0;
      if (c.type === 'arrow' && Array.isArray(c.points)) {
        const p = c.points[c.points.length - 1] ?? [0, 0];
        e.x = Math.round((Number(c.x) || 0) + p[0] / 2 - w / 2);
        e.y = Math.round((Number(c.y) || 0) + p[1] / 2 - h / 2);
      } else if (Number.isFinite(c.width) && Number.isFinite(c.height)) {
        e.x = Math.round((Number(c.x) || 0) + (Number(c.width) || 0) / 2 - w / 2);
        e.y = Math.round((Number(c.y) || 0) + (Number(c.height) || 0) / 2 - h / 2);
      }
    }
    return els;
  }

  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  function safeRestore(arr: any[]): any[] {
    if (!restore) return arr;
    try {
      return restore(arr, null);
    } catch (err) {
      // eslint-disable-next-line no-console
      console.error('[canvas] restoreElements failed:', err);
      return arr;
    }
  }

  // Turn a stored / agent-written scene into valid Excalidraw elements.
  //   simplified (the agent's form, no internals) → BUILD it ourselves with
  //     controlled geometry + centred bound labels (never trusts the converter,
  //     which scattered labels to 0,0).
  //   full (re-loading a saved doc) → rescue any origin-collapsed labels + route
  //     id-only arrows, then restoreElements.
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  function normalizeScene(raw: any): any[] {
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    const els: any[] = Array.isArray(raw)
      ? raw
      : Array.isArray(raw?.elements)
        ? raw.elements
        : [];
    if (!els.length) return [];
    if (isSimplified(els)) {
      try {
        return safeRestore(buildExcalidrawElements(els));
      } catch (err) {
        // eslint-disable-next-line no-console
        console.error('[canvas] buildExcalidrawElements failed:', err);
      }
    }
    return safeRestore(routeArrows(els));
  }

  // Load a source string into the live editor, replacing the scene + auto-fit.
  function loadScene(source: string): void {
    const ex = liveApi;
    if (!ex) return;
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    let raw: any = null;
    try {
      raw = JSON.parse(source);
    } catch {
      raw = null;
    }
    const elements = normalizeScene(raw);
    suppressSave = true;
    try {
      ex.updateScene({ elements });
      if (elements.length) ex.scrollToContent(elements, { fitToContent: true, animate: false });
    } finally {
      setTimeout(() => {
        suppressSave = false;
      }, 160);
    }
    lastApplied = source;
  }

  /** Ask the agent to edit this scene's canvas.json. The server commits it +
   *  streams the edit over canvas_updated; the source effect reloads it. */
  export async function generate(prompt: string): Promise<void> {
    const p = prompt.trim();
    if (!p || generating) return;
    generating = true;
    try {
      const res = await canvas.assist(p);
      const src = (res as { excalidraw?: unknown }).excalidraw != null
        ? JSON.stringify((res as { excalidraw?: unknown }).excalidraw)
        : '';
      if (!src) {
        toasts.info('Nothing to draw', res.note || 'The agent did not return a diagram.');
        return;
      }
      canvas.ingestDoc({ type: 'otto-canvas', version: 1, format: 'excalidraw', source: src });
      toasts.success('Drawn on canvas', res.note || 'Diagram updated.');
      void canvas.refreshSession();
    } catch (e) {
      toasts.error('Ask AI failed', e instanceof Error ? e.message : String(e));
    } finally {
      generating = false;
    }
  }
  export function isGenerating(): boolean {
    return generating;
  }

  // source → board: reload when the source changes from the store (agent edit /
  // live / generate). Skips our own just-saved source (lastApplied).
  $effect(() => {
    const src = canvas.source ?? '';
    if (src !== lastApplied) loadScene(src);
  });

  // live agent edits → store (the source effect above does the reload).
  $effect(() => {
    const _t = canvasDocBus.tick;
    if (!_t || canvasDocBus.sceneId !== canvas.currentId) return;
    const doc = canvasDocBus.doc as CanvasDoc | null;
    if (doc && typeof doc.source === 'string') canvas.ingestDoc(doc);
  });

  function scheduleSave(): void {
    if (readonly || suppressSave) return;
    if (saveTimer) clearTimeout(saveTimer);
    saveTimer = setTimeout(() => void saveNow(), 700);
  }

  async function saveNow(): Promise<void> {
    if (!excaliApi || !sceneId) return;
    const elements = excaliApi.getSceneElements();
    const appState = excaliApi.getAppState();
    const files = excaliApi.getFiles?.() ?? {};
    const scene = {
      type: 'excalidraw',
      version: 2,
      source: 'otto',
      elements,
      appState: {
        viewBackgroundColor: appState.viewBackgroundColor,
        gridSize: appState.gridSize ?? null,
      },
      files,
    };
    const str = JSON.stringify(scene);
    // Save to THIS editor's scene (always excalidraw). Only sync the store when
    // it's still the open scene — otherwise a scene switch would clobber the new
    // scene's state with this one's Excalidraw doc.
    if (canvas.currentId === sceneId) lastApplied = str; // don't bounce back through the source effect
    const doc = { type: 'otto-canvas', version: 1, format: 'excalidraw', source: str };
    try {
      await api.put(`/canvas/scenes/${sceneId}`, { doc });
      // Re-check AFTER the await: a scene switch may have landed WHILE the PUT was
      // in flight (the await is the yield point). Syncing `canvas.source` with this
      // stale flag would write Excalidraw JSON into the now-open Mermaid scene —
      // exactly the corruption we must avoid. Read currentId fresh here.
      if (canvas.currentId === sceneId) {
        canvas.source = str;
        canvas.markSaved(doc);
      }
    } catch (e) {
      if (canvas.currentId === sceneId)
        toasts.error('Canvas save failed', e instanceof Error ? e.message : String(e));
    }
  }

  function initialData() {
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    let raw: any = null;
    try {
      raw = canvas.source ? JSON.parse(canvas.source) : null;
    } catch {
      raw = null;
    }
    const elements = normalizeScene(raw);
    lastApplied = canvas.source ?? '';
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
  .excali :global(.excalidraw) {
    height: 100%;
  }
</style>
