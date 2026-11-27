<script lang="ts">
  // The Vault's Tags view — a ListPane (design.md §3): every tag with its
  // count; a row runs a tag search (the Search view opens with it).
  import type { Snippet } from 'svelte';
  import ListPane from '../../lib/components/side/ListPane.svelte';
  import SideRow from '../../lib/components/side/SideRow.svelte';
  import type { IconName } from '../../lib/components/Icon.svelte';
  import { vault } from './vault.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';

  interface Props {
    views: Snippet;
    primary: { icon: IconName; label: string; onclick: () => void };
  }
  let { views, primary }: Props = $props();

  let filter = $state('');
  const shown = $derived(
    vault.tags.filter((t) => !filter || t.tag.toLowerCase().includes(filter.toLowerCase())),
  );
</script>

<ListPane {views} {primary} count={vault.tags.length || null} testid="vault-tags">
  {#snippet searchRow()}
    <input dir="ltr" class="input" type="search" bind:value={filter} placeholder="Filter tags…" aria-label="Filter tags" />
  {/snippet}
  <LoadState
    what="tags"
    loading={vault.tagsLoading}
    error={vault.tagsError}
    empty={vault.tags.length === 0}
    onretry={() => void vault.loadTags()}
    variant="compact"
  >
    {#snippet emptyView()}
      <div class="dim">No tags yet. Add #tags in a note, or tags: in its frontmatter.</div>
    {/snippet}
    {#if shown.length === 0}
      <div class="dim">No tags match “{filter}”.</div>
    {/if}
    {#each shown as t (t.tag)}
      <SideRow icon="tag" title="#{t.tag}" testid="vault-tag-row" onclick={() => vault.searchTag(t.tag)}>
        {#snippet trailing()}{t.count}{/snippet}
      </SideRow>
    {/each}
  </LoadState>
</ListPane>

<style>
  .dim {
    color: var(--text-dim);
    font-size: var(--fs-s);
    padding-block: 4px;
    padding-inline: 8px;
  }
</style>
