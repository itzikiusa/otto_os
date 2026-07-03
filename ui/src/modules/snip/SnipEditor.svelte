<script lang="ts">
  // Snip annotation editor — chrome-less full-screen view at `#/snip/{id}`.
  //
  // Single canvas at the image's natural pixel size, CSS-scaled to fit; all
  // geometry stays in image-pixel space (pointer events are mapped through the
  // bounding-rect scale). Every committed mutation schedules a debounced
  // flatten → POST /snips/{id}/annotated, which puts the latest state on the
  // clipboard — the user can paste into a session at any moment (R4).
  import { onMount } from 'svelte';
  import { router } from '../../lib/router.svelte';
  import { snipApi } from '../../lib/snip';
  import { toasts } from '../../lib/toast.svelte';
  import { isTauri } from '../../lib/stores/ui.svelte';
  import {
    PALETTE,
    STROKES,
    FONTS,
    type Anno,
    type Tool,
    render,
    drawSelection,
    hitTest,
    hitHandle,
    moveAnno,
    resizeAnno,
    bounds,
    flatten,
    blobToB64,
  } from './annotations';

  const snipId = $derived(router.parts[1] ?? '');

  let img: HTMLImageElement | null = $state(null);
  let missing = $state(false);
  let loading = $state(true);
  let canvasEl: HTMLCanvasElement | undefined = $state();
  let wrapEl: HTMLDivElement | undefined = $state();

  let annos: Anno[] = $state([]);
  let selected: number | null = $state(null); // Anno id
  let tool: Tool = $state('rect');
  let color: string = $state(PALETTE[0]);
  let strokeIx = $state(1);
  let fontIx = $state(1);

  // Undo/redo: snapshots of the object list (cheap — plain JSON objects).
  let undoStack: Anno[][] = $state([]);
  let redoStack: Anno[][] = $state([]);

  // Auto-copy machinery.
  let copyState: 'idle' | 'pending' | 'copying' | 'copied' | 'failed' = $state('idle');
  let copyTimer: ReturnType<typeof setTimeout> | null = null;
  let copyInFlight = false;
  let copyAgain = false;

  // In-progress drawing state (not reactive — pointermove is hot).
  let drafting: Anno | null = null;
  let dragMode: 'draw' | 'move' | 'resize' | null = null;
  let dragHandle = 0;
  let dragLast = { x: 0, y: 0 };
  let nextId = 1;
  let nextBadge = 1;

  // Text overlay editing.
  let textDraft = $state<{ x: number; y: number; value: string; editId: number | null } | null>(null);
  let textareaEl: HTMLTextAreaElement | undefined = $state();

  const isSecondaryWindow =
    typeof window !== 'undefined' &&
    !!(window as unknown as { __OTTO_WIN__?: string }).__OTTO_WIN__ &&
    (window as unknown as { __OTTO_WIN__?: string }).__OTTO_WIN__ !== 'main';

  onMount(() => {
    let url: string | null = null;
    (async () => {
      try {
        url = await snipApi.imageUrl(snipId);
        const el = new Image();
        el.onload = () => {
          img = el;
          loading = false;
          queueMicrotask(redraw);
        };
        el.onerror = () => {
          missing = true;
          loading = false;
        };
        el.src = url;
      } catch {
        missing = true;
        loading = false;
      }
    })();
    return () => {
      if (url) URL.revokeObjectURL(url);
      if (copyTimer) clearTimeout(copyTimer);
    };
  });

  function redraw(): void {
    if (!canvasEl || !img) return;
    if (canvasEl.width !== img.width) canvasEl.width = img.width;
    if (canvasEl.height !== img.height) canvasEl.height = img.height;
    const ctx = canvasEl.getContext('2d');
    if (!ctx) return;
    const list = drafting ? [...annos, drafting] : annos;
    render(ctx, img, list);
    const sel = annos.find((a) => a.id === selected);
    if (sel) drawSelection(ctx, sel);
  }

  $effect(() => {
    // Redraw on any committed state change (annos/selected are reactive).
    void annos;
    void selected;
    redraw();
  });

  // ── History + auto-copy ────────────────────────────────────────────────────

  function snapshot(): void {
    undoStack = [...undoStack, annos.map((a) => ({ ...a, points: a.points?.slice() }))];
    redoStack = [];
  }

  function commit(next: Anno[]): void {
    annos = next;
    scheduleCopy();
  }

  function scheduleCopy(): void {
    copyState = 'pending';
    if (copyTimer) clearTimeout(copyTimer);
    copyTimer = setTimeout(() => void copyNow(), 800);
  }

  async function copyNow(): Promise<void> {
    if (!img) return;
    if (copyInFlight) {
      copyAgain = true;
      return;
    }
    copyInFlight = true;
    copyState = 'copying';
    try {
      const blob = await flatten(img, annos);
      const resp = await snipApi.saveAnnotated(snipId, await blobToB64(blob));
      copyState = resp.copied ? 'copied' : 'failed';
    } catch (e) {
      copyState = 'failed';
      toasts.error('Copy failed', e instanceof Error ? e.message : String(e));
    } finally {
      copyInFlight = false;
      if (copyAgain) {
        copyAgain = false;
        void copyNow();
      }
    }
  }

  function undo(): void {
    const prev = undoStack.at(-1);
    if (!prev) return;
    undoStack = undoStack.slice(0, -1);
    redoStack = [...redoStack, annos];
    selected = null;
    commit(prev);
  }

  function redo(): void {
    const next = redoStack.at(-1);
    if (!next) return;
    redoStack = redoStack.slice(0, -1);
    undoStack = [...undoStack, annos];
    selected = null;
    commit(next);
  }

  // ── Pointer handling ───────────────────────────────────────────────────────

  function toImage(e: PointerEvent | MouseEvent): { x: number; y: number } {
    const r = canvasEl!.getBoundingClientRect();
    return {
      x: ((e.clientX - r.left) / r.width) * canvasEl!.width,
      y: ((e.clientY - r.top) / r.height) * canvasEl!.height,
    };
  }

  function onPointerDown(e: PointerEvent): void {
    if (!img || textDraft) return;
    canvasEl!.setPointerCapture(e.pointerId);
    const p = toImage(e);
    dragLast = p;

    if (tool === 'select') {
      const sel = annos.find((a) => a.id === selected);
      if (sel) {
        const h = hitHandle(sel, p.x, p.y);
        if (h !== null) {
          snapshot();
          dragMode = 'resize';
          dragHandle = h;
          return;
        }
      }
      const hit = hitTest(annos, p.x, p.y);
      selected = hit?.id ?? null;
      if (hit) {
        snapshot();
        dragMode = 'move';
      }
      return;
    }

    if (tool === 'text') {
      openTextDraft(p.x, p.y, '', null);
      return;
    }

    if (tool === 'badge') {
      snapshot();
      const a: Anno = {
        id: nextId++,
        tool: 'badge',
        x1: p.x,
        y1: p.y,
        x2: p.x,
        y2: p.y,
        color,
        stroke: STROKES[strokeIx],
        font: FONTS[fontIx],
        n: nextBadge++,
      };
      commit([...annos, a]);
      return;
    }

    dragMode = 'draw';
    drafting = {
      id: nextId++,
      tool: tool as Anno['tool'],
      x1: p.x,
      y1: p.y,
      x2: p.x,
      y2: p.y,
      color,
      stroke: STROKES[strokeIx],
      font: FONTS[fontIx],
      ...(tool === 'pen' || tool === 'highlight' ? { points: [p] } : {}),
    };
  }

  function onPointerMove(e: PointerEvent): void {
    if (!img) return;
    const p = toImage(e);
    if (dragMode === 'draw' && drafting) {
      drafting.x2 = p.x;
      drafting.y2 = p.y;
      drafting.points?.push(p);
      redraw();
    } else if (dragMode === 'move' && selected !== null) {
      const dx = p.x - dragLast.x;
      const dy = p.y - dragLast.y;
      dragLast = p;
      annos = annos.map((a) => (a.id === selected ? moveAnno(a, dx, dy) : a));
    } else if (dragMode === 'resize' && selected !== null) {
      annos = annos.map((a) => (a.id === selected ? resizeAnno(a, dragHandle, p.x, p.y) : a));
    }
  }

  function onPointerUp(): void {
    if (dragMode === 'draw' && drafting) {
      const b = bounds(drafting);
      const tiny = b.w < 3 && b.h < 3 && !drafting.points;
      if (!tiny) {
        snapshot();
        commit([...annos, drafting]);
      }
      drafting = null;
      redraw();
    } else if (dragMode === 'move' || dragMode === 'resize') {
      scheduleCopy();
    }
    dragMode = null;
  }

  // ── Text tool ──────────────────────────────────────────────────────────────

  function openTextDraft(x: number, y: number, value: string, editId: number | null): void {
    textDraft = { x, y, value, editId };
    queueMicrotask(() => textareaEl?.focus());
  }

  function commitText(): void {
    if (!textDraft) return;
    const { x, y, value, editId } = textDraft;
    textDraft = null;
    const text = value.trimEnd();
    if (!text) {
      if (editId !== null) {
        snapshot();
        commit(annos.filter((a) => a.id !== editId));
      }
      return;
    }
    snapshot();
    if (editId !== null) {
      commit(annos.map((a) => (a.id === editId ? { ...a, text } : a)));
    } else {
      commit([
        ...annos,
        {
          id: nextId++,
          tool: 'text',
          x1: x,
          y1: y,
          x2: x,
          y2: y,
          text,
          color,
          stroke: STROKES[strokeIx],
          font: FONTS[fontIx],
        },
      ]);
    }
  }

  function onDblClick(e: MouseEvent): void {
    if (!img) return;
    const p = toImage(e);
    const hit = hitTest(annos, p.x, p.y);
    if (hit?.tool === 'text') {
      selected = hit.id;
      openTextDraft(hit.x1, hit.y1, hit.text ?? '', hit.id);
    }
  }

  // Overlay position for the textarea (CSS px, from image px).
  const textOverlayStyle = $derived.by(() => {
    if (!textDraft || !canvasEl || !img) return '';
    const r = canvasEl.getBoundingClientRect();
    const wrap = wrapEl?.getBoundingClientRect();
    if (!wrap) return '';
    const sx = r.width / canvasEl.width;
    const sy = r.height / canvasEl.height;
    const left = r.left - wrap.left + textDraft.x * sx;
    const top = r.top - wrap.top + textDraft.y * sy;
    const fs = FONTS[fontIx] * sy;
    return `left:${left}px;top:${top}px;font-size:${fs}px;color:${color};`;
  });

  // ── Keyboard ───────────────────────────────────────────────────────────────

  function onKeydown(e: KeyboardEvent): void {
    if (textDraft) {
      if (e.key === 'Escape') {
        textDraft = null;
        e.stopPropagation();
      } else if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) {
        commitText();
        e.stopPropagation();
      }
      return;
    }
    const mod = e.metaKey || e.ctrlKey;
    if (mod && e.key.toLowerCase() === 'z') {
      e.preventDefault();
      if (e.shiftKey) redo();
      else undo();
    } else if (mod && e.key.toLowerCase() === 'c' && selected === null) {
      e.preventDefault();
      void copyNow();
    } else if ((e.key === 'Delete' || e.key === 'Backspace') && selected !== null) {
      e.preventDefault();
      snapshot();
      commit(annos.filter((a) => a.id !== selected));
      selected = null;
    } else if (e.key === 'Escape') {
      selected = null;
    } else if (e.key.startsWith('Arrow') && selected !== null) {
      e.preventDefault();
      const d = e.shiftKey ? 10 : 1;
      const dx = e.key === 'ArrowLeft' ? -d : e.key === 'ArrowRight' ? d : 0;
      const dy = e.key === 'ArrowUp' ? -d : e.key === 'ArrowDown' ? d : 0;
      snapshot();
      commit(annos.map((a) => (a.id === selected ? moveAnno(a, dx, dy) : a)));
    }
  }

  // ── Chrome actions ─────────────────────────────────────────────────────────

  async function close(): Promise<void> {
    if (isTauri && isSecondaryWindow) {
      try {
        const { getCurrentWindow } = await import('@tauri-apps/api/window');
        await getCurrentWindow().close();
        return;
      } catch {
        // fall through to routing
      }
    }
    if (router.parts.length && history.length > 1) router.back();
    else router.go('agents');
  }

  async function deleteSnip(): Promise<void> {
    try {
      await snipApi.remove(snipId);
      toasts.info('Snip deleted');
      await close();
    } catch (e) {
      toasts.error('Delete failed', e instanceof Error ? e.message : String(e));
    }
  }

  const TOOLS: { id: Tool; label: string; icon: string; title: string }[] = [
    { id: 'select', label: 'Select', icon: '⬚', title: 'Select / move (V)' },
    { id: 'rect', label: 'Box', icon: '▭', title: 'Rectangle (R)' },
    { id: 'ellipse', label: 'Ellipse', icon: '◯', title: 'Ellipse (O)' },
    { id: 'arrow', label: 'Arrow', icon: '↗', title: 'Arrow (A)' },
    { id: 'line', label: 'Line', icon: '╱', title: 'Line (L)' },
    { id: 'pen', label: 'Pen', icon: '✎', title: 'Freehand (P)' },
    { id: 'highlight', label: 'Mark', icon: '▆', title: 'Highlighter (H)' },
    { id: 'text', label: 'Text', icon: 'T', title: 'Text (T)' },
    { id: 'pixelate', label: 'Blur', icon: '▩', title: 'Pixelate region (B)' },
    { id: 'badge', label: 'Step', icon: '➊', title: 'Numbered step (N)' },
  ];

  const copyLabel = $derived.by(() => {
    switch (copyState) {
      case 'copied':
        return 'Copied ✓';
      case 'copying':
      case 'pending':
        return 'Copying…';
      case 'failed':
        return 'Copy failed';
      default:
        return 'In clipboard';
    }
  });
