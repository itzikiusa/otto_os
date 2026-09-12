<script lang="ts">
  // PR list with state filter chips. Click → PrDetail route.
  import { api, ApiError } from '../../lib/api/client';
  import type { PrListResp, PrState, PrSummary } from '../../lib/api/types';
  import { router } from '../../lib/router.svelte';
  import Skeleton from '../../lib/components/Skeleton.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import CreatePr from './CreatePr.svelte';

  interface Props {
    repoId: string;
  }
  let { repoId }: Props = $props();

  const PER_PAGE = 50;

  const states: PrState[] = ['open', 'merged', 'declined', 'all'];
  let stateFilter: PrState = $state('open');
  let prs: PrSummary[] = $state([]);
  let page = $state(1);
  let hasMore = $state(false);
  let loadingMore = $state(false);
  /** Client-side filter over the pages already loaded (title/author/branch/#). */
  let query = $state('');
  let loading = $state(true);
  let error = $state('');
  /** Headline for the error state — distinguishes "your token is bad" from
   *  "the provider is down" instead of labelling every failure "unreachable". */
  let errorTitle = $state('Provider unreachable');
  let createOpen = $state(false);
  // Bumped by the Retry button to re-run the load effect.
  let retryRev = $state(0);

  $effect(() => {
    const id = repoId;
    const st = stateFilter;
    void retryRev;
    loading = true;
    error = '';
    void api
      .get<PrListResp>(`/repos/${id}/prs?state=${st}&page=1&per_page=${PER_PAGE}`)
      .then((r) => {
        prs = r.items;
        hasMore = r.has_more;
        page = 1;
      })
      .catch((e) => {
        prs = [];
        hasMore = false;
        error = e instanceof Error ? e.message : 'failed to load PRs';
        errorTitle =
          e instanceof ApiError && (e.status === 401 || e.status === 403)
            ? 'Provider rejected the credentials'
            : e instanceof ApiError && e.status === 404
              ? 'Repository not found on the provider'
              : 'Provider unreachable';
      })
      .finally(() => (loading = false));
  });

  /** Append the next page. Failures toast nothing — the button simply stays,
   *  so a flaky provider never wipes the rows already on screen. */
  async function loadMore(): Promise<void> {
    if (loadingMore || !hasMore) return;
    loadingMore = true;
    try {
      const next = page + 1;
      const r = await api.get<PrListResp>(
        `/repos/${repoId}/prs?state=${stateFilter}&page=${next}&per_page=${PER_PAGE}`,
      );
      prs = [...prs, ...r.items];
      hasMore = r.has_more;
      page = next;
    } catch (e) {
      error = e instanceof Error ? e.message : 'failed to load more PRs';
    } finally {
      loadingMore = false;
    }
  }

  const filtered = $derived.by(() => {
    const q = query.trim().toLowerCase();
    if (q === '') return prs;
    return prs.filter((p) =>
      [p.title, p.author, p.source_branch, p.target_branch, String(p.number)].some((v) =>
        v.toLowerCase().includes(q),
      ),
    );
  });

  /** CI chip for a row; `null` when the provider reported no CI at all. */
  function ciChip(state: string | null | undefined): { glyph: string; cls: string } | null {
    if (state === 'passing' || state === 'success') return { glyph: '✓', cls: 'ok' };
    if (state === 'failing' || state === 'failure') return { glyph: '✗', cls: 'bad' };
    if (state === 'pending') return { glyph: '●', cls: 'dim' };
    return null;
  }

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
    <input
      class="input pr-search"
      type="search"
      bind:value={query}
      placeholder="Search title, author, branch…"
      aria-label="Search pull requests"
    />
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
      title={errorTitle}
      body={error}
      actionLabel="Retry"
      onaction={() => retryRev++}
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
      {#each filtered as pr (pr.number)}
        {@const ci = ciChip(pr.ci_status)}
        <button class="pr-row card" onclick={() => router.go(`git/${repoId}/pr/${pr.number}`)}>
          <div class="pr-main">
            <div class="pr-title">
              <span class="pr-num dim">#{pr.number}</span>
              {pr.title}
            </div>
            <div class="pr-meta">
              <span class="chip {stateColors[pr.state] ?? ''}">{pr.state}</span>
              {#if ci}
                <span class="ci-chip {ci.cls}" title="CI {pr.ci_status}">{ci.glyph}</span>
              {/if}
              <span class="dim">{pr.author}</span>
              <span class="mono dim">{pr.source_branch} <span class="dir-arrow">→</span> {pr.target_branch}</span>
              <span class="grow"></span>
              <span class="dim">updated {fmtDate(pr.updated_at)}</span>
            </div>
          </div>
        </button>
      {/each}
      {#if filtered.length === 0}
        <div class="pr-nomatch dim">No pull request matches “{query}”.</div>
      {/if}
    </div>
    {#if hasMore}
      <div class="pr-more">
        <button class="btn small" disabled={loadingMore} onclick={loadMore}>
          {loadingMore ? 'Loading…' : 'Load more'}
        </button>
      </div>
    {/if}
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
  .pr-search {
    height: 22px;
    width: 200px;
    margin-inline-start: 10px;
    font-size: 11.5px;
  }
  .pr-rows {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .pr-nomatch {
    padding: 14px 2px;
    font-size: 12px;
  }
  .pr-more {
    display: flex;
    justify-content: center;
    margin-top: 10px;
  }
  .ci-chip {
    font-weight: 700;
    font-size: 12px;
    line-height: 1;
  }
  .ci-chip.ok {
    color: var(--status-idle, #34c759);
  }
  .ci-chip.bad {
    color: var(--status-exited);
  }
  .ci-chip.dim {
    color: var(--text-dim);
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
    /* Full-width search on its own row so the chips + New PR stay reachable. */
    .pr-search { height: 32px; width: 100%; margin-inline-start: 0; font-size: 13px; }
    .pr-toolbar .btn.small { height: 32px; }
    .pr-row { padding: 12px 14px; }
    .pr-title { font-size: 14px; overflow-wrap: anywhere; }
    .pr-meta { flex-wrap: wrap; gap: 6px 10px; font-size: 12.5px; min-width: 0; }
    .pr-meta .grow { display: none; }
    /* Long branch names break instead of forcing horizontal overflow. */
    .pr-meta .mono { overflow-wrap: anywhere; min-width: 0; }
  }
</style>
