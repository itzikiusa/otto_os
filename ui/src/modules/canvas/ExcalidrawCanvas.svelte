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
  import { onMount, onDestroy, untrack } from 'svelte';
  import { canvas } from '../../lib/stores/canvas.svelte';
  import { canvasDocBus } from '../../lib/events.svelte';
  import { ui } from '../../lib/stores/ui.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import type { CanvasDoc } from './types';
  import { buildExcalidrawElements, isSimplified } from './excalidraw-build';
  import { filesForSave, resolveFiles, sha256Hex } from './canvasFiles';
  import type { ExFile } from './canvasFileRefs';

  interface Props {
    readonly?: boolean;
  }
  let { readonly = false }: Props = $props();

  // The scene THIS editor is mounted for. Captured once (the component is keyed by
  // currentId, so it remounts per scene). Saves target THIS id — never
  // canvas.currentId, which on a scene switch already points at the NEXT scene and
  // would write this Excalidraw doc into a Mermaid scene (corruption).
  const sceneId = canvas.currentId;
  const saveContext = canvas.saveContext;

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
  let pendingDoc: CanvasDoc | null = null;
  // Images out of the autosave body (canvasFiles.ts): file id → sha of every
  // image the server already holds (learned from loaded refs, or after a save
  // that carried it inline lands). Those autosave as refs, not base64.
  const knownFiles = new Map<string, string>();
  const inlineOf = new WeakMap<CanvasDoc, ExFile[]>();

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
    // Mixed: an Ask AI turn rewrites the shapes in the simplified form while the
    // daemon merges the elements it set aside (images, freehand, lines, frames)
    // back in their FULL form — build the former, restore the latter.
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    const isFull = (e: any) => e && (e.versionNonce != null || e.seed != null);
    const full = els.filter(isFull);
    if (full.length && full.length < els.length) {
      try {
        return safeRestore([...routeArrows(full), ...buildExcalidrawElements(els.filter((e) => !isFull(e)))]);
      } catch (err) {
        // eslint-disable-next-line no-console
        console.error('[canvas] mixed scene build failed:', err);
      }
    }
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
      // Re-register the scene's image files (kept across an Ask AI turn) so
      // image elements never render as broken placeholders. Refs resolve via
      // the immutable file route; files the editor already holds aren't
      // fetched again.
      if (raw?.files && typeof raw.files === 'object') {
        const have = new Set(Object.keys(ex.getFiles?.() ?? {}));
        void resolveFiles(raw.files, knownFiles, have).then((files) => {
          if (files.length && liveApi === ex) ex.addFiles?.(files);
        });
      }
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
      // Bound to THIS editor's scene: a switch during the (long) agent turn
      // must not pour the result into the newly-open scene.
      if (canvas.currentId !== sceneId) {
        toasts.success('Ask AI finished', 'The drawing was saved to the scene you asked from.');
        return;
      }
      canvas.ingestDoc({ type: 'otto-canvas', version: 1, format: 'excalidraw', source: src }, sceneId);
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
  // Tracks ONLY the bus tick; the store consumes each push once, so a save
  // (dirty → false) can no longer re-apply a stale push (r3-02-01).
  $effect(() => {
    const t = canvasDocBus.tick;
    untrack(() => canvas.ingestPush(t, canvasDocBus.sceneId, canvasDocBus.doc));
  });

  function snapshotDoc(): CanvasDoc | null {
    if (!excaliApi) return null;
    const appState = excaliApi.getAppState();
    // Images the server holds go as refs — the stringify (and the PUT) stay
    // small however big the pasted screenshots are.
    const { files, inline } = filesForSave(excaliApi.getFiles?.() ?? {}, knownFiles);
    const doc: CanvasDoc = {
      type: 'otto-canvas', version: 1, format: 'excalidraw',
      source: JSON.stringify({
        type: 'excalidraw', version: 2, source: 'otto',
        elements: excaliApi.getSceneElements(),
        appState: { viewBackgroundColor: appState.viewBackgroundColor, gridSize: appState.gridSize ?? null },
        files,
      }),
    };
    if (inline.length) inlineOf.set(doc, inline);
    return doc;
  }

  /** After `doc` landed: the server now holds its inline images — learn
   *  their content address so later autosaves send refs. */
  function learnSaved(doc: CanvasDoc): void {
    const inline = inlineOf.get(doc);
    if (!inline) return;
    inlineOf.delete(doc);
    for (const f of inline) {
      void sha256Hex(f.dataURL)
        .then((sha) => knownFiles.set(f.id, sha))
        .catch(() => {});
    }
  }

  // Excalidraw fires onChange on every drag frame, scroll, zoom and selection.
  // Serializing the whole scene (base64 files included) per call cost ~1.5 ms
  // and a MB of garbage per frame. Instead: a cheap fingerprint (element
  // version sum + file count + background) drops no-op changes, and the
  // snapshot is taken once, when the 700 ms save timer fires (or on unmount).
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  let sceneVersionOf: ((els: readonly any[]) => number) | null = null;
  let lastFingerprint: string | null = null;
  let changePending = false;

  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  function onSceneChange(elements: readonly any[], appState: any, files: any): void {
    const fp = sceneVersionOf
      ? `${sceneVersionOf(elements)}:${files ? Object.keys(files).length : 0}:${appState?.viewBackgroundColor ?? ''}`
      : null;
    const unchanged = fp !== null && fp === lastFingerprint;
    const baseline = lastFingerprint === null && fp !== null;
    lastFingerprint = fp;
    // Loads / agent edits (suppressSave) and the mount's first report only
    // set the baseline; a scroll, zoom or selection is not an edit.
    if (baseline || unchanged) return;
    if (canvas.saveContext !== saveContext || readonly || suppressSave || !sceneId) return;
    changePending = true;
    if (canvas.currentId === sceneId) canvas.dirty = true; // unsaved, before the snapshot
    if (saveTimer) clearTimeout(saveTimer);
    saveTimer = setTimeout(() => {
      saveTimer = null;
      if (commitPending()) void saveNow();
    }, 700);
  }

  /** Snapshot + stage the pending change; false when there is nothing new. */
  function commitPending(): boolean {
    if (!changePending || !sceneId) return false;
    changePending = false;
    if (canvas.saveContext !== saveContext) return false;
    const doc = snapshotDoc();
    if (!doc || doc.source === pendingDoc?.source) return false;
    pendingDoc = doc;
    canvas.stageDoc(sceneId, doc, saveContext);
    return true;
  }

  async function saveNow(): Promise<void> {
    const doc = pendingDoc;
    if (!doc || !sceneId) return;
    try {
      if (await canvas.persistDoc(sceneId, doc, saveContext)) learnSaved(doc);
      // Newer local drawing wins over the response to an older snapshot.
      if (!destroyed && canvas.saveContext === saveContext && canvas.currentId === sceneId && pendingDoc === doc) {
        lastApplied = doc.source ?? '';
        canvas.source = lastApplied;
      }
    } catch (e) {
      toasts.error('Canvas save failed', e instanceof Error ? e.message : String(e));
    }
  }

  // Excalidraw accepts a Promise here: the elements are ready at once and the
  // image refs resolve (one immutable fetch each, HTTP-cached on reopen)
  // before the first paint, so files are part of the baseline, not an edit.
  async function initialData() {
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    let raw: any = null;
    try {
      raw = canvas.source ? JSON.parse(canvas.source) : null;
    } catch {
      raw = null;
    }
    const elements = normalizeScene(raw);
    lastApplied = canvas.source ?? '';
    const resolved = raw?.files && typeof raw.files === 'object' ? await resolveFiles(raw.files, knownFiles) : [];
    return {
      elements,
      appState: { viewBackgroundColor: raw?.appState?.viewBackgroundColor ?? '#ffffff' },
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      files: Object.fromEntries(resolved.map((f) => [f.id, f])) as any,
      scrollToContent: elements.length > 0,
    };
  }

  onMount(async () => {
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    const w = window as any;
    if (!w.EXCALIDRAW_ASSET_PATH) {
      // Fonts are served by Otto (vite.config.ts `excalidrawFonts`), not a CDN.
      w.EXCALIDRAW_ASSET_PATH = `${import.meta.env.BASE_URL}assets/excalidraw/`;
    }
    const React = await import('react');
    const { createRoot } = await import('react-dom/client');
    const Ex = await import('@excalidraw/excalidraw');
    await import('@excalidraw/excalidraw/index.css');
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    restore = (Ex as any).restoreElements ?? null;
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    sceneVersionOf = (Ex as any).getSceneVersion ?? null;
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
        onChange: onSceneChange,
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
      saveTimer = null;
      // The last edit is staged here (the timer that would have done it is gone).
      commitPending();
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
