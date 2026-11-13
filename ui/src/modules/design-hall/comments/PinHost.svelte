<script lang="ts">
  // The artifact view's stage host with review pins on top. It wraps the
  // stage (ArtifactStage) and lays a pin layer exactly over the RENDERED
  // content box — the fit frame, the device-frame iframe, the image, the
  // diagram or the board — measured on every resize / re-render, so pins
  // (stored as fractions of that box) stay on their spot when the pane
  // resizes or the device frame changes. The layer is click-through unless
  // Comment mode is on (C, or the Comment button): then a click / tap drops a
  // pin and a composer docks at the bottom of the stage. Open pins are drawn
  // numbered; one made on another version is muted with its version chip.
  import type { Snippet } from 'svelte';
  import { untrack } from 'svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import { designBus } from '../../../lib/events.svelte';
  import { toastError } from '../../../lib/toastError';
  import type { CommentsStore } from './commentsStore.svelte';
  import { isTypingTarget, nudge, pinAt, pinWords, pinsFor } from './pins';

  interface Props {
    store: CommentsStore;
    /** The version the stage shows (null: no version yet). */
    versionId: string | null;
    canComment: boolean;
    /** Show the Comments tab (a pin was selected). */
    onshow?: () => void;
    children: Snippet;
  }
  let { store, versionId, canComment, onshow, children }: Props = $props();

  let host = $state<HTMLDivElement | null>(null);
  /** The content box, in px relative to the host. */
  let box = $state<{ left: number; top: number; width: number; height: number } | null>(null);
  let text = $state('');
  let posting = $state(false);
  let composer = $state<HTMLTextAreaElement | null>(null);

  $effect(() => {
    const s = store;
    untrack(() => s.open(designBus, true));
  });
  $effect(() => {
    void designBus.seq;
    void designBus.resyncTick;
    untrack(() => store.follow(designBus));
  });

  /** The rendered content inside the stage (else the stage's view itself). */
  function surface(): HTMLElement | null {
    const view = host?.querySelector<HTMLElement>('[data-testid="design-stage"]');
    if (!view) return null;
    return view.querySelector<HTMLElement>('.fit, img.img, img.svg, iframe.doc, object.doc, .board') ?? view;
  }

  let frame = 0;
  function measure(): void {
    cancelAnimationFrame(frame);
    frame = requestAnimationFrame(() => {
      const h = host?.getBoundingClientRect();
      const el = surface();
      if (!h || !el) return void (box = null);
      // The WHOLE content box (pins are fractions of it, scrolled or not);
      // the host clips whatever lies outside the stage.
      const r = el.getBoundingClientRect();
      const next = r.width > 4 && r.height > 4 ? { left: r.left - h.left, top: r.top - h.top, width: r.width, height: r.height } : null;
      // Unchanged geometry keeps the same object (the observers fire often).
      if (next && box && next.left === box.left && next.top === box.top && next.width === box.width && next.height === box.height) return;
      box = next;
    });
  }

  $effect(() => {
    const el = host;
    if (!el) return;
    measure();
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    // A device switch / version view re-renders the content element.
    let watched: HTMLElement | null = null;
    const mo = new MutationObserver(() => {
      const s = surface();
      if (s !== watched) {
        if (watched) ro.unobserve(watched);
        watched = s;
        if (s) ro.observe(s);
      }
      measure();
    });
    mo.observe(el, { childList: true, subtree: true });
    el.addEventListener('scroll', measure, true);
    return () => {
      ro.disconnect();
      mo.disconnect();
      el.removeEventListener('scroll', measure, true);
      cancelAnimationFrame(frame);
    };
  });

  const drawn = $derived(pinsFor(store.threads, versionId));

  function toggleMode(): void {
    if (store.mode) store.exitMode();
    else if (canComment) store.mode = true;
  }

  /** Mouse / tap: the pointer's spot. Keyboard (Enter / Space): the middle. */
  function drop(e: MouseEvent): void {
    if (!box || !host) return;
    const h = host.getBoundingClientRect();
    const keyboard = e.detail === 0;
    const pin = keyboard ? { x: 0.5, y: 0.5 } : pinAt(e.clientX, e.clientY, { left: h.left + box.left, top: h.top + box.top, width: box.width, height: box.height });
    if (!pin) return;
    store.draft = pin;
    store.selectedId = null;
    // A keyboard drop keeps focus on the layer so the arrows can move the pin.
    if (!keyboard) queueMicrotask(() => composer?.focus());
  }

  function onLayerKey(e: KeyboardEvent): void {
    const d = store.draft;
    if (!d) return;
    const step: Record<string, [number, number]> = { ArrowLeft: [-1, 0], ArrowRight: [1, 0], ArrowUp: [0, -1], ArrowDown: [0, 1] };
    const s = step[e.key];
    if (!s) return;
    e.preventDefault();
    store.draft = nudge(d, s[0], s[1], e.shiftKey ? 0.05 : 0.01);
  }

  /** A pin always opens its thread: on a phone the Details sheet is closed
   *  until asked, and a comment just posted is already selected, so a toggle
   *  here would close what the tap was meant to open. The thread's own button
   *  clears the selection. */
  function select(id: string): void {
    store.selectedId = id;
    onshow?.();
  }

  async function post(): Promise<void> {
    const body = text.trim();
    if (!body || posting || !store.draft) return;
    posting = true;
    try {
      const c = await store.create(body, { pin: store.draft, versionId });
      text = '';
      store.draft = null;
      store.selectedId = c.id;
    } catch (e) {
      toastError('Couldn’t post the comment', e);
    } finally {
      posting = false;
    }
  }

  function cancel(): void {
    store.draft = null;
    text = '';
  }

  function onComposerKey(e: KeyboardEvent): void {
    if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) {
      e.preventDefault();
      void post();
    } else if (e.key === 'Escape') {
      e.preventDefault();
      e.stopPropagation();
      cancel();
    }
  }

  // C toggles Comment mode while the design is open; Escape leaves it. Typing
  // anywhere (source editor, a field, a board) keeps its letters.
  $effect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.defaultPrevented || e.metaKey || e.ctrlKey || e.altKey || isTypingTarget(e.target)) return;
      if (document.querySelector('[role="dialog"][aria-modal="true"]')) return;
      if ((e.key === 'c' || e.key === 'C') && canComment && !e.shiftKey) {
        e.preventDefault();
        toggleMode();
      } else if (e.key === 'Escape' && store.mode) {
        store.exitMode();
      }
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  });
</script>

