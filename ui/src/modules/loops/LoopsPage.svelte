<script lang="ts">
  import { ws } from '../../lib/stores/workspace.svelte';
  import { loops } from '../../lib/stores/loops.svelte';
  import GoalDefineForm from './GoalDefineForm.svelte';
  import PageHeader from '../../lib/components/PageHeader.svelte';
  import PageBody from '../../lib/components/PageBody.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import LoopDetail from './LoopDetail.svelte';

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

  function pillClass(status: string): string {
    switch (status) {
      case 'running':
        return 'pill working';
      case 'succeeded':
        return 'pill ok';
      case 'failed':
      case 'stopped':
        return 'pill bad';
      case 'blocked':
      case 'exhausted':
        return 'pill warn';
      default:
        return 'pill';
    }
  }
</script>

<div class="loops">
  {#if selectedId}
    <LoopDetail id={selectedId} onback={back} />
  {:else if creating}
    <GoalDefineForm oncancel={() => (creating = false)} oncreated={open} />
  {:else}
    <PageHeader
      title="Goal Loops"
      subtitle="Give a goal + a budget; a team of agents iterates toward it on an isolated branch until the acceptance criteria are met or a limit is hit."
    >
      {#snippet actions()}
        <!-- One primary per page: while the list is empty the empty state owns
             the "New goal loop" CTA. -->
        {#if list.length > 0}
          <button class="btn primary" onclick={() => (creating = true)}>New goal loop</button>
        {/if}
      {/snippet}
    </PageHeader>
    <PageBody>

    {#if loops.loadingList && list.length === 0}
      <p class="muted">Loading…</p>
    {:else if list.length === 0}
      <EmptyState
        variant="page"
        icon="refresh"
        title="No goal loops yet"
        body="Define a goal and a budget — agents iterate on an isolated branch until it's met."
        actionLabel="New goal loop"
        actionIcon="plus"
        onaction={() => (creating = true)}
      />
    {:else}
      <ul class="cards">
        {#each list as l (l.id)}
          <li>
            <button class="card" onclick={() => open(l.id)}>
              <div class="card-top">
                <span class="name">{l.name}</span>
                <span class={pillClass(l.status)}>{l.status}</span>
              </div>
              <div class="bar"><span class="bar-fill" style:width={`${l.progress_pct}%`}></span></div>
              <div class="card-meta">
                <span>iter {l.current_iteration}/{l.limits.max_iterations}</span>
                <span>{l.progress_pct}%</span>
                {#if l.status === 'running'}<span class="phase">{l.phase}</span>{/if}
              </div>
            </button>
          </li>
        {/each}
      </ul>
    {/if}
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
  .muted {
    color: var(--text-dim);
  }
  .cards {
    list-style: none;
    margin: 0;
    padding: 0;
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(260px, 1fr));
    gap: 12px;
  }
  .card {
    width: 100%;
    text-align: left;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    padding: 12px 14px;
    cursor: pointer;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .card:hover {
    background: var(--surface-2);
  }
  .card-top {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
  }
  .name {
    font-weight: 600;
    font-size: 13px;
  }
  .card-meta {
    display: flex;
    gap: 12px;
    font-size: 11.5px;
    color: var(--text-dim);
  }
  .phase {
    color: var(--status-working);
    text-transform: capitalize;
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
  .pill {
    font-size: 11px;
    padding: 1px 8px;
    border-radius: 999px;
    background: var(--surface-2);
    color: var(--text-dim);
    text-transform: capitalize;
  }
  .pill.working {
    background: color-mix(in srgb, var(--status-working) 18%, transparent);
    color: var(--status-working);
  }
  .pill.ok {
    background: var(--success-soft);
    color: var(--success);
    font-weight: 600;
  }
  .pill.bad {
    background: color-mix(in srgb, var(--status-exited) 16%, transparent);
    color: var(--status-exited);
  }
  .pill.warn {
    background: var(--status-warn-soft);
    color: var(--status-warn);
  }
</style>
