<script module lang="ts">
  // The currently-mounted canvas, held at MODULE scope (only one is open) so an
  // in-flight generate() survives a brief remount.
  let liveId: string | null = null;
</script>

<script lang="ts">
  // The Mermaid canvas: a live preview of the scene's `.mermaid` SOURCE PLUS a full
  // editor. You edit the diagram three ways, all writing the SAME `canvas.mermaid`
  // file: (1) the agent via "Ask AI", (2) directly in the Code panel (the Mermaid
  // source, with live preview), (3) — nothing is converted to Excalidraw; this stays
  // Mermaid. Mermaid's own renderer draws the full rich spectrum (subgraphs, classDef
  // colours, every shape). Pan/zoom the preview.
  import { onMount, onDestroy, tick } from 'svelte';
  import { canvas } from '../../lib/stores/canvas.svelte';
  import { canvasDocBus } from '../../lib/events.svelte';
  import { api } from '../../lib/api/client';
  import { toasts } from '../../lib/toast.svelte';
  import { renderMermaid } from './mermaid';
  import type { CanvasDoc, CanvasFormat } from './types';
  import Icon from '../../lib/components/Icon.svelte';
  import CodeEditor from '../../lib/components/CodeEditor.svelte';

  interface Props {
    readonly?: boolean;
  }
  let { readonly = false }: Props = $props();

  // The scene THIS editor is mounted for — saves target THIS id, never
  // canvas.currentId (which on a scene switch already points at the next scene).
  const sceneId = canvas.currentId;

  let surface = $state<HTMLDivElement | null>(null);
  let content = $state<HTMLDivElement | null>(null);
  let svgHtml = $state('');
  let renderError = $state('');
  let notMermaid = $state(false);
  let generating = $state(false);
  let codeOpen = $state(false);

  // Pan/zoom transform.
  let scale = $state(1);
  let tx = $state(0);
  let ty = $state(0);
  let userAdjusted = false;
  let natW = 800;
  let natH = 600;
  let renderToken = 0;
  let lastRendered = '';

  const clamp = (v: number, lo: number, hi: number) => Math.min(hi, Math.max(lo, v));

  // The agent + the user share ONE file. Save the edited Mermaid SOURCE to THIS
  // scene (guarded: if the scene switched, drop the stale save).
  async function saveMermaid(value: string): Promise<void> {
    if (canvas.currentId !== sceneId || !sceneId) return;
    const doc = { type: 'otto-canvas', version: 1, format: 'mermaid' as CanvasFormat, source: value };
    canvas.source = value; // drives the live preview re-render
    canvas.rawDoc = doc;
    try {
      await api.put(`/canvas/scenes/${sceneId}`, { doc });
      // Re-check after the await: don't mark a now-switched scene as saved/clean.
      if (canvas.currentId === sceneId) canvas.markSaved(doc);
    } catch (e) {
      if (canvas.currentId === sceneId)
        toasts.error('Save failed', e instanceof Error ? e.message : String(e));
    }
  }
  let codeTimer: ReturnType<typeof setTimeout> | null = null;
  function onCode(value: string): void {
    if (codeTimer) clearTimeout(codeTimer);
    codeTimer = setTimeout(() => void saveMermaid(value), 500);
  }

  /** Render the current source to SVG (Mermaid native) and auto-fit. */
  async function renderNow(src: string): Promise<void> {
    const token = ++renderToken;
    const text = (src ?? '').trim();
    if (!text) {
      svgHtml = '';
      renderError = '';
      notMermaid = false;
      lastRendered = '';
      return;
    }
    // Guard: a scene whose source is Excalidraw JSON (e.g. mislabelled) is not
    // Mermaid — show a clear message instead of a raw mermaid parse error.
    if (text.startsWith('{') && /"(type|elements)"\s*:/.test(text.slice(0, 80))) {
      notMermaid = true;
      svgHtml = '';
      renderError = '';
      return;
    }
    notMermaid = false;
    if (text === lastRendered) return; // nothing changed
    const out = await renderMermaid(`cv-${sceneId ?? 'x'}-${token}`, text);
    if (token !== renderToken) return; // superseded
    if (out.error || !out.svg) {
      renderError = out.error || 'Could not render the diagram';
      return; // keep the last good SVG on screen
    }
    renderError = '';
    lastRendered = text;
    svgHtml = out.svg;
    await tick();
    sizeSvg();
    if (!userAdjusted) fit();
  }

  function sizeSvg(): void {
    const svg = content?.querySelector('svg');
    if (!svg) return;
    const vb = svg.viewBox?.baseVal;
    natW = (vb && vb.width) || svg.getBoundingClientRect().width || 800;
    natH = (vb && vb.height) || svg.getBoundingClientRect().height || 600;
    svg.style.maxWidth = 'none';
    svg.style.width = `${natW}px`;
    svg.style.height = `${natH}px`;
  }

  function fit(): void {
    if (!surface) return;
    const r = surface.getBoundingClientRect();
    if (!r.width || !r.height) return;
    const s = clamp(Math.min(r.width / natW, r.height / natH) * 0.92, 0.1, 4);
    scale = s;
    tx = (r.width - natW * s) / 2;
    ty = (r.height - natH * s) / 2;
  }

  export function fitView(): void {
    userAdjusted = false;
    fit();
  }
  function zoomBy(factor: number): void {
    if (!surface) return;
    userAdjusted = true;
    const r = surface.getBoundingClientRect();
    const cx = r.width / 2;
    const cy = r.height / 2;
    const ns = clamp(scale * factor, 0.1, 6);
    tx = cx - (cx - tx) * (ns / scale);
    ty = cy - (cy - ty) * (ns / scale);
    scale = ns;
  }
  function onWheel(e: WheelEvent): void {
    e.preventDefault();
    if (!surface) return;
    userAdjusted = true;
    // Pinch (ctrl+wheel on macOS) zooms; a plain two-finger scroll pans.
    if (!e.ctrlKey) {
      tx -= e.deltaX;
      ty -= e.deltaY;
      return;
    }
    const r = surface.getBoundingClientRect();
    const cx = e.clientX - r.left;
    const cy = e.clientY - r.top;
    const factor = Math.exp(-e.deltaY * 0.0015);
    const ns = clamp(scale * factor, 0.1, 6);
    tx = cx - (cx - tx) * (ns / scale);
    ty = cy - (cy - ty) * (ns / scale);
    scale = ns;
  }

  let dragging = $state(false);
  let lastX = 0;
  let lastY = 0;
  function onPointerDown(e: PointerEvent): void {
    if ((e.target as HTMLElement)?.closest('a')) return;
    dragging = true;
    userAdjusted = true;
    lastX = e.clientX;
    lastY = e.clientY;
    (e.currentTarget as HTMLElement).setPointerCapture?.(e.pointerId);
  }
  function onPointerMove(e: PointerEvent): void {
    if (!dragging) return;
    tx += e.clientX - lastX;
    ty += e.clientY - lastY;
    lastX = e.clientX;
    lastY = e.clientY;
  }
  function onPointerUp(e: PointerEvent): void {
    dragging = false;
    (e.currentTarget as HTMLElement).releasePointerCapture?.(e.pointerId);
  }

  function downloadSvg(): void {
    const svg = content?.querySelector('svg');
    if (!svg) return;
    const blob = new Blob([new XMLSerializer().serializeToString(svg)], { type: 'image/svg+xml' });
    const url = URL.createObjectURL(blob);
    const a = document.createElement('a');
    a.href = url;
    a.download = `${(canvas.scene?.title ?? 'canvas').replace(/[^\w.-]+/g, '-')}.svg`;
    a.click();
    setTimeout(() => URL.revokeObjectURL(url), 1000);
  }

  /** Ask the agent to edit this scene's .mermaid source. */
  export async function generate(prompt: string): Promise<void> {
    const p = prompt.trim();
    if (!p || generating) return;
    generating = true;
    userAdjusted = false;
    canvas.pushConvo('user', p);
    try {
      const res = await canvas.assist(p, 'flow');
      const src = res.mermaid ?? '';
      if (!src.trim()) {
        canvas.pushConvo('assistant', res.note || 'No diagram was produced.');
        toasts.info('Nothing to draw', res.note || 'The agent did not return a diagram.');
        return;
      }
      canvas.ingestDoc({ type: 'otto-canvas', version: 1, format: 'mermaid', source: src });
      canvas.pushConvo('assistant', res.note || 'Updated the canvas.');
      toasts.success('Drawn on canvas', res.note || 'Diagram updated.');
      void canvas.refreshSession();
    } catch (e) {
      canvas.pushConvo('assistant', `Failed: ${e instanceof Error ? e.message : String(e)}`);
      toasts.error('Ask AI failed', e instanceof Error ? e.message : String(e));
    } finally {
      generating = false;
    }
  }
  export function isGenerating(): boolean {
    return generating;
  }

  // Live edits: translate the canvas_updated push into the store.
  $effect(() => {
    const _t = canvasDocBus.tick;
    if (!_t || canvasDocBus.sceneId !== canvas.currentId) return;
    const doc = canvasDocBus.doc as CanvasDoc | null;
    if (doc && typeof doc.source === 'string') canvas.ingestDoc(doc);
  });

  // Render whenever the source changes (open / generate / live / code edit).
  $effect(() => {
    const src = canvas.source; // dependency
    void renderNow(src ?? '');
  });

  onMount(() => {
    liveId = canvas.currentId;
  });
  onDestroy(() => {
    if (codeTimer) clearTimeout(codeTimer);
    if (liveId === canvas.currentId) liveId = null;
  });
