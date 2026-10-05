<script lang="ts">
  // Modal to search and attach a product story to a running session.
  // Mirrors AttachIssue.svelte but calls POST /sessions/{id}/attach-product
  // which also injects the full refined context bundle into the live PTY.
  import type { ProductStory } from '../product/types';
  import { toastError } from '../../lib/toastError';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { api } from '../../lib/api/client';
  import { toasts } from '../../lib/toast.svelte';
  import Modal from '../../lib/components/Modal.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import Badge from '../../lib/components/Badge.svelte';
  import { loadErrorText } from '../../lib/loadError';
  import { sentenceCase } from '../../lib/labels';

  interface Props {
    sessionId: string;
    onclose: () => void;
  }
  let { sessionId, onclose }: Props = $props();

  let stories = $state<ProductStory[]>([]);
  let loading = $state(true);
  let query = $state('');
  let attaching = $state(false);

  // A failed load used to fall through to "No product stories in this
  // workspace." — say it failed, with Retry.
  let loadError = $state<string | null>(null);
  function load(wsId: string): void {
    loading = true;
    loadError = null;
    void api
      .get<ProductStory[]>(`/workspaces/${wsId}/product/stories`)
      .then((s) => { stories = s; loading = false; })
      .catch((e) => {
        loadError = loadErrorText(e);
        loading = false;
      });
  }

  $effect(() => {
    const wsId = ws.currentId;
    if (!wsId) { loading = false; return; }
    load(wsId);
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
      toastError('Couldn’t attach the story', e);
    } finally {
      attaching = false;
    }
  }
</script>

<Modal title="Attach product story" width={540} {onclose}>
  <div class="hint">
    Injects the full refined context — story, analysis, Q&amp;A, approved tests, and learnings —
    into the running agent session.
  </div>

  <input
    class="search-input"
    type="search"
    placeholder="Filter by title or key…"
    bind:value={query}
    data-autofocus
  />

  <LoadState
    what="product stories"
    {loading}
    error={loadError}
    empty={filtered.length === 0}
    onretry={() => ws.currentId && load(ws.currentId)}
    rows={3}
  >
    {#snippet emptyView()}
      <EmptyState
        icon={stories.length === 0 ? 'box' : 'search'}
        title={stories.length === 0 ? 'No product stories in this workspace' : 'No stories match your filter'}
        body={stories.length === 0 ? 'Refine a story in Product first, then attach it here.' : undefined}
      />
    {/snippet}
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
            <Badge label={sentenceCase(story.stage)} />
          </button>
        </li>
      {/each}
    </ul>
  </LoadState>

  {#snippet footer()}
    <button class="btn" onclick={onclose}>Cancel</button>
  {/snippet}
</Modal>

<style>
  .hint {
    font-size: var(--fs-s);
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
    font-size: var(--fs-m);
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
    gap: 2px;
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
    padding: 6px 10px;
    cursor: pointer;
    color: var(--text);
    font-size: var(--fs-m);
  }
  .story-row:hover:not(:disabled) {
    background: var(--accent-soft);
    border-color: color-mix(in srgb, var(--accent) 30%, transparent);
  }
  .story-row:disabled {
    opacity: 0.5;
    cursor: default;
  }
  .story-key {
    font-size: var(--fs-xs);
    font-weight: 600;
    color: var(--accent-text);
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
</style>