</script>

<svelte:window onkeydown={onKeydown} />

<div class="snip-editor" data-count={annos.length}>
  <header class="snip-bar" class:tauri-pad={isTauri && isSecondaryWindow} data-tauri-drag-region>
    <div class="group tools" role="toolbar" aria-label="Annotation tools">
      {#each TOOLS as t (t.id)}
        <button
          class="tb"
          class:active={tool === t.id}
          data-tool={t.id}
          title={t.title}
          onclick={() => {
            tool = t.id;
            if (t.id !== 'select') selected = null;
          }}
        >
          <span class="icon">{t.icon}</span><span class="lbl">{t.label}</span>
        </button>
      {/each}
    </div>
    <div class="group colors" role="toolbar" aria-label="Colors">
      {#each PALETTE as c (c)}
        <button
          class="swatch"
          class:active={color === c}
          data-color={c}
          style={`background:${c}`}
          title={c}
          aria-label={`Color ${c}`}
          onclick={() => (color = c)}
        ></button>
      {/each}
    </div>
    <div class="group sizes" role="toolbar" aria-label="Stroke width">
      {#each ['S', 'M', 'L'] as s, i (s)}
        <button
          class="tb size"
          class:active={strokeIx === i}
          data-stroke={s}
          title={`Stroke ${s} / font ${FONTS[i]}px`}
          onclick={() => {
            strokeIx = i;
            fontIx = i;
          }}>{s}</button
        >
      {/each}
    </div>
    <div class="group history">
      <button class="tb" data-act="undo" title="Undo (⌘Z)" disabled={!undoStack.length} onclick={undo}>↺</button>
      <button class="tb" data-act="redo" title="Redo (⇧⌘Z)" disabled={!redoStack.length} onclick={redo}>↻</button>
    </div>
    <div class="spacer"></div>
    <span class="snip-copied" class:ok={copyState === 'copied' || copyState === 'idle'} class:bad={copyState === 'failed'}
      >{copyLabel}</span
    >
    <div class="group actions">
      <button class="tb primary" data-act="copy" title="Copy now (⌘C)" onclick={() => void copyNow()}>Copy</button>
      <button class="tb" data-act="delete-snip" title="Delete this snip" onclick={() => void deleteSnip()}>Delete</button>
      <button class="tb" data-act="close" title="Close" onclick={() => void close()}>Close</button>
    </div>
  </header>

  <div class="snip-body" bind:this={wrapEl}>
    {#if loading}
      <div class="snip-empty">Loading…</div>
    {:else if missing}
      <div class="snip-empty snip-missing">
        <p>This snip no longer exists.</p>
        <button class="tb" data-act="close" onclick={() => void close()}>Close</button>
      </div>
    {:else}
      <canvas
        class="snip-canvas"
        bind:this={canvasEl}
        onpointerdown={onPointerDown}
        onpointermove={onPointerMove}
        onpointerup={onPointerUp}
        ondblclick={onDblClick}
      ></canvas>
      {#if textDraft}
        <!-- svelte-ignore a11y_autofocus -->
        <textarea
          class="snip-textentry"
          style={textOverlayStyle}
          bind:this={textareaEl}
          bind:value={textDraft.value}
          onblur={commitText}
          placeholder="Type… (⌘↩ to commit)"
          rows="2"
        ></textarea>
      {/if}
    {/if}
  </div>
</div>

<style>
  .snip-editor {
    position: fixed;
    inset: 0;
    display: flex;
    flex-direction: column;
    background: var(--bg);
    color: var(--text);
    z-index: 50;
  }
  .snip-bar {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 8px 12px;
    border-bottom: 1px solid var(--border);
    background: var(--surface);
    flex-wrap: wrap;
  }
  /* Leave room for the macOS traffic lights in a dedicated Tauri window
     (overlay titlebar, hidden title — see tauri.conf.json). */
  .snip-bar.tauri-pad {
    padding-left: 84px;
  }
  .group {
    display: flex;
    align-items: center;
    gap: 2px;
  }
  .group + .group {
    border-left: 1px solid var(--border);
    padding-left: 10px;
  }
  .spacer {
    flex: 1;
  }
  .tb {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    border: 1px solid transparent;
    background: none;
    color: var(--text-dim);
    border-radius: var(--radius-s);
    padding: 4px 8px;
    font-size: 12px;
    cursor: pointer;
    white-space: nowrap;
  }
  .tb:hover {
    color: var(--text);
    background: color-mix(in srgb, var(--text) 8%, transparent);
  }
  .tb.active {
    color: var(--text);
    border-color: var(--accent);
    background: color-mix(in srgb, var(--accent) 14%, transparent);
  }
  .tb.primary {
    color: var(--text);
    border-color: var(--accent);
    background: var(--accent);
  }
  .tb:disabled {
    opacity: 0.4;
    cursor: default;
  }
  .tb .icon {
    font-size: 13px;
  }
  .swatch {
    width: 18px;
    height: 18px;
    border-radius: 50%;
    border: 2px solid transparent;
    cursor: pointer;
    padding: 0;
    box-shadow: inset 0 0 0 1px rgba(0, 0, 0, 0.25);
  }
  .swatch.active {
    border-color: var(--text);
  }
  .tb.size {
    width: 26px;
    justify-content: center;
  }
  .snip-copied {
    font-size: 12px;
    color: var(--text-dim);
  }
  .snip-copied.ok {
    color: var(--accent);
  }
  .snip-copied.bad {
    color: #e5484d;
  }
  .snip-body {
    position: relative;
    flex: 1;
    overflow: auto;
    display: grid;
    place-items: center;
    padding: 16px;
  }
  .snip-canvas {
    max-width: 100%;
    max-height: 100%;
    box-shadow: 0 4px 24px rgba(0, 0, 0, 0.35);
    border-radius: 4px;
    touch-action: none;
    cursor: crosshair;
  }
  .snip-textentry {
    position: absolute;
    min-width: 160px;
    min-height: 1.4em;
    background: color-mix(in srgb, var(--bg) 70%, transparent);
    border: 1px dashed var(--accent);
    border-radius: 4px;
    padding: 2px 4px;
    font-weight: 600;
    line-height: 1.25;
    resize: both;
    outline: none;
  }
  .snip-empty {
    display: flex;
    flex-direction: column;
    gap: 12px;
    align-items: center;
    color: var(--text-dim);
  }
  @media (max-width: 760px) {
    .tb .lbl {
      display: none;
    }
  }
</style>
