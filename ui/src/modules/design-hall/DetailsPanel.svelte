<script lang="ts" module>
  export type DetailsTab = 'design' | 'otto' | 'versions' | 'links';
</script>

<script lang="ts">
  // The artifact view's side panel: Otto · Versions · Links (Site Studio puts
  // its Design inspector first). Otto is the default (the Otto panel pins the
  // brief on top); "Find references" lives inside Links (LinksTab). Below 900 px the view
  // hides this panel behind its "Details" button — "Back to design" returns.
  import type { Snippet } from 'svelte';
  import { onTabKey } from './tabKeys';

  interface Props {
    tab: DetailsTab;
    linkCount: number;
    versionCount: number | null;
    otto: Snippet;
    versions: Snippet;
    links: Snippet;
    /** Site Studio's inspector, as a first "Design" tab. */
    design?: Snippet;
    backButton?: HTMLButtonElement;
    /** The compact layout's "Back to design" (none where the studio switches panels itself). */
    onback?: () => void;
  }
  let {
    tab = $bindable(),
    linkCount,
    versionCount,
    otto,
    versions,
    links,
    design,
    backButton = $bindable(),
    onback,
  }: Props = $props();

  const tabs = $derived<{ id: DetailsTab; label: string; count?: number | null }[]>([
    ...(design ? [{ id: 'design' as const, label: 'Design' }] : []),
    { id: 'otto', label: 'Otto' },
    { id: 'versions', label: 'Versions', count: versionCount },
    { id: 'links', label: 'Links', count: linkCount },
  ]);
</script>

<aside class="right" aria-label="Design details">
  {#if onback}<button class="btn small ghost compact-details back-design" bind:this={backButton} onclick={onback}>Back to design</button>{/if}
  <div class="tabs segmented" role="tablist" aria-label="Details panel">
    {#each tabs as t (t.id)}
      <button role="tab" id="dh-tab-{t.id}" aria-controls="dh-panel" aria-selected={tab === t.id} tabindex={tab === t.id ? 0 : -1}
        onkeydown={onTabKey} class:active={tab === t.id} onclick={() => (tab = t.id)} data-testid="design-tab-{t.id}">
        {t.label}{#if t.count != null}<span class="count">{t.count}</span>{/if}
      </button>
    {/each}
  </div>
  <div class="panel" role="tabpanel" id="dh-panel" aria-labelledby="dh-tab-{tab}">
    {#if tab === 'design' && design}
      {@render design()}
    {:else if tab === 'otto'}
      {@render otto()}
    {:else if tab === 'versions'}
      {@render versions()}
    {:else}
      {@render links()}
    {/if}
  </div>
</aside>

<style>
  .right {
    border-inline-start: 1px solid var(--border);
    background: var(--surface);
    min-width: 0;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }
  .tabs {
    margin: 10px 14px 0;
    align-self: flex-start;
    max-inline-size: calc(100% - 28px);
    overflow-x: auto;
  }
  .count {
    color: var(--text-dim);
    margin-inline-start: 4px;
  }
  .panel {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    margin-block-start: 10px;
    border-block-start: 1px solid var(--border);
  }
  .compact-details {
    display: none;
  }
  @container (max-width: 900px) {
    .compact-details {
      display: inline-flex;
    }
    .back-design {
      align-self: flex-start;
      margin: 10px 14px 0;
    }
    .right {
      border-inline-start: 0;
    }
  }
</style>
