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
  import { ApiError } from '../../lib/api/client';
  import { toasts } from '../../lib/toast.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import Icon, { type IconName } from '../../lib/components/Icon.svelte';
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

  // The shell keys this editor by id; cleanup saves belong to that mounted image.
  const snipId = router.parts[1] ?? '';

  let img: HTMLImageElement | null = $state(null);
  let missing = $state(false);
  let loadError = $state('');
  let imageUrl: string | null = null;
  let destroyed = false;
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
  // Move/resize history is lazy: the pre-drag snapshot is stashed on
  // pointerdown but only pushed on the first REAL change, so a plain
  // select-click doesn't grow the undo stack or trigger a no-op re-copy.
  let dragSnap: Anno[] | null = null;
  let dragChanged = false;
  let nextId = 1;
  let nextBadge = 1;

  // Text overlay editing.
  let textDraft = $state<{ x: number; y: number; value: string; editId: number | null } | null>(null);
  let textareaEl: HTMLTextAreaElement | undefined = $state();

  const isSecondaryWindow =
    typeof window !== 'undefined' &&
    !!(window as unknown as { __OTTO_WIN__?: string }).__OTTO_WIN__ &&
    (window as unknown as { __OTTO_WIN__?: string }).__OTTO_WIN__ !== 'main';

  async function loadImage(): Promise<void> {
    loading = true;
    missing = false;
    loadError = '';
    try {
      const url = await snipApi.imageUrl(snipId);
      if (destroyed) { URL.revokeObjectURL(url); return; }
      if (imageUrl) URL.revokeObjectURL(imageUrl);
      imageUrl = url;
      const el = new Image();
      el.onload = () => {
        if (destroyed) return;
        img = el;
        loading = false;
        queueMicrotask(redraw);
      };
      el.onerror = () => {
        if (destroyed) return;
        loadError = 'The image could not be decoded. Retry loading the snip.';
        loading = false;
      };
      el.src = url;
    } catch (e) {
      if (destroyed) return;
      missing = e instanceof ApiError && e.status === 404;
      loadError = missing ? '' : e instanceof Error ? e.message : 'Could not load the image. Try again.';
      loading = false;
    }
  }

  onMount(() => {
    void loadImage();
    return () => {
      destroyed = true;
      if (copyTimer) {
        // Closed inside the 800 ms debounce: still copy/save the last
        // annotation instead of silently dropping it. (The loaded image stays
        // drawable after its object URL is revoked.)
        clearTimeout(copyTimer);
        copyTimer = null;
        void copyNow();
      }
      if (imageUrl) URL.revokeObjectURL(imageUrl);
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

  function cloneAnnos(): Anno[] {
    return annos.map((a) => ({ ...a, points: a.points?.slice() }));
  }

  function snapshot(): void {
    undoStack = [...undoStack, cloneAnnos()];
    redoStack = [];
  }

  function beginDrag(mode: 'move' | 'resize'): void {
    dragMode = mode;
    dragSnap = cloneAnnos();
    dragChanged = false;
  }

  /** First real mutation of a move/resize commits the stashed snapshot. */
  function markDragChanged(): void {
    if (dragChanged || !dragSnap) return;
    undoStack = [...undoStack, dragSnap];
    redoStack = [];
    dragChanged = true;
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
    if (!img) {
      copyState = 'idle'; // never stick on "Copying…" before the image loads
      return;
    }
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
    // Prevent the mousedown default action (focus change / text selection):
    // it would blur — and thereby cancel — a text draft opened by this very
    // click, since the default action runs AFTER the handler.
    e.preventDefault();
    canvasEl!.setPointerCapture(e.pointerId);
    const p = toImage(e);
    dragLast = p;

    if (tool === 'select') {
      const sel = annos.find((a) => a.id === selected);
      if (sel) {
        const h = hitHandle(sel, p.x, p.y);
        if (h !== null) {
          beginDrag('resize');
          dragHandle = h;
          return;
        }
      }
      const hit = hitTest(annos, p.x, p.y);
      selected = hit?.id ?? null;
      if (hit) beginDrag('move');
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
      if (dx !== 0 || dy !== 0) markDragChanged();
      annos = annos.map((a) => (a.id === selected ? moveAnno(a, dx, dy) : a));
    } else if (dragMode === 'resize' && selected !== null) {
      markDragChanged();
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
    } else if ((dragMode === 'move' || dragMode === 'resize') && dragChanged) {
      scheduleCopy();
    }
    dragMode = null;
    dragSnap = null;
  }

  // ── Text tool ──────────────────────────────────────────────────────────────

  function openTextDraft(x: number, y: number, value: string, editId: number | null): void {
    textDraft = { x, y, value, editId };
    // Focus after the full click dispatch (not a microtask): the click's
    // remaining default actions would blur-and-cancel the draft otherwise.
    setTimeout(() => textareaEl?.focus(), 0);
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
    // The image coordinate remains unchanged; only the editing control moves
    // inward so text can be entered near the image's right/bottom edges.
    const width = Math.min(260, wrap.width - 16);
    const left = Math.max(8, Math.min(r.left - wrap.left + textDraft.x * sx, wrap.width - width - 8));
    const top = Math.max(8, Math.min(r.top - wrap.top + textDraft.y * sy, wrap.height - 84));
    const fs = Math.max(11, FONTS[fontIx] * sy);
    return `left:${left}px;top:${top}px;width:${width}px;max-width:${wrap.width - left - 8}px;max-height:${wrap.height - top - 8}px;font-size:${fs}px;color:${color};`;
  });

  // ── Keyboard ───────────────────────────────────────────────────────────────

  function onKeydown(e: KeyboardEvent): void {
    // The Delete confirm is up: its keys (Esc, Enter, Tab) belong to it — a
    // Backspace here must not delete the selected annotation behind it.
    if (confirmer.open) return;
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
    } else if (!mod && !e.altKey && e.key.length === 1 && !isTypingTarget(e.target)) {
      // Single-letter tool keys — the toolbar tooltips always advertised
      // them (V, R, O…) but nothing listened.
      const t = TOOLS.find((x) => x.key === e.key.toLowerCase());
      if (t) {
        e.preventDefault();
        pickTool(t.id);
      }
    } else if (e.key.startsWith('Arrow') && selected !== null) {
      e.preventDefault();
      const d = e.shiftKey ? 10 : 1;
      const dx = e.key === 'ArrowLeft' ? -d : e.key === 'ArrowRight' ? d : 0;
      const dy = e.key === 'ArrowUp' ? -d : e.key === 'ArrowDown' ? d : 0;
      snapshot();
      commit(annos.map((a) => (a.id === selected ? moveAnno(a, dx, dy) : a)));
    }
  }

  /** A keystroke meant for a field (⌘K palette, a dialog) — not a tool key. */
  function isTypingTarget(t: EventTarget | null): boolean {
    const el = t as HTMLElement | null;
    return !!el && (el.isContentEditable || /^(INPUT|TEXTAREA|SELECT)$/.test(el.tagName));
  }

  function pickTool(id: Tool): void {
    tool = id;
    if (id !== 'select') selected = null;
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
    const ok = await confirmer.ask('Delete this snip and its annotations? This can’t be undone.', {
      title: 'Delete snip',
      confirmLabel: 'Delete snip',
    });
    if (!ok) return;
    try {
      await snipApi.remove(snipId);
      toasts.info('Snip deleted');
      await close();
    } catch (e) {
      toasts.error('Delete failed', e instanceof Error ? e.message : String(e));
    }
  }

  // Tool icons come from the shared `Icon` set (16-unit viewBox, 1.4 stroke,
  // round caps) so the rail reads as one set — never a text glyph (they render
  // at different weights/baselines per font and flip meaning in RTL).
  const TOOLS: { id: Tool; label: string; icon: IconName; title: string; key: string }[] = [
    { id: 'select', label: 'Select', icon: 'cursor', title: 'Select / move (V)', key: 'v' },
    { id: 'rect', label: 'Box', icon: 'rectangle', title: 'Rectangle (R)', key: 'r' },
    { id: 'ellipse', label: 'Ellipse', icon: 'ellipse', title: 'Ellipse (O)', key: 'o' },
    { id: 'arrow', label: 'Arrow', icon: 'arrow', title: 'Arrow (A)', key: 'a' },
    { id: 'line', label: 'Line', icon: 'line', title: 'Line (L)', key: 'l' },
    { id: 'pen', label: 'Pen', icon: 'edit', title: 'Freehand (P)', key: 'p' },
    { id: 'highlight', label: 'Highlight', icon: 'marker', title: 'Highlighter (H)', key: 'h' },
    { id: 'text', label: 'Text', icon: 'text', title: 'Text (T)', key: 't' },
    { id: 'pixelate', label: 'Blur', icon: 'grid', title: 'Pixelate region (B)', key: 'b' },
    { id: 'badge', label: 'Step', icon: 'step', title: 'Numbered step (N)', key: 'n' },
  ];
  /** Human names for the palette swatches (tooltips/aria — not hex codes). */
  const COLOR_NAMES = ['Red', 'Orange', 'Yellow', 'Green', 'Blue', 'Purple', 'Black', 'White'];
  const SIZE_NAMES = ['Small', 'Medium', 'Large'];

  const copyLabel = $derived.by(() => {
    switch (copyState) {
      case 'copied':
        return 'Copied';
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
          aria-label={t.title}
          aria-pressed={tool === t.id}
          onclick={() => pickTool(t.id)}
        >
          <Icon name={t.icon} size={14} />
          <span class="lbl">{t.label}</span>
        </button>
      {/each}
    </div>
    <div class="group colors" role="toolbar" aria-label="Colors">
      {#each PALETTE as c, i (c)}
        <button
          class="swatch"
          class:active={color === c}
          data-color={c}
          style={`background:${c}`}
          title={COLOR_NAMES[i] ?? c}
          aria-label={`Color: ${COLOR_NAMES[i] ?? c}`}
          aria-pressed={color === c}
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
          title={`${SIZE_NAMES[i]} stroke and text`}
          aria-label={`${SIZE_NAMES[i]} stroke and text`}
          aria-pressed={strokeIx === i}
          onclick={() => {
            strokeIx = i;
            fontIx = i;
          }}>{s}</button
        >
      {/each}
    </div>
    <div class="group history">
      <button class="tb" data-act="undo" title="Undo (⌘Z)" aria-label="Undo" disabled={!undoStack.length} onclick={undo}>
        <Icon name="undo" size={13} />
      </button>
      <button class="tb" data-act="redo" title="Redo (⇧⌘Z)" aria-label="Redo" disabled={!redoStack.length} onclick={redo}>
        <span class="mirror"><Icon name="undo" size={13} /></span>
      </button>
    </div>
    <!-- Delete lives in its own group, away from the primary Copy, and asks first. -->
    <div class="group">
      <button class="tb snip-del" data-act="delete-snip" title="Delete this snip…" aria-label="Delete this snip…" onclick={() => void deleteSnip()}>
        <Icon name="trash" size={13} />
      </button>
    </div>
    <div class="spacer"></div>
    <span class="snip-copied" role="status" class:ok={copyState === 'copied' || copyState === 'idle'} class:bad={copyState === 'failed'}
      title="Every change is copied to the clipboard automatically"
      >{#if copyState === 'copied' || copyState === 'idle'}<Icon name="check" size={12} />{:else if copyState === 'failed'}<Icon name="warning" size={12} />{/if}{copyLabel}</span
    >
    <div class="group actions">
      <button class="btn small ghost snip-action" data-act="close" title="Close the editor (the clipboard keeps the latest copy)" onclick={() => void close()}>Close</button>
      <button class="btn small primary snip-action" data-act="copy" title="Copy now (⌘C)" onclick={() => void copyNow()}><Icon name="copy" size={12} /> Copy</button>
    </div>
  </header>

  <div class="snip-body" bind:this={wrapEl}>
    {#if loading}
      <div class="snip-empty" role="status">Loading the snip…</div>
    {:else if loadError}
      <div class="snip-empty" role="alert">
        <p class="snip-missing-title">Could not load the snip</p>
        <p>{loadError}</p>
        <button class="btn" onclick={() => void loadImage()}>Retry</button>
      </div>
    {:else if missing}
      <div class="snip-empty snip-missing" role="alert">
        <Icon name="image" size={26} />
        <p class="snip-missing-title">This snip no longer exists</p>
        <p>It was deleted, or its image file is gone. Take a new one with ⌘⇧S.</p>
        <button class="btn" data-act="close" onclick={() => void close()}>Close</button>
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
          aria-label="Annotation text"
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
    gap: 8px;
    padding: 8px 12px;
    border-bottom: 1px solid var(--border);
    background: var(--surface);
    flex-wrap: wrap;
  }
  /* Leave room for the macOS traffic lights in a dedicated Tauri window
     (overlay titlebar, hidden title — see tauri.conf.json). */
  .snip-bar.tauri-pad {
    padding-inline-start: 84px;
  }
  .group {
    display: flex;
    align-items: center;
    gap: 2px;
  }
  .group + .group {
    border-inline-start: 1px solid var(--border);
    padding-inline-start: 8px;
  }
  .spacer {
    flex: 1;
  }
  .tb {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    border: 1px solid transparent;
    background: none;
    color: var(--text-dim);
    border-radius: var(--radius-s);
    padding: 4px 6px;
    font-size: var(--fs-s);
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
  /* The icon set has no redo glyph: redo is undo, mirrored. */
  .mirror {
    display: inline-flex;
    transform: scaleX(-1);
  }
  .tb.snip-del:hover {
    color: var(--danger);
    background: var(--danger-soft);
  }
  .tb:disabled {
    opacity: 0.4;
    cursor: default;
  }
  .swatch {
    width: 18px;
    height: 18px;
    border-radius: 50%;
    border: 2px solid transparent;
    cursor: pointer;
    padding: 0;
    /* A token ring, so Black reads on the dark bar and White on the light one. */
    box-shadow: inset 0 0 0 1px var(--border-strong);
  }
  .swatch.active {
    border-color: var(--text);
  }
  .tb.size {
    width: 26px;
    justify-content: center;
  }
  .snip-copied {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    font-size: var(--fs-s);
    color: var(--text-dim);
    white-space: nowrap;
  }
  .snip-copied.ok {
    color: var(--accent-text);
  }
  .snip-copied.bad {
    color: var(--danger);
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
    box-shadow: var(--shadow);
    border-radius: var(--radius-s);
    touch-action: none;
    cursor: crosshair;
  }
  .snip-textentry {
    position: absolute;
    min-width: 0;
    box-sizing: border-box;
    min-height: 1.4em;
    background: color-mix(in srgb, var(--bg) 70%, transparent);
    border: 1px dashed var(--accent);
    border-radius: var(--radius-s);
    padding: 2px 4px;
    font-weight: 600;
    line-height: 1.25;
    resize: both;
    outline: none;
  }
  .snip-empty {
    display: flex;
    flex-direction: column;
    gap: 8px;
    align-items: center;
    text-align: center;
    max-width: 360px;
    color: var(--text-dim);
    font-size: var(--fs-m);
  }
  .snip-empty p {
    margin: 0;
  }
  .snip-missing-title {
    color: var(--text);
    font-weight: 600;
  }
  @media (max-width: 640px) {
    .tools { display: grid; grid-template-columns: repeat(5, minmax(36px, 1fr)); width: 100%; }
    .tools .tb { justify-content: center; }
    .snip-action { min-height: 36px; }
    .tb, .tb.size, .swatch { min-width: 36px; min-height: 36px; }
    .snip-bar { gap: 6px; padding: 8px; }
    .colors { flex-wrap: wrap; }
  }
  /* Narrow windows: tools go icon-only (the tooltip keeps name + key) so
     the bar stays one row as long as possible. */
  @media (max-width: 1024px) {
    .tb .lbl {
      display: none;
    }
  }
</style>
