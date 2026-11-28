<script lang="ts" module>
  export type DetailsTab = 'design' | 'otto' | 'versions' | 'links' | 'comments';
</script>

<script lang="ts">
  // The artifact view's Detail pane (design.md §3): Otto · Versions · Links ·
  // Comments as its views (Site Studio puts its Design inspector first). Otto
  // is the default (the Otto panel pins the brief on top); "Find references"
  // lives inside Links (LinksTab). Beside the stage while it keeps 480 px,
  // else a right sheet (full width on a phone) — the host decides open.
  import type { Snippet } from 'svelte';
  import DetailPane from '../../lib/components/side/DetailPane.svelte';
  import Tabs, { type TabItem } from '../../lib/components/Tabs.svelte';

  interface Props {
    tab: DetailsTab;
    open: boolean;
    onclose: () => void;
    ontoggle: () => void;
    /** Bound: presented as a sheet right now. */
    sheet?: boolean;
    linkCount: number;
    versionCount: number | null;
    otto: Snippet;
    versions: Snippet;
    links: Snippet;
    /** Review comments (the tab shows when given); `commentCount` = open threads. */
    comments?: Snippet;
    commentCount?: number | null;
    /** Site Studio's inspector, as a first "Design" tab. */
    design?: Snippet;
  }
  let {
    tab = $bindable(),
    open,
    onclose,
    ontoggle,
    sheet = $bindable(false),
    linkCount,
    versionCount,
    otto,
    versions,
    links,
    comments,
    commentCount = null,
    design,
  }: Props = $props();

  const tabs = $derived<TabItem<DetailsTab>[]>([
    ...(design ? [{ id: 'design' as const, label: 'Design', testid: 'design-tab-design' }] : []),
    { id: 'otto', label: 'Otto', testid: 'design-tab-otto' },
    { id: 'versions', label: 'Versions', count: versionCount ?? undefined, testid: 'design-tab-versions' },
    { id: 'links', label: 'Links', count: linkCount || undefined, testid: 'design-tab-links' },
    ...(comments ? [{ id: 'comments' as const, label: 'Comments', count: commentCount || undefined, testid: 'design-tab-comments' }] : []),
  ]);
</script>

<DetailPane module="design" pane="details" title="Design details" {open} {onclose} {ontoggle} bind:sheet testid="design-details">
  {#snippet views()}
    <Tabs size="s" label="Details panel" idBase="dh" {tabs} value={tab} onchange={(id) => (tab = id)} />
  {/snippet}
  <div class="panel" role="tabpanel" id="dh-panel-{tab}" aria-labelledby="dh-tab-{tab}">
    {#if tab === 'design' && design}
      {@render design()}
    {:else if tab === 'otto'}
      {@render otto()}
    {:else if tab === 'versions'}
      {@render versions()}
    {:else if tab === 'comments' && comments}
      {@render comments()}
    {:else}
      {@render links()}
    {/if}
  </div>
</DetailPane>

<style>
  .panel {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
  }
</style>