</script>

<div class="board">
  <div class="lanes">
    {#if codeOpen && !readonly}
      <aside class="code-pane">
        <div class="code-head">
          <span><Icon name="branch" size={13} /> Mermaid source</span>
          <span class="code-hint">edits save + render live</span>
        </div>
        <div class="code-body">
          <CodeEditor
            path="canvas.mermaid"
            root=""
            content={canvas.source ?? ''}
            readOnly={false}
            completionSource={() => null}
            onchange={onCode}
          />
        </div>
      </aside>
    {/if}

    <div class="preview-wrap">
      <div
        class="surface"
        class:grabbing={dragging}
        role="application"
        aria-label="Diagram — drag to pan, scroll to zoom"
        bind:this={surface}
        onwheel={onWheel}
        onpointerdown={onPointerDown}
        onpointermove={onPointerMove}
        onpointerup={onPointerUp}
        onpointercancel={onPointerUp}
      >
        {#if svgHtml}
          <div
            class="content"
            bind:this={content}
            style="transform: translate({tx}px, {ty}px) scale({scale}); transform-origin: 0 0;"
          >
            <!-- eslint-disable-next-line svelte/no-at-html-tags -->
            {@html svgHtml}
          </div>
        {:else if notMermaid}
          <div class="empty">
            <Icon name="shapes" size={28} />
            <p class="lead">This canvas holds Excalidraw content</p>
            <p class="hint">It's labelled Mermaid but contains an Excalidraw scene. Create a new
              <strong>Excalidraw</strong> canvas to edit those shapes.</p>
          </div>
        {:else if !renderError}
          <div class="empty">
            <Icon name="branch" size={28} />
            <p class="lead">Mermaid diagram</p>
            <p class="hint">Describe it in <strong>Ask AI</strong>, or open <strong>Code</strong> to
              edit the Mermaid yourself — both write the same diagram and render here live.</p>
          </div>
        {/if}

        {#if renderError}
          <div class="err" role="status">
            <Icon name="x" size={14} /> Diagram error: {renderError}
          </div>
        {/if}
      </div>

      <div class="mode-bar">
        <span class="mode-chip"><Icon name="branch" size={12} /> Mermaid</span>
        {#if !readonly}
          <button
            class="code-toggle"
            class:on={codeOpen}
            onclick={() => (codeOpen = !codeOpen)}
            title="Edit the Mermaid source"
          >
            <Icon name="edit" size={12} /> Code
          </button>
        {/if}
      </div>

      {#if svgHtml}
        <div class="zoombar">
          <button onclick={() => zoomBy(1 / 1.2)} title="Zoom out" aria-label="Zoom out">−</button>
          <button class="pct" onclick={fitView} title="Fit to screen">{Math.round(scale * 100)}%</button>
          <button onclick={() => zoomBy(1.2)} title="Zoom in" aria-label="Zoom in">+</button>
          <span class="sep"></span>
          <button onclick={downloadSvg} title="Download SVG" aria-label="Download SVG">
            <Icon name="file" size={15} />
          </button>
        </div>
      {/if}
    </div>
  </div>
</div>

<style>
  .board {
    position: relative;
    width: 100%;
    height: 100%;
    min-height: 0;
    overflow: hidden;
    background: var(--bg);
  }
  .lanes {
    display: flex;
    width: 100%;
    height: 100%;
    min-height: 0;
  }
  .code-pane {
    flex: 0 0 min(440px, 42%);
    display: flex;
    flex-direction: column;
    min-width: 0;
    min-height: 0;
    border-inline-end: 1px solid var(--border);
    background: var(--surface);
  }
  .code-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    padding: 8px 12px;
    border-bottom: 1px solid var(--border);
    font-size: 12px;
    font-weight: 600;
    color: var(--text);
    flex: none;
  }
  .code-head span:first-child {
    display: inline-flex;
    align-items: center;
    gap: 6px;
  }
  .code-hint {
    color: var(--text-dim, #888);
    font-weight: 500;
    font-size: 11px;
  }
  .code-body {
    flex: 1 1 auto;
    min-height: 0;
    overflow: hidden;
  }
  .code-body :global(.cm-editor) {
    height: 100%;
  }
  .preview-wrap {
    flex: 1 1 auto;
    position: relative;
    min-width: 0;
    min-height: 0;
    background-image: radial-gradient(
      circle,
      color-mix(in srgb, var(--text) 12%, transparent) 1px,
      transparent 1px
    );
    background-size: 22px 22px;
  }
  .surface {
    position: absolute;
    inset: 0;
    overflow: hidden;
    cursor: grab;
    touch-action: none;
  }
  .surface.grabbing {
    cursor: grabbing;
  }
  .content {
    position: absolute;
    top: 0;
    left: 0;
    will-change: transform;
  }
  .content :global(svg) {
    display: block;
  }
  .empty {
    position: absolute;
    inset: 0;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 8px;
    text-align: center;
    color: var(--text-dim, #888);
    padding: 24px;
    pointer-events: none;
  }
  .empty .lead {
    margin: 6px 0 0;
    font-size: 15px;
    font-weight: 600;
    color: var(--text);
  }
  .empty .hint {
    margin: 0;
    font-size: 13px;
    max-width: 360px;
    line-height: 1.5;
  }
  .err {
    position: absolute;
    left: 50%;
    bottom: 64px;
    transform: translateX(-50%);
    display: inline-flex;
    align-items: center;
    gap: 6px;
    max-width: 80%;
    padding: 7px 12px;
    border-radius: 8px;
    background: color-mix(in srgb, #dc2626 16%, var(--surface));
    border: 1px solid #dc2626;
    color: var(--text);
    font-size: 12px;
  }
  .mode-bar {
    position: absolute;
    top: 12px;
    left: 12px;
    z-index: 5;
    display: inline-flex;
    align-items: center;
    gap: 6px;
  }
  .mode-chip {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    padding: 4px 9px;
    border-radius: 999px;
    background: var(--surface);
    border: 1px solid var(--border);
    color: var(--text-dim, #888);
    font-size: 11px;
    font-weight: 600;
    box-shadow: var(--shadow, 0 2px 8px rgba(0, 0, 0, 0.12));
  }
  .code-toggle {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    padding: 4px 10px;
    border-radius: 999px;
    background: var(--surface);
    border: 1px solid var(--border);
    color: var(--text);
    font-size: 11px;
    font-weight: 600;
    cursor: pointer;
    box-shadow: var(--shadow, 0 2px 8px rgba(0, 0, 0, 0.12));
  }
  .code-toggle:hover,
  .code-toggle.on {
    border-color: var(--accent);
    color: var(--accent);
  }
  .zoombar {
    position: absolute;
    bottom: 16px;
    right: 16px;
    z-index: 5;
    display: inline-flex;
    align-items: center;
    gap: 2px;
    padding: 4px;
    border-radius: 999px;
    background: var(--surface);
    border: 1px solid var(--border);
    box-shadow: var(--shadow, 0 4px 16px rgba(0, 0, 0, 0.2));
  }
  .zoombar button {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    min-width: 30px;
    height: 30px;
    padding: 0 6px;
    border: none;
    background: none;
    color: var(--text);
    font-size: 16px;
    font-weight: 600;
    cursor: pointer;
    border-radius: 999px;
  }
  .zoombar button:hover {
    background: color-mix(in srgb, var(--text) 8%, transparent);
  }
  .zoombar .pct {
    font-size: 12px;
    min-width: 48px;
  }
  .zoombar .sep {
    width: 1px;
    height: 18px;
    background: var(--border);
    margin: 0 2px;
  }
</style>
