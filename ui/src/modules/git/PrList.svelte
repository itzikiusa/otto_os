<script lang="ts">
  // PR list with state filter chips. Click → PrDetail route.
  import { api, ApiError } from '../../lib/api/client';
  import type { PrListResp, PrState, PrSummary } from '../../lib/api/types';
  import { router } from '../../lib/router.svelte';
  import Skeleton from '../../lib/components/Skeleton.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import CreatePr from './CreatePr.svelte';
  import { rel } from '../../lib/stores/now.svelte';
  import type { IconName } from '../../lib/components/Icon.svelte';

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
  /** A failed "Load more" — shown inline beside the button. It used to write
   *  `error`, which swapped the WHOLE list for the error state and wiped the
   *  rows already on screen (the opposite of what loadMore promises). */
  let moreError = $state('');
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
    moreError = '';
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

  /** Append the next page. Failures toast nothing — the button simply stays
   *  (with an inline note), so a flaky provider never wipes the rows already
   *  on screen. */
  async function loadMore(): Promise<void> {
    if (loadingMore || !hasMore) return;
    loadingMore = true;
    moreError = '';
    try {
      const next = page + 1;
      const r = await api.get<PrListResp>(
        `/repos/${repoId}/prs?state=${stateFilter}&page=${next}&per_page=${PER_PAGE}`,
      );
      prs = [...prs, ...r.items];
      hasMore = r.has_more;
      page = next;
    } catch (e) {
      moreError = e instanceof Error ? e.message : 'failed to load more PRs';
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
  function ciChip(state: string | null | undefined): { icon: IconName; cls: string; label: string } | null {
    if (state === 'passing' || state === 'success') return { icon: 'check', cls: 'ok', label: 'CI passing' };
    if (state === 'failing' || state === 'failure') return { icon: 'x', cls: 'bad', label: 'CI failing' };
    if (state === 'pending') return { icon: 'clock', cls: 'warn', label: 'CI running' };
    return null;
  }

  const STATE_LABEL: Record<PrState, string> = {
    open: 'Open',
    merged: 'Merged',
    declined: 'Declined',
    all: 'All',
  };

  const stateColors: Record<string, string> = {
    open: 'ok',
    merged: 'accent',
    declined: 'bad',
  };
</script>

<div class="prlist">
  <div class="pr-toolbar">
    <div class="segmented" role="group" aria-label="Pull request state">
      {#each states as s (s)}
        <button
          class:active={stateFilter === s}
          aria-pressed={stateFilter === s}
          onclick={() => (stateFilter = s)}
        >
          {STATE_LABEL[s]}
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
    <!-- While the list is empty the empty state carries the one "New" CTA. -->
    {#if loading || error || prs.length > 0}
      <button class="btn primary small" onclick={() => (createOpen = true)}>
        <Icon name="pr" size={12} /> New pull request
      </button>
    {/if}
  </div>

  {#if loading}
    <Skeleton rows={4} height={48} />
  {:else if error}
    <EmptyState
      icon="warning"
      title={errorTitle}
      body={error}
      actionLabel="Retry"
      actionIcon="refresh"
      onaction={() => retryRev++}
    />
  {:else if prs.length === 0}
    <EmptyState
      icon="pr"
      title={stateFilter === 'all' ? 'No pull requests' : `No ${stateFilter} pull requests`}
      body="Open one from your current branch, or pick another state above."
      actionLabel="New pull request"
      actionIcon="pr"
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
              <span class="chip {stateColors[pr.state] ?? ''}">{STATE_LABEL[pr.state] ?? pr.state}</span>
              {#if ci}
                <span class="ci-chip {ci.cls}" title={ci.label} role="img" aria-label={ci.label}><Icon name={ci.icon} size={12} /></span>
              {/if}
              <span class="dim">{pr.author}</span>
              <span class="mono dim pr-branches" title="{pr.source_branch} → {pr.target_branch}">{pr.source_branch} <span class="dir-arrow">→</span> {pr.target_branch}</span>
              <span class="grow"></span>
              <span class="dim pr-updated" title={new Date(pr.updated_at).toLocaleString()}>updated {rel(pr.updated_at)}</span>
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
          {loadingMore ? 'Loading…' : moreError ? 'Retry' : 'Load more'}
        </button>
        {#if moreError}<span class="pr-more-err" role="status">Couldn't load more: {moreError}</span>{/if}
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
  .pr-search {
    height: 26px;
    width: 220px;
    margin-inline-start: 10px;
    font-size: var(--fs-s);
  }
  .pr-rows {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .pr-nomatch {
    padding: 14px 2px;
    font-size: var(--fs-s);
  }
  .pr-more {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 10px;
    margin-top: 10px;
  }
  .pr-more-err {
    color: var(--danger);
    font-size: var(--fs-s);
  }
  /* A long source → target pair ellipsizes (full pair in the title) instead of
     pushing "updated …" out of the card. */
  .pr-branches {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .pr-updated {
    flex-shrink: 0;
    white-space: nowrap;
  }
  .ci-chip {
    display: inline-flex;
    align-items: center;
  }
  /* CI glyph tones: passing = success, failing = danger, pending = warning
     (the glyph shape carries the meaning too: ✓ ✗ ●). */
  .ci-chip.ok {
    color: var(--success);
  }
  .ci-chip.bad {
    color: var(--danger);
  }
  .ci-chip.warn {
    color: var(--warning);
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
    font-size: var(--fs-m);
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
    font-size: var(--fs-xs);
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
    .pr-toolbar .segmented > button { height: 32px; padding: 0 12px; }
    /* Full-width search on its own row so the chips + New PR stay reachable. */
    .pr-search { height: 32px; width: 100%; margin-inline-start: 0; font-size: var(--fs-m); }
    .pr-toolbar .btn.small { height: 32px; }
    .pr-row { padding: 12px 14px; }
    .pr-title { font-size: var(--fs-l); overflow-wrap: anywhere; }
    .pr-meta { flex-wrap: wrap; gap: 6px 10px; font-size: var(--fs-s); min-width: 0; }
    .pr-meta .grow { display: none; }
    /* Long branch names break instead of forcing horizontal overflow. */
    .pr-meta .mono { overflow-wrap: anywhere; min-width: 0; }
    .pr-branches { white-space: normal; }
  }
</style>
