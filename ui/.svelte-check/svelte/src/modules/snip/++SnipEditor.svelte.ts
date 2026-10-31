///<reference types="svelte" />
;
import { toastError } from '../../lib/toastError';
// Snip annotation editor — chrome-less full-screen view at `#/snip/{id}`.
//
// Single canvas at the image's natural pixel size, CSS-scaled to fit; all
// geometry stays in image-pixel space (pointer events are mapped through the
// bounding-rect scale). Every committed mutation schedules a debounced
// flatten → POST /snips/{id}/annotated, which puts the latest state on the
// clipboard — the user can paste into a session at any moment (R4).
import { onMount, tick, untrack } from 'svelte';
import { router } from '../../lib/router.svelte';
import { snipApi } from '../../lib/snip';
import { ApiError } from '../../lib/api/client';
import { toasts } from '../../lib/toast.svelte';
import { loadErrorText } from '../../lib/loadError';
import { confirmer } from '../../lib/confirm.svelte';
import Icon, { type IconName } from '../../lib/components/Icon.svelte';
import { isTauri, ui } from '../../lib/stores/ui.svelte';
import {
    PALETTE,
    STROKES,
    FONTS,
    type Anno,
    type Tool,
    render,
    renderOver,
    drawSelection,
    hitTest,
    hitHandle,
    moveAnno,
    resizeAnno,
    bounds,
    flatten,
    annosHash,
    uploadNeeded,
  } from './annotations';
