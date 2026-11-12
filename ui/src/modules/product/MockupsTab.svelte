<script lang="ts">
  // MockupsTab — the story's Design tab (the `mockups` tab id is kept so deep
  // links and E2E keep resolving; the tab's LABEL is "Design"). Design Hall is
  // the one home for designs, so the tab is the story's designs from the Hall
  // (StoryDesigns); the old DesignArena opens only from its ⋯ ("Open the
  // classic arena"). Undo: render `<DesignArena />` here again.
  import StoryDesigns from './StoryDesigns.svelte';
  import Skeleton from '../../lib/components/Skeleton.svelte';
  import { product } from '../../lib/stores/product.svelte';

  const story = $derived(product.detail?.story ?? null);
  const isEpic = $derived(!!story && (story.tree_kind === 'epic' || product.childrenOf(story.id).length > 0));
</script>

{#if story}
  {#key story.id}
    <StoryDesigns {story} {isEpic} />
  {/key}
{:else}
  <!-- The story's detail is still loading (the page shows its own load error). -->
  <Skeleton rows={2} height={180} />
{/if}
