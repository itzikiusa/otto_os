<script lang="ts">
  // DesignBoard — the arena's Excalidraw board (design §4.2). Excalidraw is
  // React, so it mounts React-in-Svelte (host div + createRoot + createElement),
  // the same island pattern as Canvas' ExcalidrawCanvas — but bound to PROPS, not
  // the canvas store: the arena owns the source, the debounce and the conflict
  // handling; this component only turns `source` into a board and emits the
  // full Excalidraw scene after manual edits (≤ 1 per 250 ms) via `onchange`.
  //
  //   source → board   parse + normalise. A full saved doc goes through
  //                    restoreElements; the agent's simplified form (no Excalidraw
  //                    internals) is BUILT with the shared canvas builder so labels
  //                    stay centred (the stock converter scatters them to 0,0).
  //   board → source   `onchange(JSON)` of the FULL scene after a manual edit,
  //                    at most once per 250 ms (trailing; flushed on unmount).
  import { onMount, onDestroy } from 'svelte';
  import LoadState from '../../../lib/components/LoadState.svelte';
  import { ui } from '../../../lib/stores/ui.svelte';
  import { buildExcalidrawElements, isSimplified } from '../../canvas/excalidraw-build';
  import type { BinaryFileData, BinaryFiles } from '@excalidraw/excalidraw/types';

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

  /** Image bytes already inlined by a Design import (`data:` URLs). A historical
   *  `otto-canvas-file:` marker is left out: Design Hall does not resolve it.
   *  A row is an Excalidraw file only when the guard below accepts it. */
  function isRecord(value: unknown): value is Record<string, unknown> {
    return typeof value === 'object' && value !== null && !Array.isArray(value);
  }

  function isBinaryMime(value: string): value is BinaryFileData['mimeType'] {
    const mimes = [
      'image/svg+xml',
      'image/png',
      'image/jpeg',
      'image/gif',
      'image/webp',
      'image/bmp',
      'image/x-icon',
      'image/avif',
      'image/jfif',
      'application/octet-stream',
    ] as const;
    for (const mime of mimes) {
      if (mime === value) return true;
    }
    return false;
  }

  function finiteNumber(value: unknown): number | null {
    return typeof value === 'number' && Number.isFinite(value) ? value : null;
  }

  /** Runtime shape of Excalidraw `BinaryFileData`. Brands are not asserted;
   *  the predicate is the only way a parsed row becomes that type. */
  function isBinaryFileData(value: {
    mimeType: string;
    id: string;
    dataURL: string;
    created: number;
    lastRetrieved?: number;
    version?: number;
  }): value is BinaryFileData {
    return isBinaryMime(value.mimeType) && value.id.length > 0 && value.dataURL.startsWith('data:');
  }

  function binaryFileFrom(key: string, file: unknown): BinaryFileData | null {
    if (!isRecord(file)) return null;
    const dataURL = file.dataURL;
    if (typeof dataURL !== 'string' || !dataURL.startsWith('data:')) return null;
    const semi = dataURL.indexOf(';');
    const header = semi > 5 ? dataURL.slice(5, semi) : '';
    const mimeType = typeof file.mimeType === 'string' ? file.mimeType : header;
    const id = typeof file.id === 'string' && file.id.length > 0 ? file.id : key;
    const lastRetrieved = finiteNumber(file.lastRetrieved);
    const version = finiteNumber(file.version);
    const candidate = {
      mimeType,
      id,
      dataURL,
      created: finiteNumber(file.created) ?? 0,
      ...(lastRetrieved === null ? {} : { lastRetrieved }),
      ...(version === null ? {} : { version }),
    };
    if (!isBinaryFileData(candidate)) return null;
    return candidate;
  }

  function dataUrlFiles(files: unknown): BinaryFiles {
    const out: BinaryFiles = {};
    if (!isRecord(files)) return out;
    for (const [key, file] of Object.entries(files)) {
      const row = binaryFileFrom(key, file);
      if (row) out[row.id] = row;
    }
    return out;
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
      const files = Object.values(dataUrlFiles(raw?.files));
      if (files.length && typeof ex.addFiles === 'function') {
        ex.addFiles(files);
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

  // SE-19 (same fix as Canvas' SD-05): Excalidraw fires onChange on every drag
  // frame, scroll, zoom and selection. Serializing the whole scene (base64
  // files included) per call cost 5–10 ms at 2k–5k elements. A cheap
  // fingerprint (element version sum + file count + background) drops no-op
  // reports, and the scene is serialized once per 250 ms trailing window (and
  // flushed on unmount) instead of once per frame.
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  let sceneVersionOf: ((els: readonly any[]) => number) | null = null;
  let lastFingerprint: string | null = null;
  let emitTimer: ReturnType<typeof setTimeout> | null = null;
  const EMIT_MS = 250;

  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  function handleChange(elements?: readonly any[], appState?: any, files?: any): void {
    if (readonly || !excaliApi || !onchange) return;
    const fp = sceneVersionOf && elements
      ? `${sceneVersionOf(elements)}:${files ? Object.keys(files).length : 0}:${appState?.viewBackgroundColor ?? ''}`
      : null;
    const unchanged = fp !== null && fp === lastFingerprint;
    lastFingerprint = fp;
    // A programmatic load only moves the baseline; scroll/zoom/select is no edit.
    if (suppressChange || unchanged) return;
    if (emitTimer) return; // trailing emit already scheduled
    emitTimer = setTimeout(flushChange, EMIT_MS);
  }

  function flushChange(): void {
    if (emitTimer) clearTimeout(emitTimer);
    emitTimer = null;
    if (readonly || !excaliApi || !onchange) return;
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

  function initialData(): {
    elements: ReturnType<typeof normalizeScene>;
    appState: { viewBackgroundColor: string };
    files: BinaryFiles;
    scrollToContent: boolean;
  } {
    const raw = parse(source ?? '');
    const elements = normalizeScene(raw);
    lastApplied = source ?? '';
    const files: BinaryFiles = dataUrlFiles(raw?.files);
    return {
      elements,
      appState: { viewBackgroundColor: raw?.appState?.viewBackgroundColor ?? '#ffffff' },
      files,
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
    try {
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
    // Never drop the last <250 ms of drawing: emit it before the island goes.
    if (emitTimer) {
      try { flushChange(); } catch { /* ignore */ }
    }
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
  {#if loadError || !ready}
    <div class="board-state">
      <LoadState what="the board" loading={!ready && !loadError} error={loadError} empty variant="compact" />
    </div>
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
  .board-state {
    position: absolute;
    inset: 0;
    display: grid;
    place-items: center;
    padding: 16px;
  }
</style>
