<script lang="ts">
  // Canvases linked to this story (canvas scenes whose story_id == this story).
  // Lists them with a click-to-open deep-link into the Canvas module, and a
  // "New canvas" action that creates one already linked to the story.
  import Icon from '../../lib/components/Icon.svelte';
  import { api } from '../../lib/api/client';
  import { canvas } from '../../lib/stores/canvas.svelte';
  import { router } from '../../lib/router.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import type { CanvasSceneSummary } from '../canvas/types';

  interface Props {
    storyId: string;
  }
  let { storyId }: Props = $props();

  let scenes = $state<CanvasSceneSummary[]>([]);
  let loading = $state(false);
  let creating = $state(false);
  let picking = $state(false);
  let linkingId = $state('');

  async function load(): Promise<void> {
    loading = true;
    try {
      scenes = await api.get<CanvasSceneSummary[]>(`/product/stories/${storyId}/linked-canvases`);
    } catch {
      scenes = [];
    } finally {
      loading = false;
    }
  }

  $effect(() => {
    if (storyId) void load();
  });

  // Existing canvases that aren't already linked to this story (the picker).
  const candidates = $derived(canvas.scenes.filter((c) => !scenes.some((l) => l.id === c.id)));

  async function openPicker(): Promise<void> {
    picking = !picking;
    if (picking && !canvas.scenes.length) await canvas.loadScenes().catch(() => {});
  }

  async function link(sceneId: string): Promise<void> {
    if (linkingId) return;
    linkingId = sceneId;
    try {
      await canvas.updateMeta(sceneId, { story_id: storyId });
      picking = false;
      await load();
      toasts.success('Canvas linked', 'Attached to this story');
    } catch (e) {
      toasts.error('Link failed', e instanceof Error ? e.message : String(e));
    } finally {
      linkingId = '';
    }
  }

  function open(id: string): void {
    canvas.pendingOpenId = id;
    router.go('canvas');
  }

  function excaliDoc(): unknown {
    return {
      type: 'otto-canvas',
      version: 1,
      format: 'excalidraw',
      source: JSON.stringify({ type: 'excalidraw', version: 2, source: 'otto', elements: [] }),
    };
  }

  async function createLinked(): Promise<void> {
    if (creating) return;
    creating = true;
    try {
      const created = await canvas.create('Untitled canvas', excaliDoc(), storyId);
      canvas.pendingOpenId = created.id;
      router.go('canvas');
      toasts.success('Canvas created', 'Linked to this story');
    } catch (e) {
      toasts.error('Could not create canvas', e instanceof Error ? e.message : String(e));
    } finally {
      creating = false;
    }
  }

  function label(s: CanvasSceneSummary): string {
    return s.section ? `${s.section.replace(/\//g, ' / ')} · ${s.title}` : s.title;
  }
</script>

<div class="linked-canvases">
  <div class="lc-head">
    <span class="lc-title"><Icon name="shapes" size={14} /> Canvases</span>
    <div class="lc-actions">
      <button class="lc-btn" class:on={picking} onclick={openPicker}>
        <Icon name="plug" size={12} /> Link existing
      </button>
      <button class="lc-btn primary" onclick={createLinked} disabled={creating}>
        <Icon name="plus" size={12} /> {creating ? 'Creating…' : 'New'}
      </button>
    </div>
  </div>

  {#if picking}
    <div class="lc-picker">
      {#if !candidates.length}
        <p class="lc-empty">No other canvases to link.</p>
      {:else}
        {#each candidates as c (c.id)}
          <button class="lc-cand" onclick={() => link(c.id)} disabled={!!linkingId}>
            <Icon name="shapes" size={12} />
            <span class="lc-name">{label(c)}</span>
            {#if linkingId === c.id}<span class="lc-busy">…</span>{:else}<Icon name="plus" size={12} />{/if}
          </button>
        {/each}
      {/if}
    </div>
  {/if}
  {#if loading && !scenes.length}
    <p class="lc-empty">Loading…</p>
  {:else if !scenes.length}
    <p class="lc-empty">No canvases linked yet. Create one to design this story visually.</p>
  {:else}
    <ul class="lc-list">
      {#each scenes as s (s.id)}
        <li>
          <button class="lc-row" onclick={() => open(s.id)} title="Open in Canvas">
            <Icon name="shapes" size={13} />
            <span class="lc-name">{label(s)}</span>
            <Icon name="chevronRight" size={13} />
          </button>
        </li>
      {/each}
    </ul>
  {/if}
</div>

<style>
  .linked-canvases {
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface);
    padding: 10px 12px;
  }
  .lc-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-bottom: 6px;
  }
  .lc-title {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-size: 12px;
    font-weight: 700;
    color: var(--text-dim, #888);
    text-transform: uppercase;
    letter-spacing: 0.02em;
  }
  .lc-actions {
    display: flex;
    gap: 6px;
  }
  .lc-btn {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    border: 1px solid var(--border);
    background: var(--bg);
    color: var(--text);
    border-radius: 7px;
    padding: 3px 9px;
    font-size: 12px;
    font-weight: 600;
    cursor: pointer;
  }
  .lc-btn:hover,
  .lc-btn.on {
    border-color: var(--accent);
    color: var(--accent);
  }
  .lc-btn.primary {
    background: var(--accent);
    color: #fff;
    border-color: var(--accent);
  }
  .lc-btn.primary:hover {
    color: #fff;
    filter: brightness(1.08);
  }
  .lc-picker {
    display: flex;
    flex-direction: column;
    gap: 2px;
    margin-bottom: 8px;
    max-height: 180px;
    overflow-y: auto;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    padding: 4px;
    background: var(--bg);
  }
  .lc-cand {
    width: 100%;
    display: flex;
    align-items: center;
    gap: 7px;
    padding: 5px 7px;
    border: none;
    background: none;
    color: var(--text);
    border-radius: var(--radius-s);
    cursor: pointer;
    text-align: start;
    font-size: 12.5px;
  }
  .lc-cand:hover {
    background: var(--surface-2);
  }
  .lc-busy {
    color: var(--text-dim, #888);
  }
  .lc-empty {
    margin: 4px 0 2px;
    font-size: 12px;
    color: var(--text-dim, #888);
  }
  .lc-list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .lc-row {
    width: 100%;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 8px;
    border: none;
    background: none;
    color: var(--text);
    border-radius: var(--radius-s);
    cursor: pointer;
    text-align: start;
  }
  .lc-row:hover {
    background: var(--surface-2);
  }
  .lc-name {
    flex: 1 1 auto;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: 13px;
  }
</style>
