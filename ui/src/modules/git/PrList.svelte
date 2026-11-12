<script lang="ts">
  // PR list — the left pane of the PR workspace (also usable on its own).
  // Search takes free text + qualifiers (pr-query.ts), Sort / Filters menus,
  // "Authored by you" grouping, lazy +/- counts per visible row, multi-select
  // "Open N together", and a quiet background refresh every 2 minutes that
  // never resets scroll, selection or the search.
  import { api, ApiError } from '../../lib/api/client';
  import type { PrListResp, PrState, PrSummary } from '../../lib/api/types';
  import { myGitHandles } from './myGitHandles.svelte';
  import { router } from '../../lib/router.svelte';
  import Skeleton from '../../lib/components/Skeleton.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import CreatePr from './CreatePr.svelte';
  import { rel } from '../../lib/stores/now.svelte';
  import { loadErrorText } from '../../lib/loadError';
  import { plural } from '../../lib/plural';
  import { ctxMenu } from '../../lib/contextmenu.svelte';
  import { invalidatePr, prListCache, prListKey } from './pr-cache';
  import { ciOf, groupPrs, hasClientFilter, isMine, matchesPr, mergeRefreshed, parsePrQuery, sortPrs, SORT_LABEL, type PrGroup } from './pr-query';
  import { PrStatsLoader } from './pr-stats.svelte';
  import PrListRow from './PrListRow.svelte';
  import { CLEARED_FILTERS, DEFAULT_PREFS, activeFilters, authorItems, filterItems, readPrefs, sortItems, writePrefs, type PrListPrefs } from './pr-list-prefs';
  import { SvelteSet } from 'svelte/reactivity';
  import { untrack } from 'svelte';
  import { pollWhileVisible } from '../../lib/poll';

  interface Props {
    repoId: string;
    /** Highlighted row (the PR shown beside the list). */
    activeNumber?: number | null;
    /** Rows with an open tab in the workspace (a small dot). */
    openNumbers?: number[];
    /** Click = foreground; ⌘/Ctrl-click = background tab. Absent → route to the PR. */
    onopen?: (pr: PrSummary, opts: { background: boolean }) => void;
    /** "Open N together". */
    onopenmany?: (prs: PrSummary[]) => void;
    /** A fresher copy of one PR (the detail pane's read) — merged into its row. */
    patch?: PrSummary | null;
  }
  let { repoId, activeNumber = null, openNumbers = [], onopen, onopenmany, patch = null }: Props = $props();

  const PER_PAGE = 50;
  const REFRESH_MS = 2 * 60_000;

  // ── Per-repo preferences (sort, state, filters, folded groups) ──────────
  let prefs = $state<PrListPrefs>({ ...DEFAULT_PREFS });
  let prefsFor = '';
  $effect.pre(() => {
    if (prefsFor !== repoId) {
      prefsFor = repoId;
      prefs = readPrefs(repoId);
    }
  });
  function setPrefs(p: Partial<PrListPrefs>): void {
    prefs = { ...prefs, ...p };
    writePrefs(repoId, prefs);
  }

  // ── Search (debounced parse; `is:` overrides the state chips) ───────────
  let query = $state('');
  let parsedSource = $state('');
  $effect(() => {
    const v = query;
    const t = setTimeout(() => (parsedSource = v), v === '' ? 0 : 180);
    return () => clearTimeout(t);
  });
  const parsed = $derived(parsePrQuery(parsedSource));
  const effState: PrState = $derived(parsed.state ?? prefs.state);

  // ── Data ─────────────────────────────────────────────────────────────────
  const states: PrState[] = ['open', 'merged', 'declined', 'all'];
  let prs: PrSummary[] = $state.raw([]);
  /** Rows the last page-1 read returned (a refresh replaces exactly those). */
  let page1Count = 0;
  let page = $state(1);
  let hasMore = $state(false);
  let loadingMore = $state(false);
  let loading = $state(true);
  let refreshing = $state(false);
  let lastUpdated = $state(0);
  let error = $state('');
  /** A failed "Load more" — inline beside the button, never wipes the rows. */
  let moreError = $state('');
  const moreErrId = $props.id();
  let errorCause = $state('The provider is unreachable.');
  let createOpen = $state(false);
  let retryRev = $state(0);
  let loadRevision = 0;
  let lastRetryRev = 0;

  function causeOf(e: unknown): string {
    return e instanceof ApiError && (e.status === 401 || e.status === 403)
      ? 'The provider rejected the credentials.'
      : e instanceof ApiError && e.status === 404
        ? 'The repository wasn’t found on the provider.'
        : 'The provider is unreachable.';
  }

  $effect(() => {
    const id = repoId;
    const st = effState;
    void retryRev;
    const revision = ++loadRevision;
    let active = true;
    loadingMore = false;
    refreshing = false;
    error = '';
    moreError = '';
    untrack(() => selected.clear());
    // Stale-while-revalidate: paint the last answer at once; a fresh one
    // (<45 s, not a Retry) needs no request at all.
    const key = prListKey(id, st, 1);
    const hit = prListCache.get(key);
    if (hit) {
      prs = hit.value.items;
      page1Count = hit.value.items.length;
      hasMore = hit.value.has_more;
      page = 1;
      loading = false;
      if (hit.fresh && retryRev === lastRetryRev) {
        untrack(() => (lastUpdated ||= Date.now()));
        return () => {
          active = false;
          if (loadRevision === revision) loadRevision++;
        };
      }
    } else {
      loading = true;
    }
    lastRetryRev = retryRev;
    void api
      .get<PrListResp>(`/repos/${id}/prs?state=${st}&page=1&per_page=${PER_PAGE}`)
      .then((r) => {
        prListCache.set(key, r);
        if (!active) return;
        prs = r.items;
        page1Count = r.items.length;
        hasMore = r.has_more;
        page = 1;
        lastUpdated = Date.now();
      })
      .catch((e) => {
        if (!active) return;
        if (hit) return; // a failed revalidation keeps the cached rows
        prs = [];
        hasMore = false;
        error = e instanceof Error && e.message ? e.message : 'Couldn’t load pull requests.';
        errorCause = causeOf(e);
      })
      .finally(() => {
        if (active) loading = false;
      });
    return () => {
      active = false;
      if (loadRevision === revision) loadRevision++;
    };
  });

  /** Re-read page 1 in place: rows from later pages stay, scroll/selection/
   *  search untouched. `manual` skips the cache and surfaces a failure. */
  async function refresh(manual: boolean): Promise<void> {
    if (refreshing || loading || error) {
      if (manual && error) retryRev++;
      return;
    }
    const revision = loadRevision;
    const st = effState;
    refreshing = true;
    try {
      if (manual) prListCache.delete(prListKey(repoId, st, 1));
      const r = await api.get<PrListResp>(`/repos/${repoId}/prs?state=${st}&page=1&per_page=${PER_PAGE}`);
      prListCache.set(prListKey(repoId, st, 1), r);
      if (revision !== loadRevision) return;
      prs = mergeRefreshed(prs, r.items, page1Count);
      page1Count = r.items.length;
      if (page === 1) hasMore = r.has_more;
      lastUpdated = Date.now();
    } catch (e) {
      if (manual && revision === loadRevision) {
        moreError = '';
        // Keep the rows; say why the refresh failed under the toolbar.
        refreshError = `${causeOf(e)} ${loadErrorText(e)}`;
      }
    } finally {
      if (revision === loadRevision) refreshing = false;
    }
  }
  let refreshError = $state('');
  $effect(() => {
    if (lastUpdated) refreshError = '';
  });

  // Quiet background refresh: every 2 min while visible (pollWhileVisible
  // pays a tick missed while hidden on return), plus on window focus when the
  // last read is older than that.
  $effect(() => {
    void repoId;
    const due = () => Date.now() - untrack(() => lastUpdated) >= REFRESH_MS - 1000;
    const poller = pollWhileVisible(async () => {
      if (due()) await refresh(false);
    }, { ms: REFRESH_MS, immediate: false });
    const onFocus = () => {
      if (due()) void refresh(false);
    };
    window.addEventListener('focus', onFocus);
    return () => {
      poller.stop();
      window.removeEventListener('focus', onFocus);
    };
  });

  async function loadMore(): Promise<void> {
    if (loadingMore || !hasMore) return;
    const revision = loadRevision;
    loadingMore = true;
    moreError = '';
    try {
      const next = page + 1;
      const r = await api.get<PrListResp>(`/repos/${repoId}/prs?state=${effState}&page=${next}&per_page=${PER_PAGE}`);
      if (revision !== loadRevision) return;
      const seen = new Set(prs.map((p) => p.number));
      prs = [...prs, ...r.items.filter((p) => !seen.has(p.number))];
      hasMore = r.has_more;
      page = next;
    } catch (e) {
      if (revision !== loadRevision) return;
      moreError = loadErrorText(e);
    } finally {
      if (revision === loadRevision) loadingMore = false;
    }
  }

  // The detail pane read a fresher copy: fold its summary fields into the row.
  $effect(() => {
    const p = patch;
    if (!p) return;
    const i = prs.findIndex((r) => r.number === p.number);
    if (i < 0) return;
    const row = prs[i];
    const next: PrSummary = {
      ...row,
      title: p.title,
      author: p.author || row.author,
      state: p.state,
      source_branch: p.source_branch,
      target_branch: p.target_branch,
      updated_at: p.updated_at,
      created_at: p.created_at ?? row.created_at,
      url: p.url || row.url,
      draft: p.draft ?? row.draft,
      ci_status: p.ci_status ?? row.ci_status,
      labels: p.labels ?? row.labels,
      head_sha: p.head_sha ?? row.head_sha,
    };
    if (JSON.stringify(next) === JSON.stringify(row)) return;
    const copy = prs.slice();
    copy[i] = next;
    prs = copy;
  });

  // ── Identity ("Authored by you") ────────────────────────────────────────
  myGitHandles.load();
  const myHandles = $derived(myGitHandles.handles);

  // ── Lazy +/- counts ─────────────────────────────────────────────────────
  let stats = $state.raw<PrStatsLoader | null>(null);
  $effect(() => {
    const l = new PrStatsLoader(repoId);
    stats = l;
    return () => l.dispose();
  });
  const sizeOf = (p: PrSummary): number | null => {
    const s = stats?.get(p);
    return s ? s.add + s.del : null;
  };
  const anyStats = $derived(stats ? stats.stats.size > 0 : false);

  // ── View: filter → sort → group ─────────────────────────────────────────
  const visible = $derived.by(() => {
    const q = parsed;
    const pf = prefs;
    const handles = myHandles;
    return prs.filter((p) => {
      if (!matchesPr(p, q, handles)) return false;
      if (pf.draft === 'hide' && p.draft) return false;
      if (pf.draft === 'only' && !p.draft) return false;
      if (pf.ci && ciOf(p) !== pf.ci) return false;
      if (pf.author && p.author !== pf.author) return false;
      if (pf.mine && !isMine(p, handles)) return false;
      return true;
    });
  });
  const groups: PrGroup[] = $derived(groupPrs(sortPrs(visible, prefs.sort, sizeOf), myHandles));
  const folded = $derived(new Set(prefs.folded));
  const flat = $derived(groups.flatMap((g) => (folded.has(g.id) ? [] : g.items)));
  const activeFilterCount = $derived(activeFilters(prefs));
  const STATE_LABEL: Record<PrState, string> = { open: 'Open', merged: 'Merged', declined: 'Declined', all: 'All' };
  const filtering = $derived(hasClientFilter(parsed) || activeFilterCount > 0);
  const openSet = $derived(new Set(openNumbers));

  function toggleGroup(id: string): void {
    const s = new Set(prefs.folded);
    if (s.has(id)) s.delete(id);
    else s.add(id);
    setPrefs({ folded: [...s] });
  }

  // ── Menus ───────────────────────────────────────────────────────────────
  function sortMenu(e: MouseEvent): void {
    ctxMenu.show(e, sortItems(prefs, anyStats, setPrefs));
  }

  function filterMenu(e: MouseEvent): void {
    const trigger = e.currentTarget as HTMLElement;
    const authors = [...new Set(prs.map((p) => p.author).filter(Boolean))].sort((x, y) => x.localeCompare(y));
    const pickAuthor = () =>
      ctxMenu.showAt(trigger, authorItems(prefs, authors, setPrefs), {
        filter: authors.length > 8,
        filterPlaceholder: 'Filter authors…',
        maxVisible: 40,
        align: 'end',
      });
    ctxMenu.show(e, filterItems(prefs, { hasHandles: myHandles.size > 0 || prs.some((p) => p.is_mine), authors, pickAuthor }, setPrefs));
  }

  // ── Opening + selection ─────────────────────────────────────────────────
  const selected = new SvelteSet<number>();
  let anchor: number | null = null;

  function open(pr: PrSummary, background: boolean): void {
    if (onopen) onopen(pr, { background });
    else router.go(`git/${repoId}/pr/${pr.number}`);
  }

  function selectRange(to: number): void {
    const order = flat.map((p) => p.number);
    const a = anchor !== null ? order.indexOf(anchor) : -1;
    const b = order.indexOf(to);
    if (a < 0 || b < 0) {
      selected.add(to);
    } else {
      for (let i = Math.min(a, b); i <= Math.max(a, b); i++) selected.add(order[i]);
    }
    anchor = to;
  }

  function toggleSelect(pr: PrSummary, shift: boolean): void {
    if (shift) {
      selectRange(pr.number);
      return;
    }
    if (selected.has(pr.number)) selected.delete(pr.number);
    else selected.add(pr.number);
    anchor = pr.number;
  }

  function onRowClick(e: MouseEvent, pr: PrSummary): void {
    if (e.shiftKey) {
      e.preventDefault();
      selectRange(pr.number);
      return;
    }
    open(pr, e.metaKey || e.ctrlKey);
  }

  function openSelected(): void {
    const rows = flat.filter((p) => selected.has(p.number));
    if (rows.length === 0) return;
    if (onopenmany) onopenmany(rows);
    else if (onopen) rows.forEach((p, i) => onopen(p, { background: i > 0 }));
    else open(rows[0], false);
    selected.clear();
  }

  // ↑/↓ walk the rows' open buttons; Enter opens (⌘Enter: background).
  function arrowKeys(list: HTMLElement): () => void {
    const h = (e: KeyboardEvent) => onListKey(e, list);
    list.addEventListener('keydown', h);
    return () => list.removeEventListener('keydown', h);
  }
  function onListKey(e: KeyboardEvent, list: HTMLElement): void {
    if (e.key !== 'ArrowDown' && e.key !== 'ArrowUp') return;
    const btns = [...list.querySelectorAll<HTMLButtonElement>('.pr-open')];
    const i = btns.indexOf(document.activeElement as HTMLButtonElement);
    const next = btns[Math.max(0, Math.min(btns.length - 1, i < 0 ? 0 : i + (e.key === 'ArrowDown' ? 1 : -1)))];
    if (next) {
      e.preventDefault();
      next.focus();
    }
  }
  function onOpenKey(e: KeyboardEvent, pr: PrSummary): void {
    if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) {
      e.preventDefault();
      open(pr, true);
    }
  }

  function clearSearch(): void {
    query = '';
    parsedSource = '';
  }
