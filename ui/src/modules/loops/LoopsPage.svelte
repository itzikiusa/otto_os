<script lang="ts">
  // Older goal loops (goal.md §5.2): Goal Loops are now goals in Workflows.
  // This read-only history keeps the loops made before, reachable from the
  // Runs tab's foot, old notices and pasted `#/loops/<id>` links. An
  // unfinished loop can be continued as a goal (the Goal form on Workflows).
  import { ws } from '../../lib/stores/workspace.svelte';
  import { loops } from '../../lib/stores/loops.svelte';
  import { loopsPagePort } from '../../lib/uiCommands/loops';
  import PageHeader from '../../lib/components/PageHeader.svelte';
  import PageBody from '../../lib/components/PageBody.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import LoopDetail from './LoopDetail.svelte';
  import StatusBadge from '../../lib/components/StatusBadge.svelte';
  import { loopStatusOf } from './loopStatus';
  import RelTime from '../../lib/components/RelTime.svelte';
  import { untrack } from 'svelte';
  import { router } from '../../lib/router.svelte';

  let selectedId = $state<string | null>(null);

  // (Re)load the list when the workspace changes.
  $effect(() => {
    const id = ws.currentId;
    if (id) void loops.loadList(id);
  });

  // A workspace switch closes the open loop (it belongs to the previous workspace).
  // The first workspace of a cold load is not a switch: a pasted `#/loops/<id>`
  // mounts before the workspace is selected and must stay on its loop.
  let viewWs = untrack(() => ws.currentId);
  $effect(() => {
    const id = ws.currentId;
    if (id === viewWs) return;
    const was = viewWs;
    viewWs = id;
    if (was === null) return;
    untrack(() => {
      if (selectedId) back();
    });
  });

  const list = $derived(loops.list);

  function open(id: string): void {
    selectedId = id;
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

  // The URL carries the open loop (`#/loops/<id>`): route → page and back.
  $effect(() => {
    const [mod, id] = router.parts;
    if (mod !== 'loops') return;
    untrack(() => {
      if (id && id !== selectedId) open(id);
      else if (!id && selectedId) back();
    });
  });
  let routedId: string | null = null;
  $effect(() => {
    const id = selectedId;
    const was = routedId;
    routedId = id;
    if (!id && !was) return;
    untrack(() => {
      if (router.module === 'loops' && (router.parts[1] ?? null) !== id) router.replace(id ? `loops/${id}` : 'loops');
    });
  });
</script>

<div class="loops">
  {#if selectedId}
    {#key selectedId}
      <LoopDetail id={selectedId} onback={back} />
    {/key}
  {:else}
    <PageHeader title="Older goal loops" subtitle="Goal Loops are now goals in Workflows. These are kept for reference." />
    <PageBody>
    {#if !ws.currentId}
      <EmptyState variant="page" icon="refresh" title="No workspace" body="Older goal loops belong to a workspace." />
    {:else}
    <LoadState
      what="older goal loops"
      variant="page"
      loading={loops.loadingList}
      error={loops.listError}
      empty={list.length === 0}
      onretry={() => ws.currentId && void loops.loadList(ws.currentId)}
    >
      {#snippet emptyView()}
        <EmptyState variant="page" icon="target" title="No older goal loops" body="Goals now run in Workflows." actionLabel="Go to Workflows" onaction={() => router.go('workflows')} />
      {/snippet}
      <ul class="cards">
        {#each list as l (l.id)}
          <li>
            <button class="loop-card" onclick={() => open(l.id)}>
              <div class="card-top">
                <span class="name" title={l.name}>{l.name}</span>
                <StatusBadge status={loopStatusOf(l)} />
              </div>
              {#if l.definition?.summary}<span class="goal" title={l.definition.summary}>{l.definition.summary}</span>{/if}
              <div class="bar" aria-hidden="true"><span class="bar-fill status-{l.status}" style:width={`${l.progress_pct}%`}></span></div>
              <div class="card-meta">
                <span>{l.config.graph ? 'Engineering graph' : 'Goal loop'}</span>
                <span>Iteration {l.current_iteration} of {l.limits.max_iterations}</span>
                <span>{l.progress_pct}% complete</span>
                {#if l.updated_at}<span class="when">Updated <RelTime iso={l.updated_at} /></span>{/if}
              </div>
            </button>
          </li>
        {/each}
      </ul>
    </LoadState>
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
  .cards {
    list-style: none;
    margin: 0;
    padding: 0;
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(min(100%, 260px), 1fr));
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
    outline: 2px solid var(--accent-text);
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
    border-radius: var(--radius-s);
    background: var(--surface-2);
    overflow: hidden;
  }
  .bar-fill {
    display: block;
    height: 100%;
    background: var(--text-dim);
  }
  .bar-fill.status-running {
    background: var(--info);
  }
  .bar-fill.status-blocked,
  .bar-fill.status-exhausted {
    background: var(--warning);
  }
  .bar-fill.status-failed {
    background: var(--danger);
  }
  .bar-fill.status-succeeded {
    background: var(--success);
  }
  .bar-fill.status-paused,
  .bar-fill.status-draft,
  .bar-fill.status-stopped {
    background: var(--text-dim);
  }
</style>