<div class="stage-host pin-host" bind:this={host}>
  {@render children()}
  {#if box}
    <div class="layer" class:capture={store.mode} data-testid="design-pin-layer"
      style:inset-inline-start="{box.left}px" style:inset-block-start="{box.top}px" style:inline-size="{box.width}px" style:block-size="{box.height}px">
      {#if store.mode}
        <button class="drop" onclick={drop} onkeydown={onLayerKey} data-testid="design-pin-drop"
          aria-label="Drop a comment pin: click a spot on the design, or press Enter for the middle and move it with the arrow keys"
          title="Click a spot to pin a comment"></button>
      {/if}
      {#each drawn as d (d.thread.root.id)}
        {@const p = d.thread.root.pin!}
        <button class="pin" class:muted={d.muted} class:sel={store.selectedId === d.thread.root.id}
          style:inset-inline-start="{p.x * 100}%" style:inset-block-start="{p.y * 100}%"
          onclick={() => select(d.thread.root.id)} data-testid="design-pin"
          aria-pressed={store.selectedId === d.thread.root.id}
          aria-label="Comment {d.thread.n} by {d.thread.root.author_name}{d.muted && d.thread.root.version_seq != null ? `, made on v${d.thread.root.version_seq}` : ''}, {pinWords(p)}"
          title={d.thread.root.body}>{d.thread.n}</button>
      {/each}
      {#if store.draft}
        <span class="pin draft" aria-hidden="true" style:inset-inline-start="{store.draft.x * 100}%" style:inset-block-start="{store.draft.y * 100}%"><Icon name="plus" size={12} /></span>
      {/if}
    </div>
  {/if}
  {#if canComment || store.mode}
    <button class="btn small mode-btn" class:on={store.mode} aria-pressed={store.mode} onclick={toggleMode} data-testid="design-comment-mode"
      title={store.mode ? 'Leave comment mode (C or Esc)' : 'Comment on the design (C)'}>
      <Icon name="comment" size={12} /> {store.mode ? 'Done' : 'Comment'}
    </button>
  {/if}
  {#if store.draft}
    <div class="composer" role="group" aria-label="New comment at {pinWords(store.draft)}" data-testid="design-pin-composer">
      <textarea dir="auto" bind:this={composer} bind:value={text} rows="2" maxlength="4000" placeholder="Say what should change here…"
        aria-label="Comment text" onkeydown={onComposerKey}></textarea>
      <div class="ph-row">
        <span class="hint">⌘↩ to post</span>
        <button class="btn small ghost" onclick={cancel}>Cancel</button>
        <button class="btn small primary" onclick={() => void post()} disabled={!text.trim() || posting}>{posting ? 'Posting…' : 'Post'}</button>
      </div>
    </div>
  {/if}
</div>

<style>
  .pin-host {
    position: relative;
    overflow: hidden;
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
    container-type: inline-size;
  }
  .pin-host > :global(.stage) {
    flex: 1;
  }
  /* The design is never mirrored: pin fractions are of its physical box. */
  .layer {
    position: absolute;
    direction: ltr;
    pointer-events: none;
    z-index: 2;
  }
  .layer.capture {
    pointer-events: auto;
  }
  .drop {
    position: absolute;
    inset: 0;
    inline-size: 100%;
    block-size: 100%;
    padding: 0;
    border: 1px dashed var(--accent-solid);
    background: color-mix(in srgb, var(--accent-solid) 6%, transparent);
    cursor: crosshair;
  }
  .drop:focus-visible {
    outline: 2px solid var(--accent-text);
    outline-offset: -2px;
  }
  .pin {
    position: absolute;
    transform: translate(-50%, -50%);
    pointer-events: auto;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    min-inline-size: 24px;
    block-size: 24px;
    padding: 0 6px;
    border-radius: 999px;
    border: 2px solid var(--surface);
    background: var(--accent-solid);
    color: var(--accent-contrast);
    font-size: var(--fs-xs);
    font-weight: 600;
    box-shadow: var(--shadow-card);
    cursor: pointer;
  }
  .pin.muted {
    background: var(--surface);
    color: var(--text-dim);
    border-color: var(--border);
  }
  .pin.sel {
    outline: 3px solid var(--accent-soft);
    z-index: 1;
  }
  .pin:focus-visible {
    outline: 2px solid var(--accent-text);
    outline-offset: 2px;
  }
  .pin.draft {
    background: var(--warning);
    color: var(--surface);
  }
  .mode-btn {
    position: absolute;
    inset-block-start: 8px;
    inset-inline-end: 8px;
    z-index: 3;
    background: var(--surface);
    box-shadow: var(--shadow-card);
  }
  .mode-btn.on {
    background: var(--accent-soft);
    color: var(--accent-text);
  }
  .composer {
    position: absolute;
    inset-inline: 12px;
    inset-block-end: 12px;
    z-index: 3;
    max-inline-size: 420px;
    margin-inline: auto;
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 10px;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    box-shadow: var(--shadow-card);
  }
  .composer textarea {
    inline-size: 100%;
    max-block-size: 30vh;
    resize: vertical;
    font: inherit;
    font-size: var(--fs-s);
  }
  .ph-row {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .hint {
    flex: 1;
    min-inline-size: 0;
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  @media (prefers-reduced-motion: no-preference) {
    .pin {
      transition: transform var(--dur-fast) ease;
    }
    .pin.sel {
      transform: translate(-50%, -50%) scale(1.15);
    }
  }
</style>
