<script lang="ts">
  // Modal to search and attach a product story to a running session.
  // Mirrors AttachIssue.svelte but calls POST /sessions/{id}/attach-product
  // which also injects the full refined context bundle into the live PTY.
  import type { ProductStory } from '../product/types';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { api } from '../../lib/api/client';
  import { toasts } from '../../lib/toast.svelte';
  import Modal from '../../lib/components/Modal.svelte';

  interface Props {
    sessionId: string;
    onclose: () => void;
  }
  let { sessionId, onclose }: Props = $props();

  let stories = $state<ProductStory[]>([]);
  let loading = $state(true);
  let query = $state('');
  let attaching = $state(false);

  $effect(() => {
    const wsId = ws.currentId;
    if (!wsId) { loading = false; return; }
    void api
      .get<ProductStory[]>(`/workspaces/${wsId}/product/stories`)
      .then((s) => { stories = s; loading = false; })
      .catch(() => { loading = false; });
  });

  const filtered = $derived(
    query.trim() === ''
      ? stories
      : stories.filter(
          (s) =>
            s.title.toLowerCase().includes(query.toLowerCase()) ||
            s.source_key.toLowerCase().includes(query.toLowerCase()),
        ),
  );

  async function attach(story: ProductStory): Promise<void> {
    attaching = true;
    try {
      await ws.attachProductStory(sessionId, story.id);
      toasts.success('Context attached', story.title);
      onclose();
    } catch (e) {
      toasts.error('Attach failed', e instanceof Error ? e.message : String(e));
    } finally {
      attaching = false;
    }
  }
</script>

<Modal title="Attach Product Story" width={540} {onclose}>
  <div class="hint">
    Injects the full refined context — story, analysis, Q&amp;A, approved tests, and learnings —
    into the running agent session.
  </div>

  <!-- svelte-ignore a11y_autofocus -->
  <input
    class="search-input"
    type="search"
    placeholder="Filter by title or key…"
    bind:value={query}
    autofocus
  />

  {#if loading}
    <p class="dim">Loading stories…</p>
  {:else if filtered.length === 0}
    <p class="dim">{stories.length === 0 ? 'No product stories in this workspace.' : 'No stories match your filter.'}</p>
  {:else}
    <ul class="story-list">
      {#each filtered as story (story.id)}
        <li>
          <button
            class="story-row"
            disabled={attaching}
            onclick={() => { void attach(story); }}
          >
            <span class="story-key">{story.source_key}</span>
            <span class="story-title">{story.title}</span>
            <span class="story-stage chip">{story.stage}</span>
          </button>
        </li>
      {/each}
    </ul>
  {/if}

  {#snippet footer()}
    <button class="btn" onclick={onclose}>Cancel</button>
  {/snippet}
</Modal>

<style>
  .hint {
    font-size: 12px;
    color: var(--text-dim);
    margin-bottom: 10px;
    line-height: 1.5;
  }
  .search-input {
    width: 100%;
    box-sizing: border-box;
    padding: 6px 10px;
    background: var(--surface-2);
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    color: var(--text);
    font-size: 13px;
    margin-bottom: 10px;
    outline: none;
  }
  .search-input:focus {
    border-color: color-mix(in srgb, var(--accent) 55%, transparent);
  }
  .story-list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 3px;
    max-height: 340px;
    overflow-y: auto;
  }
  .story-row {
    width: 100%;
    display: flex;
    align-items: center;
    gap: 8px;
    text-align: start;
    background: transparent;
    border: 1px solid transparent;
    border-radius: var(--radius-m);
    padding: 7px 10px;
    cursor: pointer;
    color: var(--text);
    font-size: 13px;
  }
  .story-row:hover:not(:disabled) {
    background: color-mix(in srgb, var(--accent) 10%, transparent);
    border-color: color-mix(in srgb, var(--accent) 30%, transparent);
  }
  .story-row:disabled {
    opacity: 0.5;
    cursor: default;
  }
  .story-key {
    font-size: 11px;
    font-weight: 600;
    color: var(--accent);
    white-space: nowrap;
    flex-shrink: 0;
  }
  .story-title {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .story-stage {
    flex-shrink: 0;
    font-size: 9.5px;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    height: 16px;
  }
  .dim {
    color: var(--text-dim);
    font-size: 12.5px;
    text-align: center;
    padding: 20px 0;
  }
</style>
