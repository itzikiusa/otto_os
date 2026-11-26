<script lang="ts">
  // The Workflows tab (ux.md §8): the definitions in the reading column, each
  // row its last run's word (else when it ended) and a dim line; goals
  // (Goal Loops folded in) in their own fold. Right-click / long-press: the
  // row menu the page hands in. A filter above the list once it is long.
  import EmptyState from '../../../lib/components/EmptyState.svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import InboxFold from '../../../lib/components/InboxFold.svelte';
  import InboxRow from '../../../lib/components/InboxRow.svelte';
  import LoadState from '../../../lib/components/LoadState.svelte';
  import { ctxMenu, type MenuItem } from '../../../lib/contextmenu.svelte';
  import { now as clockNow } from '../../../lib/stores/now.svelte';
  import type { Workflow } from '../../../lib/api/types';
  import { splitGoals, workflowRow } from './workflowRows';

  interface Props {
    workflows: readonly Workflow[];
    loading: boolean;
    error: string | null;
    onretry: () => void;
    onopen: (wf: Workflow) => void;
    menu: (wf: Workflow) => MenuItem[];
    /** The empty state's primary (absent on a phone). */
    onnew?: () => void;
  }
  let { workflows, loading, error, onretry, onopen, menu, onnew }: Props = $props();

  let query = $state('');
  let goalsOpen = $state(false);
  const now = $derived(clockNow());
  const shown = $derived(query.trim() ? workflows.filter((w) => w.name.toLowerCase().includes(query.trim().toLowerCase())) : workflows);
  const split = $derived(splitGoals(shown));

  function openMenu(e: MouseEvent, wf: Workflow): void {
    e.preventDefault();
    ctxMenu.show(e, menu(wf));
  }
</script>

{#snippet row(wf: Workflow)}
  {@const w = workflowRow(wf, now)}
  <div data-testid="wf-workflow-row" data-workflow-id={wf.id}>
    <InboxRow twoLine onPage title={wf.name} word={w.word} tone={w.tone} time={w.time} strong={w.tone === 'warning'} onclick={() => onopen(wf)} oncontextmenu={(e) => openMenu(e, wf)}>
      {#snippet line3()}<span class="ellipsis" dir="auto">{w.line3}</span>{/snippet}
      {#snippet actions()}
        <button type="button" class="icon-btn" aria-label="Workflow actions" title="Workflow actions" onclick={(e) => { e.stopPropagation(); openMenu(e, wf); }}><Icon name="more" size={14} /></button>
      {/snippet}
    </InboxRow>
  </div>
{/snippet}

<div class="read-col wf-list">
  <LoadState what="workflows" variant="page" loading={loading && workflows.length === 0} {error} empty={workflows.length === 0} {onretry}>
    {#snippet emptyView()}
      <EmptyState variant="page" icon="workbench" title="No workflows yet" body="A workflow chains agent steps, checks and messages into one run." actionLabel={onnew ? 'New workflow' : undefined} onaction={onnew} headingLevel={2} />
    {/snippet}
    {#if workflows.length > 8}
      <span class="search">
        <Icon name="search" size={13} />
        <input type="search" dir="auto" bind:value={query} placeholder="Filter workflows…" aria-label="Filter workflows" />
      </span>
    {/if}
    <InboxFold label="Workflows" count={split.workflows.length}>
      {#each split.workflows as wf (wf.id)}{@render row(wf)}{/each}
      {#if !split.workflows.length}<p class="none">{query.trim() ? `No workflows match “${query.trim()}”.` : 'Only goals so far.'}</p>{/if}
    </InboxFold>
    {#if split.goals.length}
      <InboxFold label="Goals" count={split.goals.length} expanded={goalsOpen} ontoggle={() => (goalsOpen = !goalsOpen)} testid="wf-goals-fold">
        {#if goalsOpen}{#each split.goals as wf (wf.id)}{@render row(wf)}{/each}{/if}
      </InboxFold>
    {/if}
  </LoadState>
</div>

<style>
  .wf-list {
    display: flex;
    flex-direction: column;
    gap: var(--sp-6);
    padding-block: var(--sp-6);
  }
  .search {
    display: flex;
    align-items: center;
    gap: var(--sp-3);
    block-size: 32px;
    padding-inline: var(--sp-4);
    border: 1px solid var(--border-strong);
    border-radius: var(--radius-m);
    background: var(--surface);
    color: var(--text-dim);
  }
  .search:focus-within {
    border-color: var(--accent-line);
    box-shadow: 0 0 0 2px var(--accent-soft);
  }
  .search input {
    flex: 1;
    min-inline-size: 0;
    border: 0;
    background: transparent;
    color: var(--text);
    font: inherit;
    font-size: var(--fs-m);
    outline: none;
  }
  .none {
    margin: 0;
    padding: var(--sp-5);
    color: var(--text-dim);
    font-size: var(--fs-m);
  }
  @media (max-width: 640px) {
    .wf-list {
      padding-inline: var(--sp-4);
    }
    .search {
      block-size: 40px;
    }
  }
</style>
