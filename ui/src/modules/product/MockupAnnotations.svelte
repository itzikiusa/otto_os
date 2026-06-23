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
  import { product } from '../../lib/stores/product.svelte';
  import { toasts } from '../../lib/toast.svelte';
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
      toasts.error('Could not load annotations', e instanceof Error ? e.message : String(e));
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
    } catch (e) {
      toasts.error('Could not add note', e instanceof Error ? e.message : String(e));
    } finally {
      saving = false;
    }
  }

  function cancelPending(): void {
    pending = null;
    pendingBody = '';
  }

  async function toggleResolved(n: MockupAnnotation): Promise<void> {
    try {
      const updated = await product.patchAnnotation(n.id, { resolved: !n.resolved });
      notes = notes.map((x) => (x.id === n.id ? updated : x));
    } catch (e) {
      toasts.error('Could not update note', e instanceof Error ? e.message : String(e));
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
      toasts.error('Could not delete note', e instanceof Error ? e.message : String(e));
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
    <div
      class="pin pending"
      style="left:{pending.x_pct * 100}%; top:{pending.y_pct * 100}%"
    >
      {notes.length + 1}
    </div>
    <!-- Inline editor near the pin. Stop propagation so clicks inside it don't
         drop another pin. -->
    <div
      class="editor"
      style="left:{pending.x_pct * 100}%; top:{pending.y_pct * 100}%"
      role="dialog"
      tabindex="-1"
      aria-label="New annotation"
      onclick={(e) => e.stopPropagation()}
      onkeydown={(e) => e.stopPropagation()}
    >
      <textarea
        bind:value={pendingBody}
        placeholder="Add a note…"
        rows="3"
        use:focusOnMount
      ></textarea>
      <div class="editor-actions">
        <button class="mini ghost" onclick={cancelPending}>Cancel</button>
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
    <div class="mode-toggle" role="tablist" aria-label="Annotation mode">
      <button
        class="mt"
        class:active={mode === 'annotate'}
        role="tab"
        aria-selected={mode === 'annotate'}
        onclick={() => (mode = 'annotate')}
      >
        <Icon name="pin" size={11} /> Annotate
      </button>
      <button
        class="mt"
        class:active={mode === 'interact'}
        role="tab"
        aria-selected={mode === 'interact'}
        onclick={() => { mode = 'interact'; cancelPending(); }}
      >
        <Icon name="eye" size={11} /> Interact
      </button>
    </div>
  </div>

  <div class="note-list">
    {#if loading}
      <div class="note-empty">Loading…</div>
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
    background: var(--accent);
    color: #fff;
    font-size: 11px;
    font-weight: 700;
    display: grid;
    place-items: center;
    box-shadow: 0 1px 4px rgba(0, 0, 0, 0.4);
    /* Pins are clickable for their tooltip even in interact mode. */
    pointer-events: auto;
    cursor: default;
  }
  .pin.resolved {
    background: var(--text-dim);
    opacity: 0.7;
  }
  .pin.pending {
    background: #f59e0b;
    animation: pulse 1.2s ease-in-out infinite;
  }
  @keyframes pulse {
    0%, 100% { box-shadow: 0 0 0 0 color-mix(in srgb, #f59e0b 50%, transparent); }
    50% { box-shadow: 0 0 0 6px transparent; }
  }

  .editor {
    position: absolute;
    transform: translate(-50%, 14px);
    z-index: 6;
    width: 220px;
    background: var(--surface, var(--bg));
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    box-shadow: 0 6px 20px rgba(0, 0, 0, 0.25);
    padding: 8px;
    pointer-events: auto;
  }
  .editor textarea {
    width: 100%;
    box-sizing: border-box;
    resize: vertical;
    font-size: 12px;
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
    font-size: 11.5px;
    font-weight: 500;
    cursor: pointer;
    border: 1px solid var(--border);
    background: transparent;
    color: var(--text);
  }
  .mini.primary {
    background: var(--accent);
    border-color: var(--accent);
    color: #fff;
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
  .mt {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    padding: 4px 10px;
    border: none;
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--text-dim);
    font-size: 11.5px;
    font-weight: 500;
    cursor: pointer;
  }
  .mt:hover {
    color: var(--text);
  }
  .mt.active {
    background: color-mix(in srgb, var(--accent) 15%, transparent);
    color: var(--accent);
  }
  .note-list {
    max-height: 200px;
    overflow-y: auto;
  }
  .note-empty {
    padding: 10px;
    font-size: 11.5px;
    color: var(--text-dim);
  }
  .note {
    display: flex;
    align-items: flex-start;
    gap: 8px;
    padding: 7px 8px;
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
    background: var(--accent);
    color: #fff;
    font-size: 10.5px;
    font-weight: 700;
    display: grid;
    place-items: center;
  }
  .note.resolved .note-num {
    background: var(--text-dim);
  }
  .note-body {
    flex: 1;
    min-width: 0;
    font-size: 12px;
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
    background: color-mix(in srgb, #ef4444 15%, transparent);
    color: #ef4444;
  }
</style>