import LoadState from '../../lib/components/LoadState.svelte';
import Skeleton from '../../lib/components/Skeleton.svelte';
function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
   

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

  // `$state.raw`: every mutation replaces the array (commit/map), so deep
  // proxies only cost a proxy per point of every pen stroke (S3).
  let annos: Anno[] = $state.raw([]);
  let selected: number | null = $state(null); // Anno id
  let tool: Tool = $state('rect');
  let color: string = $state(PALETTE[0]);
  let strokeIx = $state(1);
  let fontIx = $state(1);

  // Undo/redo: snapshots of the object list (cheap — plain JSON objects).
  // Raw + capped: an hour of editing kept every snapshot forever (S4).
  const UNDO_CAP = 100;
  let undoStack: Anno[][] = $state.raw([]);
  let redoStack: Anno[][] = $state.raw([]);
  function pushUndo(s: Anno[]): void {
    undoStack = [...undoStack.slice(-(UNDO_CAP - 1)), s];
  }

  // Auto-copy machinery.
  let copyState: 'idle' | 'pending' | 'copying' | 'copied' | 'failed' = $state('idle');
  let copyTimer: ReturnType<typeof setTimeout> | null = null;
  let copyInFlight: Promise<boolean> | null = null;
  let leavePending: Promise<boolean> | null = null;
  let allowClose = false;
  // Fingerprint of the annotations last saved successfully — an unchanged
  // state is never re-encoded or re-uploaded. `annosHash([])` = the original.
  let savedHash = annosHash([]);

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
        loadError = 'The image couldn’t be decoded. Retry loading the snip.';
        loading = false;
      };
      el.src = url;
    } catch (e) {
      if (destroyed) return;
      missing = e instanceof ApiError && e.status === 404;
      loadError = missing ? '' : e instanceof Error ? e.message : 'Couldn’t load the image. Try again.';
      loading = false;
    }
  }

  // The editor is a full-screen overlay on the Modal layer: register it so
  // the native browser webview hides under it (untracked: pushModal reads the
  // counter it bumps — see Modal.svelte).
  $effect(() => {
    untrack(() => ui.pushModal());
    return () => untrack(() => ui.popModal());
  });

  onMount(() => {
    void loadImage();
    return () => {
      destroyed = true;
      if (frameQueued) cancelAnimationFrame(frameQueued);
      if (baseLayer) { baseLayer.width = baseLayer.height = 0; baseLayer = null; }
      if (copyTimer) { clearTimeout(copyTimer); copyTimer = null; }
      if (imageUrl) URL.revokeObjectURL(imageUrl);
    };
  });

  // Committed layer (image + `annos`) cached off-screen while a shape/pen
  // stroke is being drawn, so each frame blits it and draws only the stroke
  // (a pen path used to re-render the full-resolution image + every
  // annotation + the whole growing path on every pointer move).
  let baseLayer: HTMLCanvasElement | null = null;
  let baseFor: Anno[] | null = null;
  function committedLayer(image: HTMLImageElement): HTMLCanvasElement | null {
    if (baseLayer && baseFor === annos && baseLayer.width === image.width && baseLayer.height === image.height) return baseLayer;
    baseLayer ??= document.createElement('canvas');
    baseLayer.width = image.width;
    baseLayer.height = image.height;
    const bctx = baseLayer.getContext('2d');
    if (!bctx) return null;
    render(bctx, image, annos);
    baseFor = annos;
    return baseLayer;
  }

  // Move/resize: every OTHER annotation is fixed for the whole drag, so the
  // image + those are rendered once and each frame blits them and draws only
  // the dragged one (it was a full-resolution re-render per pointer move).
  // The dragged shape sits on top while dragging; the full render on release
  // restores its z-order.
  let dragBase: HTMLCanvasElement | null = null;
  function dragLayer(image: HTMLImageElement): HTMLCanvasElement | null {
    if (dragBase) return dragBase;
    const c = document.createElement('canvas');
    c.width = image.width;
    c.height = image.height;
    const bctx = c.getContext('2d');
    if (!bctx) return null;
    render(bctx, image, annos.filter((a) => a.id !== selected));
    dragBase = c;
    return c;
  }

  function redraw(): void {
    if (!canvasEl || !img) return;
    if (canvasEl.width !== img.width) canvasEl.width = img.width;
    if (canvasEl.height !== img.height) canvasEl.height = img.height;
    const ctx = canvasEl.getContext('2d');
    if (!ctx) return;
    const sel = annos.find((a) => a.id === selected);
    const moving = (dragMode === 'move' || dragMode === 'resize') && sel;
    const base = drafting ? committedLayer(img) : moving ? dragLayer(img) : null;
    if (drafting && base) renderOver(ctx, base, img, [drafting]);
    else if (moving && base) renderOver(ctx, base, img, [moving]);
    else {
      render(ctx, img, drafting ? [...annos, drafting] : annos);
      // Nothing is being drawn: free the off-screen copy (a 5K snip is ~60 MB).
      if (baseLayer) { baseLayer.width = baseLayer.height = 0; baseLayer = null; baseFor = null; }
    }
    if (!moving && dragBase) { dragBase.width = dragBase.height = 0; dragBase = null; }
    if (sel) drawSelection(ctx, sel);
  }

  // At most one repaint per frame, however many pointer events / state
  // changes arrive in between (trackpads and 120 Hz displays deliver several
  // pointermoves per frame).
  let frameQueued = 0;
  function requestRedraw(): void {
    if (frameQueued) return;
    frameQueued = requestAnimationFrame(() => {
      frameQueued = 0;
      redraw();
    });
  }

  $effect(() => {
    // Redraw on any committed state change (annos/selected are reactive), and
    // once the image / canvas exist — the repaint itself runs in a frame
    // callback, outside this effect, so every dependency is read here.
    void annos;
    void selected;
    void img;
    void canvasEl;
    requestRedraw();
  });

  // ── History + auto-copy ────────────────────────────────────────────────────

  function cloneAnnos(): Anno[] {
    return annos.map((a) => ({ ...a, points: a.points?.slice() }));
  }

  function snapshot(): void {
    pushUndo(cloneAnnos());
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
    pushUndo(dragSnap);
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

  /** Return persistence success independently of clipboard success. */
  async function copyNow(explicit = false): Promise<boolean> {
    if (copyTimer) { clearTimeout(copyTimer); copyTimer = null; }
    while (copyInFlight) await copyInFlight;
    if (!img) return uploadNeeded(annos, savedHash) === null;
    const snapshot = annos;
    const hash = uploadNeeded(snapshot, savedHash);
    if (hash === null && !explicit) return true;
    copyState = 'copying';
    const pending = (async () => {
      try {
        const resp = hash === null
          ? await snipApi.copy(snipId)
          : await snipApi.saveAnnotatedPng(snipId, await flatten(img!, snapshot));
        if (hash !== null) savedHash = hash;
        copyState = resp.copied ? 'copied' : 'failed';
        return true;
      } catch (e) {
        copyState = 'failed';
        toasts.error('Couldn’t copy the snip', `${loadErrorText(e)} Try Copy (⌘C) again — the snip stays saved in Otto.`);
        return hash === null;
      }
    })();
    copyInFlight = pending;
    try { return await pending; }
    finally { if (copyInFlight === pending) copyInFlight = null; }
  }

  async function drainPersistence(): Promise<boolean> {
    commitText();
    if (copyTimer) { clearTimeout(copyTimer); copyTimer = null; }
    while (copyInFlight) await copyInFlight;
    while (uploadNeeded(annos, savedHash) !== null) {
      if (!(await copyNow())) return false;
    }
    return true;
  }

  async function approveLeave(): Promise<boolean> {
    if (allowClose) return true;
    if (leavePending) return leavePending;
    const pending = (async () => {
      if (await drainPersistence()) return true;
      const { value } = await confirmer.choose('The latest annotations couldn’t be saved. Keep the editor open to try again, or discard these changes.', {
        title: 'Annotations not saved',
        options: [
          { label: 'Retry saving', value: 'retry', kind: 'primary' },
          { label: 'Discard changes', value: 'discard', kind: 'danger' },
        ],
      });
      if (value === 'retry') return drainPersistence();
      return value === 'discard';
    })();
    leavePending = pending;
    try { return await pending; }
    finally { if (leavePending === pending) leavePending = null; }
  }

  $effect(() => router.guard(approveLeave));
  onMount(() => {
    if (!isTauri) return;
    let disposed = false;
    let unlisten: (() => void) | undefined;
    void import('@tauri-apps/api/window').then(async ({ getCurrentWindow }) => {
      const win = getCurrentWindow();
      const stop = await win.onCloseRequested((event) => {
        if (allowClose) return;
        event.preventDefault();
        void approveLeave().then(async (allowed) => {
          if (!allowed || disposed) return;
          allowClose = true;
          try { await win.close(); }
          catch (e) { allowClose = false; toastError('Couldn’t close the editor', e); }
        });
      });
      if (disposed) stop(); else unlisten = stop;
    }).catch((e) => toasts.warn('Closing this window won’t ask about unsaved annotations', loadErrorText(e)));
    return () => { disposed = true; unlisten?.(); };
  });

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
    pushUndo(annos);
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
      requestRedraw();
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
      requestRedraw();
    } else if (dragMode === 'move' || dragMode === 'resize') {
      if (dragChanged) scheduleCopy();
      // Full render restores the dragged shape's z-order + frees dragBase.
      requestRedraw();
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

  function fitTextInImage(a: Anno): Anno {
    const ctx = canvasEl?.getContext('2d');
    if (!img || !ctx) return a;
    ctx.save();
    ctx.font = `600 ${a.font}px ui-sans-serif, system-ui, sans-serif`;
    const lines = (a.text ?? '').split('\n');
    const width = Math.max(...lines.map(line => ctx.measureText(line).width));
    ctx.restore();
    const x = Math.max(4, Math.min(a.x1, img.width - width - 4));
    const y = Math.max(4, Math.min(a.y1, img.height - a.font * 1.25 * lines.length - 4));
    return moveAnno(a, x - a.x1, y - a.y1);
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
      commit(annos.map((a) => (a.id === editId ? fitTextInImage({ ...a, text }) : a)));
    } else {
      commit([
        ...annos,
        fitTextInImage({
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
        }),
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
    // Keep the input reachable at image edges. Committing measures the text
    // separately and fits the annotation itself within the image.
    const width = Math.min(260, wrap.width - 16);
    const left = Math.max(8, Math.min(r.left - wrap.left + textDraft.x * sx, wrap.width - width - 8));
    const top = Math.max(8, Math.min(r.top - wrap.top + textDraft.y * sy, wrap.height - 84));
    const fs = Math.max(11, FONTS[fontIx] * sy);
    return `left:${left}px;top:${top}px;width:${width}px;max-width:${wrap.width - left - 8}px;max-height:${wrap.height - top - 8}px;font-size:${fs}px;color:${color};`;
  });

  // Keyboard placement starts at the image center. The same selection, undo,
  // move and copy paths serve both pointer and keyboard changes.
  async function focusCanvas(): Promise<void> {
    await tick();
    canvasEl?.focus();
  }
  function placeWithKeyboard(): void {
    if (!img) return;
    const current = annos.find(a => a.id === selected);
    if (current?.tool === 'text') {
      openTextDraft(current.x1, current.y1, current.text ?? '', current.id);
      return;
    }
    if (tool === 'select') {
      selected = annos[0]?.id ?? null;
      return;
    }
    const x = Math.round(img.width / 2), y = Math.round(img.height / 2);
    if (tool === 'text') { openTextDraft(x, y, '', null); return; }
    const w = Math.min(120, img.width / 4), h = Math.min(80, img.height / 4);
    const a: Anno = {
      id: nextId++, tool, x1: x - w / 2, y1: y - h / 2, x2: x + w / 2, y2: y + h / 2,
      color, stroke: STROKES[strokeIx], font: FONTS[fontIx],
      ...(tool === 'badge' ? { n: nextBadge++ } : {}),
      ...(tool === 'pen' || tool === 'highlight' ? { points: [{ x: x - w / 2, y }, { x: x + w / 2, y }] } : {}),
    };
    snapshot(); commit([...annos, a]); selected = a.id;
  }

  // ── Keyboard ───────────────────────────────────────────────────────────────

  function onKeydown(e: KeyboardEvent): void {
    // The Delete confirm is up: its keys (Esc, Enter, Tab) belong to it — a
    // Backspace here must not delete the selected annotation behind it.
    if (confirmer.open) return;
    if (textDraft) {
      if (e.key === 'Escape') {
        textDraft = null;
        void focusCanvas();
        e.stopPropagation();
      } else if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) {
        commitText();
        void focusCanvas();
        e.stopPropagation();
      }
      return;
    }
    if (e.target === canvasEl && e.key === 'Enter') {
      e.preventDefault(); placeWithKeyboard(); return;
    }
    if (e.target === canvasEl && (e.key === '[' || e.key === ']')) {
      e.preventDefault();
      const index = annos.findIndex(a => a.id === selected);
      selected = annos[(index + (e.key === ']' ? 1 : -1) + annos.length) % annos.length]?.id ?? null;
      return;
    }
    const mod = e.metaKey || e.ctrlKey;
    if (mod && e.key.toLowerCase() === 'z') {
      e.preventDefault();
      if (e.shiftKey) redo();
      else undo();
    } else if (mod && e.key.toLowerCase() === 'c' && selected === null) {
      e.preventDefault();
      void copyNow(true);
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
      if (!(await approveLeave())) return;
      allowClose = true;
      try {
        const { getCurrentWindow } = await import('@tauri-apps/api/window');
        await getCurrentWindow().close();
        return;
      } catch {
        allowClose = false;
        // fall through to the guarded router
      }
    }
    if (router.canGoBack) router.back(); // a relaunch restores this route with nothing behind it
    else router.go('agents');
  }

  async function deleteSnip(): Promise<void> {
    const ok = await confirmer.ask('Delete this snip and its annotations? This can’t be undone.', {
      title: 'Delete snip',
      danger: true, confirmLabel: 'Delete snip',
    });
    if (!ok) return;
    try {
      await snipApi.remove(snipId);
      allowClose = true;
      toasts.info('Snip deleted');
      await close();
    } catch (e) {
      toastError('Couldn’t delete', e);
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
;
async () => {

 { svelteHTML.createElement("svelte:window", {  "onkeydown":onKeydown,});}

 { svelteHTML.createElement("div", {   "class":`snip-editor`,...__sveltets_2_empty({"data-count":annos.length}),});
   { svelteHTML.createElement("header", {   "class":`snip-bar`,...__sveltets_2_empty({"data-tauri-drag-region":true}),});isTauri && isSecondaryWindow;
     { svelteHTML.createElement("div", {     "class":`group tools`,"role":`toolbar`,"aria-label":`Annotation tools`,});
         for(let t of __sveltets_2_ensureArray(TOOLS)){t.id;
         { svelteHTML.createElement("button", {              "class":`tb`,...__sveltets_2_empty({"data-tool":t.id}),"title":t.title,"aria-label":t.title,"aria-pressed":tool === t.id,"onclick":() => pickTool(t.id),});tool === t.id;
           { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":t.icon,"size":14,}});}
           { svelteHTML.createElement("span", { "class":`lbl`,});t.label; }
         }
      }
     }
     { svelteHTML.createElement("div", {     "class":`group colors`,"role":`toolbar`,"aria-label":`Colors`,});
          for(let c of __sveltets_2_ensureArray(PALETTE)){let i = 1;c;
         { svelteHTML.createElement("button", {                "class":`swatch`,...__sveltets_2_empty({"data-color":c}),"style":`background:${c}`,"title":COLOR_NAMES[i] ?? c,"aria-label":`Color: ${COLOR_NAMES[i] ?? c}`,"aria-pressed":color === c,"onclick":() => (color = c),});color === c; }
      }
     }
     { svelteHTML.createElement("div", {     "class":`group sizes`,"role":`toolbar`,"aria-label":`Stroke width`,});
          for(let s of __sveltets_2_ensureArray((['S', 'M', 'L']))){let i = 1;s;
         { svelteHTML.createElement("button", {             "class":`tb size`,...__sveltets_2_empty({"data-stroke":s}),"title":`${SIZE_NAMES[i]} stroke and text`,"aria-label":`${SIZE_NAMES[i]} stroke and text`,"aria-pressed":strokeIx === i,"onclick":() => {
            strokeIx = i;
            fontIx = i;
          },});strokeIx === i;s; }
      }
     }
     { svelteHTML.createElement("div", { "class":`group history`,});
       { svelteHTML.createElement("button", {             "class":`tb`,...__sveltets_2_empty({"data-act":`undo`}),"title":`Undo`,"aria-keyshortcuts":`Meta+Z`,"aria-label":`Undo`,"disabled":!undoStack.length,"onclick":undo,});
         { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`undo`,"size":13,}});}
       }
       { svelteHTML.createElement("button", {             "class":`tb`,...__sveltets_2_empty({"data-act":`redo`}),"title":`Redo`,"aria-keyshortcuts":`Shift+Meta+Z`,"aria-label":`Redo`,"disabled":!redoStack.length,"onclick":redo,});
         { svelteHTML.createElement("span", { "class":`mirror`,}); { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`undo`,"size":13,}});} }
       }
     }
    
     { svelteHTML.createElement("div", { "class":`group`,});
       { svelteHTML.createElement("button", {         "class":`tb snip-del`,...__sveltets_2_empty({"data-act":`delete-snip`}),"title":`Delete this snip…`,"aria-label":`Delete this snip…`,"onclick":() => void deleteSnip(),});
         { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`trash`,"size":13,}});}
       }
     }
     { svelteHTML.createElement("div", { "class":`spacer`,}); }
     { svelteHTML.createElement("span", {        "class":`snip-copied`,"role":`status`,"title":`Every change is copied to the clipboard automatically`,});copyState === 'copied' || copyState === 'idle';copyState === 'failed';if(copyState === 'copied' || copyState === 'idle'){ { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`check`,"size":12,}});}} else if (copyState === 'failed'){ { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`warning`,"size":12,}});}}copyLabel; }
     { svelteHTML.createElement("div", { "class":`group actions`,});
       { svelteHTML.createElement("button", {       "class":`btn small ghost snip-action`,...__sveltets_2_empty({"data-act":`close`}),"title":`Close the editor (the clipboard keeps the latest copy)`,"onclick":() => void close(),});  }
       { svelteHTML.createElement("button", {       "class":`btn small primary snip-action`,...__sveltets_2_empty({"data-act":`copy`}),"title":`Copy now (⌘C)`,"onclick":() => void copyNow(true),}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`copy`,"size":12,}});}  }
     }
   }

   { const $$_div1 = svelteHTML.createElement("div", {  "class":`snip-body`,});wrapEl = $$_div1;
    if(loading){
       { svelteHTML.createElement("div", { "class":`snip-empty`,}); { svelteHTML.createElement("span", {    "class":`spinner`,"aria-hidden":`true`,});__sveltets_2_ensureType(String, Number, " "); } { const $$_notelekS3C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS3C({ target: __sveltets_2_any(), props: {    "rows":0,"label":`the snip`,}});} }
    } else if (loadError){
       { const $$_etatSdaoL2C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL2C({ target: __sveltets_2_any(), props: {        "variant":`compact`,"what":`the snip`,"error":loadError,"empty":true,"onretry":() => void loadImage(),}});}
    } else if (missing){
       { svelteHTML.createElement("div", {   "class":`snip-empty snip-missing`,"role":`alert`,});
         { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`image`,"size":26,}});}
         { svelteHTML.createElement("p", { "class":`snip-missing-title`,});     }
         { svelteHTML.createElement("p", {});               }
         { svelteHTML.createElement("button", {     "class":`btn`,...__sveltets_2_empty({"data-act":`close`}),"onclick":() => void close(),});  }
       }
    }else{
      
       { const $$_canvas2 = svelteHTML.createElement("canvas", {                    "class":`snip-canvas`,"tabindex":0,"role":`application`,"aria-label":`Snip annotation canvas`,"aria-describedby":`snip-keyboard-help`,"onpointerdown":onPointerDown,"onpointermove":onPointerMove,"onpointerup":onPointerUp,"ondblclick":onDblClick,});canvasEl = $$_canvas2; }
       { svelteHTML.createElement("p", {   "id":`snip-keyboard-help`,"class":`snip-keyboard-help`,});                               }
       { svelteHTML.createElement("span", {   "class":`sr-only`,"role":`status`,});selected === null ? `${annos.length} annotations` : `Selected ${annos.find(a => a.id === selected)?.tool ?? 'annotation'} ${annos.findIndex(a => a.id === selected) + 1} of ${annos.length}`; }
      if(textDraft){
         { const $$_textarea2 = svelteHTML.createElement("textarea", {                 "dir":`auto`,"class":`snip-textentry`,"aria-label":`Annotation text`,"style":textOverlayStyle,"bind:value":textDraft.value,"onblur":commitText,"placeholder":`Type… (⌘↩ to commit)`,"rows":2,});textareaEl = $$_textarea2;/*Ωignore_startΩ*/() => textDraft.value = __sveltets_2_any(null);/*Ωignore_endΩ*/ }
      }
    }
   }
 }


};
return { props: {} as Record<string, never>, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const SnipEditor__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type SnipEditor__SvelteComponent_ = ReturnType<typeof SnipEditor__SvelteComponent_>;
/*Ωignore_endΩ*/export default SnipEditor__SvelteComponent_;