</script>

<div class="prlist">
  <div class="pr-toolbar">
    <div class="pr-search-wrap">
      <span class="pr-search-ico" aria-hidden="true"><Icon name="search" size={13} /></span>
      <input
        dir="auto"
        class="input pr-search"
        type="search"
        bind:value={query}
        placeholder="Search or label:bug"
        aria-label="Filter pull requests"
        title="Text, #number, author:, label:, base:, head:, is:open|merged|declined|draft|mine, ci:failing"
        onkeydown={(e) => {
          if (e.key === 'Escape' && query) {
            e.preventDefault();
            clearSearch();
          }
        }}
      />
    </div>
    <div class="pr-tools">
      <button class="btn small pr-tool" aria-haspopup="menu" onclick={sortMenu} title="Sort: {SORT_LABEL[prefs.sort]}" aria-label="Sort pull requests ({SORT_LABEL[prefs.sort]})">
        <Icon name="swap" size={13} /><span class="pr-tool-label">Sort</span>
      </button>
      <button class="btn small pr-tool" class:on={activeFilterCount > 0} aria-haspopup="menu" onclick={filterMenu}
        title={activeFilterCount > 0 ? `${plural(activeFilterCount, 'filter')} on` : 'Filter pull requests'}
        aria-label={activeFilterCount > 0 ? `Filters (${activeFilterCount} on)` : 'Filters'}>
        <Icon name="filter" size={13} /><span class="pr-tool-label">Filters</span>
        {#if activeFilterCount > 0}<span class="pr-badge">{activeFilterCount}</span>{/if}
      </button>
      <button class="btn small ghost pr-icon" aria-busy={refreshing} onclick={() => void refresh(true)}
        title={lastUpdated ? `Refresh · updated ${rel(lastUpdated)}` : 'Refresh'} aria-label="Refresh pull requests">
        {#if refreshing}<span class="spinner" style="--spinner-size: 12px" aria-hidden="true"></span>{:else}<Icon name="refresh" size={13} />{/if}
      </button>
      <!-- While the list is empty the empty state carries the one "New" CTA. -->
      {#if loading || error || prs.length > 0}
        <button class="btn small ghost pr-icon" onclick={() => (createOpen = true)} title="New pull request" aria-label="New pull request">
          <Icon name="plus" size={14} />
        </button>
      {/if}
    </div>
  </div>

  <div class="pr-subbar">
    <div class="segmented pr-states" role="group" aria-label="Pull request state">
      {#each states as s (s)}
        <button
          class:active={effState === s}
          aria-pressed={effState === s}
          disabled={parsed.state !== null}
          title={parsed.state !== null ? 'Set by is: in the search' : undefined}
          onclick={() => setPrefs({ state: s })}
        >{STATE_LABEL[s]}</button>
      {/each}
    </div>
    <span class="grow"></span>
    {#if lastUpdated && !loading}
      <span class="pr-updated-hint dim" title={new Date(lastUpdated).toLocaleString()}>
        {refreshing ? 'Refreshing…' : `Updated ${rel(lastUpdated)}`}
      </span>
    {/if}
  </div>
  <!-- Always mounted: a status region inserted with its text is usually silent. -->
  <div class="pr-refresh-err" role="status" class:empty={!refreshError}>{refreshError ? `Couldn’t refresh: ${refreshError}` : ''}</div>

  {#if selected.size > 0}
    <div class="pr-selbar" role="region" aria-label="Selected pull requests">
      <span>{selected.size} selected</span>
      <span class="grow"></span>
      <button class="btn small ghost" onclick={() => selected.clear()}>Clear</button>
      <button class="btn small primary" onclick={openSelected}>
        Open {selected.size === 1 ? '1' : selected.size} together
      </button>
    </div>
  {/if}

  <div class="pr-scroll">
    {#if loading}
      <Skeleton rows={6} height={40} />
    {:else if error}
      <LoadState what="pull requests" error={`${errorCause} ${error}`} empty onretry={() => retryRev++} />
    {:else if prs.length === 0}
      <EmptyState
        icon="pr"
        title={effState === 'all' ? 'No pull requests' : `No ${effState} pull requests`}
        body="Open one from your current branch, or pick another state above."
        actionLabel="New pull request"
        actionIcon="pr"
        onaction={() => (createOpen = true)}
      />
    {:else}
      <div class="pr-rows" role="list" aria-label="Pull requests" {@attach arrowKeys}>
        {#each groups as g (g.id)}
          {#if g.id !== 'all'}
            <button class="pr-group" aria-expanded={!folded.has(g.id)} onclick={() => toggleGroup(g.id)}>
              <Icon name={folded.has(g.id) ? 'chevronRight' : 'chevronDown'} size={12} />
              <Icon name={g.id === 'mine' ? 'edit' : 'people'} size={12} />
              <span>{g.label}</span>
              <span class="pr-group-count">{g.items.length}</span>
              <span class="pr-group-rule" aria-hidden="true"></span>
            </button>
          {/if}
          {#if !folded.has(g.id)}
            {#each g.items as pr (pr.number)}
              <PrListRow
                {pr}
                stat={stats?.get(pr) ?? null}
                active={activeNumber === pr.number}
                selected={selected.has(pr.number)}
                selecting={selected.size > 0}
                openTab={openSet.has(pr.number)}
                observe={stats ? stats.observe(pr) : null}
                ontoggle={(shift) => toggleSelect(pr, shift)}
                onrowclick={(e) => onRowClick(e, pr)}
                onopen={(background) => open(pr, background)}
              />
            {/each}
          {/if}
        {/each}
        {#if visible.length === 0}
          <div class="pr-nomatch">
            <span class="dim">No pull request matches {query.trim() ? `“${query.trim()}”` : 'these filters'}.</span>
            {#if query.trim()}<button class="btn small ghost" onclick={clearSearch}>Clear search</button>{/if}
            {#if activeFilterCount > 0}
              <button class="btn small ghost" onclick={() => setPrefs({ ...CLEARED_FILTERS })}>Clear filters</button>
            {/if}
          </div>
        {/if}
      </div>
      {#if hasMore}
        <div class="pr-more">
          <button class="btn small" disabled={loadingMore} onclick={loadMore} aria-describedby={moreError ? moreErrId : undefined}>
            {loadingMore ? 'Loading more pull requests…' : moreError ? 'Retry' : 'Load more'}
          </button>
          {#if filtering}<span class="dim pr-more-hint">Search covers the {prs.length} loaded</span>{/if}
          <!-- Always mounted: a status region inserted with its text is usually silent. -->
          <span id={moreErrId} class="pr-more-err" role="status">{moreError ? `Couldn’t load more: ${moreError}` : ''}</span>
        </div>
      {/if}
    {/if}
  </div>
</div>

{#if createOpen}
  <CreatePr
    {repoId}
    onclose={() => (createOpen = false)}
    oncreated={(pr) => {
      createOpen = false;
      invalidatePr(repoId);
      retryRev++;
      if (onopen) onopen(pr, { background: false });
      else router.go(`git/${repoId}/pr/${pr.number}`);
    }}
  />
{/if}

<style>
  .prlist {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
    min-width: 0;
  }
  .pr-toolbar {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 10px 12px 6px;
  }
  .pr-search-wrap {
    position: relative;
    flex: 1 1 auto;
    min-width: 0;
  }
  .pr-search-ico {
    position: absolute;
    inset-inline-start: 8px;
    top: 50%;
    transform: translateY(-50%);
    color: var(--text-dim);
    display: inline-flex;
    pointer-events: none;
  }
  .pr-search {
    width: 100%;
    height: 28px;
    padding-inline-start: 28px;
    font-size: var(--fs-s);
  }
  .pr-tools {
    display: flex;
    align-items: center;
    gap: 4px;
    flex: none;
  }
  .pr-tool {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    height: 28px;
  }
  .pr-tool.on {
    color: var(--accent-text);
    border-color: color-mix(in srgb, var(--accent) 45%, var(--border));
  }
  .pr-badge {
    min-width: 16px;
    height: 16px;
    padding: 0 4px;
    border-radius: 999px;
    background: var(--accent-solid);
    color: var(--accent-contrast);
    font-size: var(--fs-xs);
    line-height: 16px;
    text-align: center;
  }
  .pr-icon {
    width: 28px;
    height: 28px;
    padding: 0;
    display: inline-flex;
    align-items: center;
    justify-content: center;
  }
  /* Narrow pane: Sort / Filters collapse to their icons. */
  @container (max-width: 360px) {
    .pr-tool-label {
      display: none;
    }
  }
  .prlist {
    container-type: inline-size;
  }
  .pr-subbar {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 0 12px 8px;
    border-block-end: 1px solid var(--border);
  }
  .pr-states > button:disabled {
    cursor: default;
  }
  .pr-updated-hint {
    font-size: var(--fs-xs);
    white-space: nowrap;
  }
  .pr-refresh-err.empty {
    padding: 0;
  }
  .pr-refresh-err {
    padding: 6px 12px;
    font-size: var(--fs-xs);
    color: var(--danger);
  }
  .pr-selbar {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 6px 12px;
    background: var(--accent-soft);
    font-size: var(--fs-s);
    border-block-end: 1px solid var(--border);
  }
  .pr-scroll {
    flex: 1 1 auto;
    min-height: 0;
    overflow-y: auto;
    padding: 4px 6px 12px;
  }
  .pr-rows {
    display: flex;
    flex-direction: column;
  }
  .pr-group {
    display: flex;
    align-items: center;
    gap: 6px;
    width: 100%;
    padding: 10px 8px 4px;
    border: none;
    background: none;
    color: var(--text-dim);
    font-size: var(--fs-xs);
    cursor: pointer;
    text-align: start;
  }
  .pr-group:hover {
    color: var(--text);
  }
  .pr-group-count {
    font-variant-numeric: tabular-nums;
  }
  .pr-group-rule {
    flex: 1;
    height: 1px;
    background: var(--border);
  }
  .pr-nomatch {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 8px;
    padding: 14px 8px;
    font-size: var(--fs-s);
  }
  .pr-more {
    display: flex;
    align-items: center;
    justify-content: center;
    flex-wrap: wrap;
    gap: 10px;
    margin-top: 10px;
  }
  .pr-more-hint {
    font-size: var(--fs-xs);
  }
  .pr-more-err {
    color: var(--danger);
    font-size: var(--fs-s);
  }

  /* ── Mobile + tablet (≤1024px): full-width pane, 32px+ touch targets,
     the checkbox always reachable (no hover), legible text. ── */
  @media (max-width: 1024px) {
    .pr-toolbar {
      flex-wrap: wrap;
    }
    .pr-search-wrap {
      flex-basis: 100%;
    }
    .pr-search {
      height: 34px;
      font-size: var(--fs-m);
    }
    .pr-tool,
    .pr-icon {
      height: 34px;
    }
    .pr-icon {
      width: 34px;
    }
    .pr-states > button {
      height: 30px;
      padding: 0 10px;
    }
  }
</style>
