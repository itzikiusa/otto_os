<script lang="ts">
  // PR list with state filter chips. Click → PrDetail route.
  import { api } from '../../lib/api/client';
  import type { PrState, PrSummary } from '../../lib/api/types';
  import { router } from '../../lib/router.svelte';
  import Skeleton from '../../lib/components/Skeleton.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import CreatePr from './CreatePr.svelte';

  interface Props {
    repoId: string;
  }
  let { repoId }: Props = $props();

  const states: PrState[] = ['open', 'merged', 'declined', 'all'];
  let stateFilter: PrState = $state('open');
  let prs: PrSummary[] = $state([]);
  let loading = $state(true);
  let error = $state('');
  let createOpen = $state(false);

  $effect(() => {
    const id = repoId;
    const st = stateFilter;
    loading = true;
    error = '';
    void api
      .get<PrSummary[]>(`/repos/${id}/prs?state=${st}`)
      .then((p) => (prs = p))
      .catch((e) => {
        prs = [];
        error = e instanceof Error ? e.message : 'failed to load PRs';
      })
      .finally(() => (loading = false));
  });

  function fmtDate(iso: string): string {
    return new Date(iso).toLocaleDateString([], { month: 'short', day: 'numeric' });
  }

  const stateColors: Record<string, string> = {
    open: 'ok',
    merged: 'accent',
    declined: 'bad',
  };
</script>

<div class="prlist">
  <div class="pr-toolbar">
    <div class="row">
      {#each states as s (s)}
        <button class="chip filter-chip" class:active={stateFilter === s} onclick={() => (stateFilter = s)}>
          {s}
        </button>
      {/each}
    </div>
    <span class="grow"></span>
    <button class="btn primary small" onclick={() => (createOpen = true)}>
      <Icon name="pr" size={11} /> New PR
    </button>
  </div>

  {#if loading}
    <Skeleton rows={4} height={48} />
  {:else if error}
    <EmptyState
      icon="pr"
      title="Provider unreachable"
      body={error}
    />
  {:else if prs.length === 0}
    <EmptyState
      icon="pr"
      title="No {stateFilter === 'all' ? '' : stateFilter} pull requests"
      body="Create one from your current branch, or change the filter."
      actionLabel="New Pull Request"
      onaction={() => (createOpen = true)}
    />
  {:else}
    <div class="pr-rows">
      {#each prs as pr (pr.number)}
        <button class="pr-row card" onclick={() => router.go(`git/${repoId}/pr/${pr.number}`)}>
          <div class="pr-main">
            <div class="pr-title">
              <span class="pr-num dim">#{pr.number}</span>
              {pr.title}
            </div>
            <div class="pr-meta">
              <span class="chip {stateColors[pr.state] ?? ''}">{pr.state}</span>
              <span class="dim">{pr.author}</span>
              <span class="mono dim">{pr.source_branch} <span class="dir-arrow">→</span> {pr.target_branch}</span>
              <span class="grow"></span>
              <span class="dim">updated {fmtDate(pr.updated_at)}</span>
            </div>
          </div>
        </button>
      {/each}
    </div>
  {/if}
</div>

{#if createOpen}
  <CreatePr
    {repoId}
    onclose={() => (createOpen = false)}
    oncreated={(pr) => {
      createOpen = false;
      router.go(`git/${repoId}/pr/${pr.number}`);
    }}
  />
{/if}

<style>
  .prlist {
    padding: 12px 14px;
    overflow-y: auto;
    height: 100%;
  }
  .pr-toolbar {
    display: flex;
    align-items: center;
    margin-bottom: 12px;
  }
  .filter-chip {
    cursor: pointer;
    height: 22px;
    background: transparent;
  }
  .filter-chip.active {
    background: color-mix(in srgb, var(--accent) 15%, transparent);
    border-color: color-mix(in srgb, var(--accent) 45%, transparent);
    color: var(--accent);
  }
  .pr-rows {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .pr-row {
    text-align: start;
    padding: 10px 14px;
    cursor: pointer;
    transition: border-color 130ms ease-out;
  }
  .pr-row:hover {
    border-color: color-mix(in srgb, var(--accent) 40%, var(--border));
  }
  .pr-title {
    font-size: 13px;
    font-weight: 600;
  }
  .pr-num {
    font-weight: 400;
    margin-inline-end: 4px;
  }
  .pr-meta {
    display: flex;
    align-items: center;
    gap: 10px;
    margin-top: 5px;
    font-size: 11.5px;
  }
  /* source→target separator mirrors in place under RTL. */
  .dir-arrow {
    display: inline-block;
  }
  :global([dir='rtl']) .dir-arrow {
    transform: scaleX(-1);
  }

  /* ── Mobile + tablet (≤1024px): legible PR cards whose meta wraps instead of
     overflowing. 1024 so iPad portrait + landscape phone widths also wrap. */
  @media (max-width: 1024px) {
    .prlist { padding: 12px; }
    .pr-toolbar { flex-wrap: wrap; gap: 8px; }
    .pr-toolbar .row { flex-wrap: wrap; gap: 6px; }
    .filter-chip { height: 32px; padding: 0 12px; font-size: 13px; }
    .pr-toolbar .btn.small { height: 32px; }
    .pr-row { padding: 12px 14px; }
    .pr-title { font-size: 14px; overflow-wrap: anywhere; }
    .pr-meta { flex-wrap: wrap; gap: 6px 10px; font-size: 12.5px; min-width: 0; }
    .pr-meta .grow { display: none; }
    /* Long branch names break instead of forcing horizontal overflow. */
    .pr-meta .mono { overflow-wrap: anywhere; min-width: 0; }
  }
</style>
