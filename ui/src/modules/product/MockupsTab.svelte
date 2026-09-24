<script lang="ts">
  // MockupsTab — kept as a thin alias of the Design Arena so the `mockups` tab id
  // (deep links, E2E) keeps resolving; the tab's LABEL is "Design" (design §6).
  // Above the arena: the Design Hall graph of this story (designs that implement
  // it + what they use, "Open in Design Hall"). The arena itself is unchanged.
  import DesignArena from './design/DesignArena.svelte';
  import DesignGraphStrip from '../design-hall/DesignGraphStrip.svelte';
  import { product } from '../../lib/stores/product.svelte';

  const story = $derived(product.detail?.story ?? null);
  const isEpic = $derived(!!story && (story.tree_kind === 'epic' || product.childrenOf(story.id).length > 0));
</script>

{#if story}
  {#key story.id}
    <DesignGraphStrip storyId={story.id} storyKey={story.source_key} storyTitle={story.title} {isEpic} />
  {/key}
{/if}
<DesignArena />
