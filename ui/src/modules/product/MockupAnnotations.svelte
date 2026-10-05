<script lang="ts">
  // MockupAnnotations — an absolutely-positioned overlay over the mockup render
  // box plus a side list of notes. Two modes:
  //   • Annotate: overlay has pointer-events:auto and captures clicks; a click
  //     drops a pin at relative (x_pct, y_pct) = (offsetX/clientWidth,
  //     offsetY/clientHeight), clamped to 0..1, opening an inline editor.
  //   • Interact: overlay has pointer-events:none so the iframe beneath receives
  //     input (relevant when HTML interactivity is enabled).
  // Coordinates are relative, so pins survive resize. Pins render at
  //   left:{x_pct*100}% top:{y_pct*100}%.
  import { tick } from 'svelte';
  import Skeleton from '../../lib/components/Skeleton.svelte';
  import { toastError } from '../../lib/toastError';
  import { product } from '../../lib/stores/product.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import type { MockupAnnotation } from './types';

  interface Props {
    attachmentId: string;
    // The viewer's render-box element — the overlay is rendered as its child and
    // covers it exactly (so percentage coordinates map 1:1).
    box: HTMLElement;
  }
  const { attachmentId, box }: Props = $props();

  // ── State ─────────────────────────────────────────────────────────────────
  let notes = $state<MockupAnnotation[]>([]);
  let loading = $state(false);
  let mode = $state<'annotate' | 'interact'>('annotate');

  // Inline editor for a pending (un-saved) pin.
  let pending = $state<{ x_pct: number; y_pct: number } | null>(null);
  let pendingBody = $state('');
  let saving = $state(false);

  // Geometry of the render box, tracked so the absolute overlay stays aligned
  // and percentage-positioned pins survive resize. Relative to .render-wrap
  // (the box's offset parent).
  let geom = $state({ left: 0, top: 0, width: 0, height: 0 });

  function measure(): void {
    if (!box) return;
    geom = {
      left: box.offsetLeft,
      top: box.offsetTop,
      width: box.offsetWidth,
      height: box.offsetHeight,
    };
  }

  // ── Load on mount / attachment change ───────────────────────────────────────
  $effect(() => {
    // Re-run when the attachment id changes.
    attachmentId;
    void loadNotes();
  });

  // Keep the overlay aligned to the box (initial + on resize).
  $effect(() => {
    if (!box) return;
    measure();
    const ro = new ResizeObserver(() => measure());
    ro.observe(box);
    window.addEventListener('resize', measure);
    return () => {
      ro.disconnect();
      window.removeEventListener('resize', measure);
    };
  });

  async function loadNotes(): Promise<void> {
    loading = true;
    try {
      notes = await product.listAnnotations(attachmentId);
    } catch (e) {
      toastError('Couldn’t load annotations', e);
    } finally {
      loading = false;
    }
  }

  /** Svelte action: focus the node when it mounts (replaces the `autofocus`
   *  attribute, which the a11y linter flags). */
  function focusOnMount(node: HTMLElement) {
    node.focus();
  }

  const clamp = (v: number): number => Math.max(0, Math.min(1, v));

  /** Annotate-mode click on the overlay → open the inline editor at the spot. */
  function onOverlayClick(e: MouseEvent): void {
    if (mode !== 'annotate') return;
    const el = e.currentTarget as HTMLElement;
    const rect = el.getBoundingClientRect();
    const w = el.clientWidth || rect.width || 1;
    const h = el.clientHeight || rect.height || 1;
    const x = clamp((e.clientX - rect.left) / w);
    const y = clamp((e.clientY - rect.top) / h);
    pending = { x_pct: x, y_pct: y };
    pendingBody = '';
    viaButton = false;
  }

  async function savePending(): Promise<void> {
    if (!pending || saving) return;
    const body = pendingBody.trim();
    if (!body) {
      pending = null;
      return;
    }
    saving = true;
    try {
      const created = await product.addAnnotation(attachmentId, {
        x_pct: pending.x_pct,
        y_pct: pending.y_pct,
        body,
      });
      notes = [...notes, created];
      pending = null;
      pendingBody = '';
      if (viaButton) {
        viaButton = false;
        void tick().then(() => addBtn?.focus());
      }
    } catch (e) {
      toastError('Couldn’t add note', e);
    } finally {
      saving = false;
    }
  }

  function cancelPending(): void {
    pending = null;
    pendingBody = '';
  }

  // ── Keyboard path: "Add annotation" + arrow-key pin placement ───────────────
  // A pin can be started without a mouse: the button drops the pending pin at the
  // centre of what is currently VISIBLE of the render box, the arrow keys on the
  // pin nudge it (⇧ for a big step) in the same normalised coordinates a click
  // produces, and the note saves through the same savePending() path.
  let addBtn = $state<HTMLButtonElement | null>(null);
  /** The pending pin was started from the keyboard button → give focus back to it. */
  let viaButton = false;
  const PIN_STEP = 0.01;
  const PIN_STEP_BIG = 0.05;

  /** The scroll container the overlay lives in (the render box's offset parent). */
  function scroller(): HTMLElement | null {
    return (box?.offsetParent as HTMLElement | null) ?? null;
  }

  function startAtCentre(): void {
    if (mode !== 'annotate') mode = 'annotate';
    const wrap = scroller();
    const w = geom.width || 1;
    const h = geom.height || 1;
    // Centre of the visible part of the box (the box can be taller than the pane).
    const cx = wrap ? wrap.scrollLeft + wrap.clientWidth / 2 - geom.left : w / 2;
    const cy = wrap ? wrap.scrollTop + wrap.clientHeight / 2 - geom.top : h / 2;
    pending = { x_pct: clamp(cx / w), y_pct: clamp(cy / h) };
    pendingBody = '';
    viaButton = true;
  }

  function onPinKey(e: KeyboardEvent): void {
    if (!pending) return;
    const step = e.shiftKey ? PIN_STEP_BIG : PIN_STEP;
    let { x_pct, y_pct } = pending;
    if (e.key === 'ArrowLeft') x_pct -= step;
    else if (e.key === 'ArrowRight') x_pct += step;
    else if (e.key === 'ArrowUp') y_pct -= step;
    else if (e.key === 'ArrowDown') y_pct += step;
    else if (e.key === 'Escape') {
      e.preventDefault();
      closeEditor();
      return;
    } else return;
    e.preventDefault();
    pending = { x_pct: clamp(x_pct), y_pct: clamp(y_pct) };
  }

  /** Esc / Cancel: drop the pending pin; a keyboard-started one returns to Add. */
  function closeEditor(): void {
    const back = viaButton;
    viaButton = false;
    cancelPending();
    if (back) void tick().then(() => addBtn?.focus());
  }

  // ── Editor placement ───────────────────────────────────────────────────────
  // The editor is placed SEPARATELY from the pin: its measured size is clamped
  // into the visible part of the scroll container (with an inset), its width and
  // height are capped to the room available (it scrolls inside), and it opens
  // above the pin only when it does not fit below.
  let editorEl = $state<HTMLElement | null>(null);
  let editorStyle = $state('');
  const EDITOR_W = 220;
  const EDITOR_GAP = 14;
  const EDITOR_INSET = 8;
  const EDITOR_MIN_H = 96;

  function placeEditor(): void {
    const wrap = scroller();
    if (!pending || !editorEl || !wrap) return;
    const minX = wrap.scrollLeft - geom.left + EDITOR_INSET;
    const maxX = wrap.scrollLeft + wrap.clientWidth - geom.left - EDITOR_INSET;
    const minY = wrap.scrollTop - geom.top + EDITOR_INSET;
    const maxY = wrap.scrollTop + wrap.clientHeight - geom.top - EDITOR_INSET;
    const pinX = pending.x_pct * geom.width;
    const pinY = pending.y_pct * geom.height;
    const width = Math.max(120, Math.min(EDITOR_W, maxX - minX));
    // Natural (uncapped) height: the content height, plus the 1px borders.
    const natural = editorEl.scrollHeight + 2;
    const below = maxY - (pinY + EDITOR_GAP);
    const above = pinY - EDITOR_GAP - minY;
    let top: number;
    let height: number;
    if (natural <= below) {
      height = natural;
      top = pinY + EDITOR_GAP;
    } else if (natural <= above) {
      height = natural;
      top = pinY - EDITOR_GAP - natural;
    } else if (below >= above) {
      height = Math.max(EDITOR_MIN_H, below);
      top = pinY + EDITOR_GAP;
    } else {
      height = Math.max(EDITOR_MIN_H, above);
      top = pinY - EDITOR_GAP - height;
    }
    height = Math.min(height, Math.max(EDITOR_MIN_H, maxY - minY));
    // Final clamp into the visible region.
    top = Math.max(minY, Math.min(top, maxY - height));
    const left = Math.max(minX, Math.min(pinX - width / 2, maxX - width));
    editorStyle = `left:${Math.round(left)}px; top:${Math.round(top)}px; width:${Math.round(width)}px; max-height:${Math.round(height)}px`;
  }

  $effect(() => {
    // Re-place when the pin moves, the box resizes or the note grows.
    void pending;
    void geom;
    void pendingBody;
    if (!pending) {
      editorStyle = '';
      return;
    }
    if (!editorEl) return;
    placeEditor();
  });
  $effect(() => {
    if (!pending) return;
    const wrap = scroller();
    if (!wrap) return;
    const again = (): void => placeEditor();
    wrap.addEventListener('scroll', again, { passive: true });
    window.addEventListener('resize', again);
    return () => {
      wrap.removeEventListener('scroll', again);
      window.removeEventListener('resize', again);
    };
  });

  async function toggleResolved(n: MockupAnnotation): Promise<void> {
    try {
      const updated = await product.patchAnnotation(n.id, { resolved: !n.resolved });
      notes = notes.map((x) => (x.id === n.id ? updated : x));
    } catch (e) {
      toastError('Couldn’t update note', e);
    }
  }

  async function removeNote(n: MockupAnnotation): Promise<void> {
    const ok = await confirmer.ask('Delete this annotation?', {
      title: 'Delete annotation',
      confirmLabel: 'Delete',
      danger: true,
    });
    if (!ok) return;
    try {
      await product.deleteAnnotation(n.id);
      notes = notes.filter((x) => x.id !== n.id);
    } catch (e) {
      toastError('Couldn’t delete note', e);
    }
  }

