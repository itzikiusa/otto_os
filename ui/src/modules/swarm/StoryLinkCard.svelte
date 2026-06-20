<script lang="ts">
  // Cross-link card: shows the Product story that originated a swarm project
  // (via Plan → Swarm). Rendered in the Kanban toolbar when the selected
  // project carries a story_id. Calls GET /swarm/tasks/{tid}/story using the
  // first task of the project to resolve the back-link, or falls back to
  // GET /product/stories/{sid} when the project.story_id is already known
  // directly (avoids needing a task).
  //
  // The card is intentionally minimal — title + stage badge + a "View story"
  // link that navigates to the Product section. When no link exists (project
  // was not seeded from a story) the card renders nothing.
  import Icon from '../../lib/components/Icon.svelte';
  import { api } from '../../lib/api/client';
  import { product } from '../../lib/stores/product.svelte';
  import { router } from '../../lib/router.svelte';
  import type { ProductStory } from '../product/types';
  import type { SwarmProject } from './types';

  interface Props {
    project: SwarmProject;
  }
  let { project }: Props = $props();

  // ── Fetch ─────────────────────────────────────────────────────────────────
  let story = $state<ProductStory | null>(null);
  let loading = $state(false);
  let error = $state<string | null>(null);

  $effect(() => {
    const sid = project.story_id ?? null;
    if (!sid) {
      story = null;
      return;
    }
    load(sid);
  });

  async function load(sid: string): Promise<void> {
    loading = true;
    error = null;
    try {
      // GET /product/stories/{sid} returns ProductStoryDetail; we only need
      // the `story` field from it.
      const detail = await api.get<{ story: ProductStory }>(`/product/stories/${sid}`);
      story = detail.story ?? null;
    } catch (e) {
      // Silently swallow — the card is supplementary.
      error = e instanceof Error ? e.message : String(e);
    } finally {
      loading = false;
    }
  }

  // ── Navigation ─────────────────────────────────────────────────────────────
  async function viewStory(): Promise<void> {
    if (!story) return;
    product.selectedId = story.id;
    router.go('product');
  }

  const STAGE_CLASS: Record<string, string> = {
    draft: 'dim',
    analysis: 'accent',
    planning: 'accent',
    ready: 'good',
    done: 'done',
  };
</script>

{#if loading}
  <!-- tiny inline spinner — don't block toolbar -->
  <span class="slc-loading dim">Loading…</span>
{:else if story}
  <div class="story-link-card">
    <Icon name="note" size={12} />
    <span class="slc-label dim">From story</span>
    <span class="slc-title" title={story.title}>{story.title}</span>
    {#if story.stage}
      <span class="chip {STAGE_CLASS[story.stage] ?? 'dim'}">{story.stage}</span>
    {/if}
    <button class="btn small ghost slc-view" onclick={viewStory} title="Open this story in Product">
      View story
    </button>
  </div>
{/if}

<style>
  .story-link-card {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 4px 10px;
    border-bottom: 1px solid var(--border);
    background: color-mix(in srgb, var(--accent) 5%, var(--surface));
    font-size: 12px;
    flex-wrap: wrap;
  }
  .slc-label {
    font-size: 10.5px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.05em;
  }
  .slc-title {
    font-weight: 500;
    color: var(--text);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    max-width: 260px;
  }
  .slc-view {
    margin-left: auto;
    flex: none;
  }
  .slc-loading {
    font-size: 11px;
    padding: 4px 10px;
  }
  .chip.good {
    background: color-mix(in srgb, var(--status-done, green) 15%, transparent);
    color: var(--status-done, green);
  }
  .chip.accent {
    background: color-mix(in srgb, var(--accent) 15%, transparent);
    color: var(--accent);
  }
  .chip.done {
    background: color-mix(in srgb, var(--status-done, green) 10%, transparent);
    color: var(--text-dim);
  }
</style>
