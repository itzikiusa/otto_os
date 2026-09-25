<script lang="ts">
  import { ws } from '../../lib/stores/workspace.svelte';
  import { loops } from '../../lib/stores/loops.svelte';
  import { loopsPagePort } from '../../lib/uiCommands/loops';
  import GoalDefineForm from './GoalDefineForm.svelte';
  import PageHeader from '../../lib/components/PageHeader.svelte';
  import PageBody from '../../lib/components/PageBody.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import LoopDetail from './LoopDetail.svelte';
  import StatusBadge from '../../lib/components/StatusBadge.svelte';
  import { loopStatus } from './loopStatus';
  import Icon from '../../lib/components/Icon.svelte';
  import RelTime from '../../lib/components/RelTime.svelte';

  const PHASE_LABEL: Record<string, string> = {
    planning: 'Planning',
    executing: 'Executing',
    evaluating: 'Evaluating',
    digesting: 'Digesting',
    waiting: 'Waiting on an agent',
  };

  let selectedId = $state<string | null>(null);
  let creating = $state(false);

  // (Re)load the list when the workspace changes.
  $effect(() => {
    const id = ws.currentId;
    if (id) void loops.loadList(id);
  });

  const list = $derived(loops.list);

  function open(id: string): void {
    selectedId = id;
    creating = false;
    void loops.loadDetail(id);
  }
  function back(): void {
    selectedId = null;
    loops.closeDetail();
    const id = ws.currentId;
    if (id) void loops.loadList(id);
  }
  // Agent UI control (lib/uiCommands/loops.ts) opens a loop's detail here.
  $effect(() => loopsPagePort.bind({ open, selectedId: () => selectedId }));
</script>

<div class="loops">
  {#if selectedId}
    <LoopDetail id={selectedId} onback={back} />
  {:else if creating}
    <GoalDefineForm oncancel={() => (creating = false)} oncreated={open} />
  {:else}
    <PageHeader
      title="Goal Loops"
      subtitle="Agents iterate toward a goal within a budget"
    >
      {#snippet actions()}
        <!-- One primary per page: while the list is empty the empty state owns
             the "New goal loop" CTA. -->
        {#if list.length > 0}
          <button class="btn small primary" onclick={() => (creating = true)}><Icon name="plus" size={12} /> New goal loop</button>
        {/if}
      {/snippet}
    </PageHeader>
    <PageBody>

    <LoadState
      what="goal loops"
      variant="page"
      loading={loops.loadingList}
      error={loops.listError}
      empty={list.length === 0}
      onretry={() => ws.currentId && void loops.loadList(ws.currentId)}
    >
      {#snippet emptyView()}
        <EmptyState
          variant="page"
          icon="refresh"
          title="No goal loops yet"
          body="Define a goal and a budget — agents iterate on an isolated branch until it's met."
          actionLabel="New goal loop"
          actionIcon="plus"
          onaction={() => (creating = true)}
        />
      {/snippet}
      <ul class="cards">
        {#each list as l (l.id)}
          <li>
            <button class="loop-card" onclick={() => open(l.id)}>
              <div class="card-top">
                <span class="name" title={l.name}>{l.name}</span>
                <StatusBadge status={loopStatus(l.status)} />
              </div>
              {#if l.definition?.summary}<span class="goal" title={l.definition.summary}>{l.definition.summary}</span>{/if}
              <div class="bar" aria-hidden="true"><span class="bar-fill" class:done={l.status === 'succeeded'} style:width={`${l.progress_pct}%`}></span></div>
              <div class="card-meta">
                <span>Iteration {l.current_iteration} of {l.limits.max_iterations}</span>
                <span>{l.progress_pct}% complete</span>
                {#if l.status === 'running' && PHASE_LABEL[l.phase]}<span class="phase">{PHASE_LABEL[l.phase]}</span>
                {:else if l.updated_at}<span class="when">Updated <RelTime iso={l.updated_at} /></span>{/if}
              </div>
            </button>
          </li>
        {/each}
      </ul>
    </LoadState>
    </PageBody>
  {/if}
</div>

<style>
  .loops {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }
  .cards {
    list-style: none;
    margin: 0;
    padding: 0;
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(260px, 1fr));
    gap: 12px;
  }
  .loop-card {
    width: 100%;
    text-align: start;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    padding: 12px 14px;
    cursor: pointer;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .loop-card:hover {
    border-color: var(--border-strong);
    background: var(--hover);
  }
  .loop-card:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
  }
  .card-top {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
  }
  .name {
    font-weight: 600;
    font-size: var(--fs-m);
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .card-meta {
    display: flex;
    flex-wrap: wrap;
    gap: 4px 12px;
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .phase {
    color: var(--text);
  }
  .when {
    margin-inline-start: auto;
  }
  .goal {
    font-size: var(--fs-s);
    color: var(--text-dim);
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }
  .bar {
    height: 5px;
    border-radius: 3px;
    background: var(--surface-2);
    overflow: hidden;
  }
  .bar-fill {
    display: block;
    height: 100%;
    background: var(--status-working);
  }
  .bar-fill.done {
    background: var(--success);
  }
</style>
