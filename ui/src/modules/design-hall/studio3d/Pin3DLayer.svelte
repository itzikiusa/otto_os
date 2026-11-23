<script lang="ts">
  // Projected 3D pin markers + hovered polygon readout + docked 3D polygon
  // comment composer layered over the Three.js viewport.
  import { untrack } from 'svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import { designBus } from '../../../lib/events.svelte';
  import { toastError } from '../../../lib/toastError';
  import { pin3dLabel, type Pin3dAnchor, type ProjectedPin3d } from '../../../lib/design/scene3d';
  import type { CommentsStore } from '../comments/commentsStore.svelte';

  interface Props {
    store: CommentsStore;
    pins: ProjectedPin3d[];
    hovered: Pin3dAnchor | null;
    draftAnchor: Pin3dAnchor | null;
    pickMode: boolean;
    versionId: string | null;
    onselectpin: (id: string) => void;
    oncleardraft: () => void;
  }
  let {
    store,
    pins,
    hovered,
    draftAnchor,
    pickMode,
    versionId,
    onselectpin,
    oncleardraft,
  }: Props = $props();

  let text = $state('');
  let posting = $state(false);
  let composerEl = $state<HTMLTextAreaElement | null>(null);

  $effect(() => {
    const s = store;
    untrack(() => s.open(designBus, true));
  });
  $effect(() => {
    void designBus.seq;
    void designBus.resyncTick;
    untrack(() => store.follow(designBus));
  });

  $effect(() => {
    if (draftAnchor) queueMicrotask(() => composerEl?.focus());
  });

  async function submit(): Promise<void> {
    const body = text.trim();
    if (!body || posting || !draftAnchor) return;
    posting = true;
    try {
      const c = await store.create(body, { pin3d: draftAnchor, versionId });
      text = '';
      store.selectedId = c.id;
      oncleardraft();
    } catch (e) {
      toastError('Couldn’t post the polygon comment', e);
    } finally {
      posting = false;
    }
  }

  function onComposerKey(e: KeyboardEvent): void {
    if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) {
      e.preventDefault();
      void submit();
    } else if (e.key === 'Escape') {
      e.preventDefault();
      e.stopPropagation();
      text = '';
      oncleardraft();
    }
  }
</script>

<div class="p3d-layer" data-testid="s3d-pin-layer">
  {#each pins as p (p.id)}
    {#if p.visible}
      <button
        type="button"
        class="p3d-pin"
        class:sel={store.selectedId === p.id}
        class:muted={p.muted}
        class:back={p.backfacing}
        style:transform={`translate(${p.x.toFixed(1)}px, ${p.y.toFixed(1)}px) translate(-50%, -50%)`}
        aria-pressed={store.selectedId === p.id}
        aria-label={`3D comment ${p.n}: ${p.label}`}
        title={`${p.label} — ${p.body}`}
        onclick={() => onselectpin(p.id)}
        data-testid="s3d-pin-marker"
      >{p.n}</button>
    {/if}
  {/each}

  {#if pickMode && hovered && !draftAnchor}
    <div class="p3d-hover" data-testid="s3d-poly-readout">
      <Icon name="target" size={12} />
      <span>{pin3dLabel(hovered)}</span>
      {#if hovered.vertices}
        <span class="mono">v[{hovered.vertices.join(',')}]</span>
      {/if}
      <span class="mono">({hovered.position[0]}, {hovered.position[1]}, {hovered.position[2]})</span>
      <span class="dim">Click to comment · Shift+click adds faces</span>
    </div>
  {/if}

  {#if draftAnchor}
    <div
      class="p3d-composer"
      role="group"
      aria-label={`New comment on ${pin3dLabel(draftAnchor)}`}
      data-testid="s3d-poly-composer"
    >
      <div class="p3d-anchor">
        <Icon name="pin" size={12} />
        <strong>{pin3dLabel(draftAnchor)}</strong>
        {#if draftAnchor.vertices}
          <span class="mono">v[{draftAnchor.vertices.join(',')}]</span>
        {/if}
        <span class="mono">({draftAnchor.position[0]}, {draftAnchor.position[1]}, {draftAnchor.position[2]})</span>
      </div>
      <textarea
        dir="auto"
        bind:this={composerEl}
        bind:value={text}
        rows="2"
        maxlength="4000"
        placeholder="Describe what should change on this polygon or mesh…"
        aria-label="3D polygon comment"
        onkeydown={onComposerKey}
      ></textarea>
      <div class="p3d-row">
        <span class="dim">⌘↩ post · Shift+click another face to expand selection</span>
        <button
          type="button"
          class="btn small ghost"
          onclick={() => {
            text = '';
            oncleardraft();
          }}
        >Cancel</button>
        <button
          type="button"
          class="btn small primary"
          disabled={!text.trim() || posting}
          onclick={() => void submit()}
          data-testid="s3d-poly-post"
        >{posting ? 'Posting…' : 'Post'}</button>
      </div>
    </div>
  {/if}
</div>

<style>
  .p3d-layer {
    position: absolute;
    inset: 0;
    pointer-events: none;
    overflow: hidden;
    z-index: 3;
    direction: ltr;
  }
  .p3d-pin {
    position: absolute;
    inset-block-start: 0;
    inset-inline-start: 0;
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
  .p3d-pin.muted {
    background: var(--surface);
    color: var(--text-dim);
    border-color: var(--border);
  }
  .p3d-pin.back {
    opacity: 0.45;
  }
  .p3d-pin.sel {
    outline: 3px solid var(--accent-soft-strong);
    z-index: 2;
  }
  .p3d-pin:focus-visible {
    outline: 2px solid var(--accent-text);
    outline-offset: 2px;
  }
  .p3d-hover {
    position: absolute;
    inset-block-start: 48px;
    inset-inline-start: 12px;
    max-inline-size: calc(100% - 24px);
    display: inline-flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 8px;
    padding: 6px 10px;
    border-radius: var(--radius-m);
    background: var(--surface);
    border: 1px solid var(--accent-line);
    color: var(--text);
    font-size: var(--fs-xs);
    box-shadow: var(--shadow-card);
  }
  .p3d-composer {
    position: absolute;
    inset-inline: 12px;
    inset-block-end: 12px;
    max-inline-size: 460px;
    margin-inline: auto;
    pointer-events: auto;
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 10px 12px;
    border-radius: var(--radius-m);
    background: var(--surface);
    border: 1px solid var(--border-strong);
    box-shadow: var(--shadow-card);
    z-index: 4;
  }
  .p3d-anchor {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 6px;
    font-size: var(--fs-xs);
    color: var(--accent-text);
  }
  .p3d-composer textarea {
    inline-size: 100%;
    max-block-size: 28vh;
    resize: vertical;
    font: inherit;
    font-size: var(--fs-s);
    padding: 6px 8px;
    border: 1px solid var(--control-border);
    border-radius: var(--radius-s);
    background: var(--bg);
    color: var(--text);
  }
  .p3d-row {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .mono {
    font-family: var(--font-mono);
    color: var(--text-dim);
  }
  .dim {
    flex: 1;
    min-inline-size: 0;
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  @media (pointer: coarse) {
    .p3d-pin {
      min-inline-size: 40px;
      block-size: 40px;
    }
    .p3d-composer textarea {
      font-size: 16px; /* ui-guards: allow */
    }
    .p3d-row :global(.btn) {
      min-block-size: 40px;
    }
  }
</style>