</script>

<!-- Overlay sits over the render box (matched to its measured geometry). In
     interact mode pointer-events:none lets input fall through to the iframe; in
     annotate mode it captures clicks. -->
<div
  class="overlay"
  class:annotate={mode === 'annotate'}
  role="presentation"
  style="left:{geom.left}px; top:{geom.top}px; width:{geom.width}px; height:{geom.height}px"
  onclick={onOverlayClick}
>
  {#each notes as n, i (n.id)}
    <div
      class="pin"
      class:resolved={n.resolved}
      style="left:{n.x_pct * 100}%; top:{n.y_pct * 100}%"
      title={n.body}
    >
      {i + 1}
    </div>
  {/each}

  {#if pending}
    <!-- Focusable so the pin can be placed with the arrow keys (⇧ = big step). -->
    <button
      type="button"
      class="pin pending"
      style="left:{pending.x_pct * 100}%; top:{pending.y_pct * 100}%"
      aria-label="New annotation position. Arrow keys move it, Shift for larger steps; Escape cancels."
      onkeydown={onPinKey}
      onclick={(e) => e.stopPropagation()}
    >
      {notes.length + 1}
    </button>
    <!-- Inline editor near the pin. Stop propagation so clicks inside it don't
         drop another pin. -->
    <div
      class="editor"
      class:unplaced={!editorStyle}
      bind:this={editorEl}
      style={editorStyle}
      role="dialog"
      tabindex="-1"
      aria-label="New annotation"
      onclick={(e) => e.stopPropagation()}
      onkeydown={(e) => {
        e.stopPropagation();
        if (e.key === 'Escape') {
          e.preventDefault();
          closeEditor();
        }
      }}
    >
      <textarea
        bind:value={pendingBody}
        aria-label="Annotation note"
        placeholder="e.g. Make this button the primary action"
        rows="3"
        use:focusOnMount
      ></textarea>
      <div class="editor-actions">
        <button class="mini ghost" onclick={closeEditor}>Cancel</button>
        <button class="mini primary" onclick={savePending} disabled={saving || !pendingBody.trim()}>
          {saving ? 'Saving…' : 'Add'}
        </button>
      </div>
    </div>
  {/if}
</div>

<!-- Controls + side list (rendered outside the render box; absolutely placed
     against the viewer so it doesn't disturb the render box's geometry). -->
<div class="side">
  <div class="side-head">
    <div class="mode-toggle" role="group" aria-label="Annotation mode">
      <button
        class="mt"
        class:active={mode === 'annotate'}
        aria-pressed={mode === 'annotate'}
        onclick={() => (mode = 'annotate')}
      >
        <Icon name="pin" size={12} /> Annotate
      </button>
      <button
        class="mt"
        class:active={mode === 'interact'}
        aria-pressed={mode === 'interact'}
        onclick={() => { mode = 'interact'; cancelPending(); }}
      >
        <Icon name="eye" size={12} /> Interact
      </button>
    </div>
    {#if mode === 'annotate'}
      <button
        class="mini add-note"
        bind:this={addBtn}
        onclick={startAtCentre}
        title="Drop a pin in the middle of the visible mockup, then move it with the arrow keys (⇧ for bigger steps)"
      >
        <Icon name="plus" size={12} /> Add annotation
      </button>
    {/if}
  </div>

  <div class="note-list">
    {#if loading}
      <Skeleton rows={3} height={28} label="annotations" />
    {:else if notes.length === 0}
      <div class="note-empty">
        No annotations yet.{mode === 'annotate' ? ' Click the mockup to drop a pin.' : ''}
      </div>
    {:else}
      {#each notes as n, i (n.id)}
        <div class="note" class:resolved={n.resolved}>
          <span class="note-num">{i + 1}</span>
          <span class="note-body">{n.body}</span>
          <div class="note-actions">
            <button
              class="note-btn"
              onclick={() => toggleResolved(n)}
              title={n.resolved ? 'Reopen' : 'Resolve'}
              aria-label={n.resolved ? 'Reopen' : 'Resolve'}
            >
              <Icon name="check" size={12} />
            </button>
            <button
              class="note-btn danger"
              onclick={() => removeNote(n)}
              title="Delete"
              aria-label="Delete"
            >
              <Icon name="trash" size={12} />
            </button>
          </div>
        </div>
      {/each}
    {/if}
  </div>
</div>

<style>
  .overlay {
    position: absolute;
    z-index: 5;
    /* Interact mode: clicks fall through to the iframe. */
    pointer-events: none;
  }
  .overlay.annotate {
    pointer-events: auto;
    cursor: crosshair;
  }

  .pin {
    position: absolute;
    transform: translate(-50%, -50%);
    width: 20px;
    height: 20px;
    border-radius: 999px;
    background: var(--accent-solid);
    color: var(--accent-contrast);
    font-size: var(--fs-xs);
    font-weight: 600;
    display: grid;
    place-items: center;
    box-shadow: var(--shadow-card);
    /* Pins are clickable for their tooltip even in interact mode. */
    pointer-events: auto;
    cursor: default;
  }
  .pin.resolved {
    background: var(--text-dim);
    opacity: 0.7;
  }
  .pin.pending {
    background: var(--status-warn);
    animation: otto-pulse 1.4s ease-in-out infinite;
  }

  .editor {
    position: absolute;
    z-index: 6;
    width: 220px;
    max-width: 100%;
    box-sizing: border-box;
    /* Capped to the room available (inline max-height): scroll inside. */
    overflow: auto;
    overscroll-behavior: contain;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    box-shadow: var(--glass-shadow);
    padding: 8px;
    pointer-events: auto;
  }
  /* Until placeEditor() has measured it: keep it laid out (so it can take focus). */
  .editor.unplaced {
    opacity: 0;
  }
  .pin.pending:focus-visible {
    outline: 2px solid var(--accent-text);
    outline-offset: 2px;
  }
  button.pin {
    border: 0;
    padding: 0;
    font: inherit;
    font-size: var(--fs-xs);
    font-weight: 600;
  }
  .editor textarea {
    width: 100%;
    box-sizing: border-box;
    resize: vertical;
    font-size: var(--fs-s);
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--bg);
    color: var(--text);
    padding: 6px;
  }
  .editor-actions {
    display: flex;
    justify-content: flex-end;
    gap: 6px;
    margin-top: 6px;
  }
  .mini {
    padding: 4px 10px;
    border-radius: var(--radius-s);
    font-size: var(--fs-xs);
    font-weight: 500;
    cursor: pointer;
    border: 1px solid var(--border);
    background: transparent;
    color: var(--text);
  }
  .mini.primary {
    background: var(--accent-solid);
    border-color: var(--accent-solid);
    color: var(--accent-contrast);
  }
  .mini.primary:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
  .mini.ghost:hover {
    background: color-mix(in srgb, var(--text-dim) 12%, transparent);
  }

  /* ── Side list — placed below the render box (in normal flow of the viewer). */
  .side {
    margin-top: 10px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    overflow: hidden;
  }
  .side-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 6px 8px;
    border-bottom: 1px solid var(--border);
  }
  .mode-toggle {
    display: flex;
    gap: 2px;
  }
  .add-note {
    display: inline-flex;
    align-items: center;
    gap: 4px;
  }
  .mt {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    padding: 4px 10px;
    border: none;
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--text-dim);
    font-size: var(--fs-xs);
    font-weight: 500;
    cursor: pointer;
  }
  .mt:hover {
    color: var(--text);
  }
  .mt.active {
    background: var(--accent-soft);
    color: var(--accent-text);
  }
  .note-list {
    max-height: 200px;
    overflow-y: auto;
  }
  .note-empty {
    padding: 10px;
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .note {
    display: flex;
    align-items: flex-start;
    gap: 8px;
    padding: 6px 8px;
    border-bottom: 1px solid color-mix(in srgb, var(--border) 60%, transparent);
  }
  .note:last-child {
    border-bottom: none;
  }
  .note.resolved .note-body {
    text-decoration: line-through;
    color: var(--text-dim);
  }
  .note-num {
    flex-shrink: 0;
    width: 18px;
    height: 18px;
    border-radius: 999px;
    background: var(--accent-solid);
    color: var(--accent-contrast);
    font-size: var(--fs-xs);
    font-weight: 600;
    display: grid;
    place-items: center;
  }
  .note.resolved .note-num {
    background: var(--text-dim);
  }
  .note-body {
    flex: 1;
    min-width: 0;
    font-size: var(--fs-s);
    line-height: 1.4;
    white-space: pre-wrap;
    word-break: break-word;
  }
  .note-actions {
    display: flex;
    gap: 2px;
    flex-shrink: 0;
  }
  .note-btn {
    display: grid;
    place-items: center;
    width: 22px;
    height: 22px;
    border: none;
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--text-dim);
    cursor: pointer;
  }
  .note-btn:hover {
    background: color-mix(in srgb, var(--text-dim) 14%, transparent);
    color: var(--text);
  }
  .note-btn.danger:hover {
    background: color-mix(in srgb, var(--danger) 15%, transparent);
    color: var(--danger);
  }
</style>
