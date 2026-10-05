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
  import Badge from '../../lib/components/Badge.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import { loadErrorText } from '../../lib/loadError';
  import { badgeTone, storyStage } from '../../lib/status';
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
      // Supplementary: one compact line + Retry, never a page-level error.
      error = loadErrorText(e);
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

  const stage = $derived(story?.stage ? storyStage(story.stage) : null);
</script>

{#if !story && (loading || error) && project.story_id}
  <div class="slc-state">
    <LoadState what="the linked story" variant="compact" {loading} {error} empty rows={1} onretry={() => void load(project.story_id!)} />
  </div>
{:else if story}
  <div class="story-link-card">
    <Icon name="note" size={12} />
    <span class="slc-label dim">From story</span>
    <span class="slc-title" title={story.title}>{story.title}</span>
    {#if stage}
      <Badge tone={badgeTone(stage.tone)} label={stage.label} />
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
    font-size: var(--fs-s);
    flex-wrap: wrap;
  }
  .slc-label {
    font-size: var(--fs-xs);
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: .06em;
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
    margin-inline-start: auto;
    flex: none;
  }
  .slc-state {
    padding: 4px 10px;
  }
</style>
