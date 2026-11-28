<script lang="ts">
  // PR list — the PR workspace's list pane (ListPane: "Pull requests" count,
  // + New, ⋯ = Sort / Refresh). Search takes free text + qualifiers
  // (pr-query.ts) beside the Filters menu,
  // "Needs you" first (prStatus.needsYou), lazy +/- counts per visible row, multi-select
  // "Open N together", and a quiet background refresh every 2 minutes that
  // never resets scroll, selection or the search.
  import { api, ApiError } from '../../lib/api/client';
  import type { GitAccount, PrListResp, PrState, PrSummary } from '../../lib/api/types';
  import { myGitHandles } from './myGitHandles.svelte';
  import { router } from '../../lib/router.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import CreatePr from './CreatePr.svelte';
  import { rel } from '../../lib/stores/now.svelte';
  import { loadErrorText } from '../../lib/loadError';
  import { plural } from '../../lib/plural';
  import { ctxMenu, type MenuItem } from '../../lib/contextmenu.svelte';
  import ListPane from '../../lib/components/side/ListPane.svelte';
  import { gitPhoneMenu } from './gitPhoneMenu.svelte';
  import { invalidatePr, prListCache, prListKey } from './pr-cache';
  import { accountCause, ciOf, groupPrs, hasClientFilter, isMine, matchesPr, mergeRefreshed, parsePrQuery, sortPrs, SORT_LABEL, type PrGroup } from './pr-query';
  import { needsYou } from './prStatus';
  import SideSection from '../../lib/components/side/SideSection.svelte';
  import { PrStatsLoader } from './pr-stats.svelte';
  import PrListRow from './PrListRow.svelte';
  import { CLEARED_FILTERS, DEFAULT_PREFS, activeFilters, authorItems, filterItems, listToken, prListUrl, readPrefs, sortItems, writePrefs, type PrListPrefs } from './pr-list-prefs';
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
    /** The rows in list order, once per repo when they first load. */
    onfirst?: (prs: PrSummary[]) => void;
    /** The repository has no pull request at all (every state, no filter): the page's EmptyState takes over. */
    onempty?: () => void;
  }
  let { repoId, activeNumber = null, openNumbers = [], onopen, onopenmany, patch = null, onfirst, onempty }: Props = $props();

  const PER_PAGE = 50;
  const REFRESH_MS = 2 * 60_000;

  // ── Per-repo preferences (sort, state, filters) ──────────
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
  /** The server list: the state, or the "review requested from you" one. */
  const listTok = $derived(listToken(effState, prefs.review));

  // ── Data ─────────────────────────────────────────────────────────────────
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
  let errorCause = $state('The provider is unreachable.');
  let createOpen = $state(false);
  let retryRev = $state(0);
  let loadRevision = 0;
  let lastRetryRev = 0;

  function causeOf(e: unknown): string {
    const msg = e instanceof Error ? e.message : '';
    // A forge quota refusal comes back as a 502 "<provider> rate limited — retry in Ns" (otto-git providers/client.rs).
    if (/\brate limited\b/i.test(msg)) return 'The provider rate-limited this request.';
    return accountCause(msg) ?? (e instanceof ApiError && (e.status === 401 || e.status === 403)
      ? 'The provider rejected the credentials.'
      : e instanceof ApiError && e.status === 404
        ? 'The repository wasn’t found on the provider.'
        : 'The provider is unreachable.');
  }
  /** The cause, then git's own words — unless the cause already says it all. */
  const failText = (cause: string, raw: string): string => (accountCause(raw) ? cause : `${cause} ${raw}`);

  $effect(() => {
    const id = repoId;
    const st = listTok;
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
      .get<PrListResp>(prListUrl(id, st, 1, PER_PAGE))
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
        // A missing token: name the account it belongs to.
        const msg = error;
        if (/token missing/i.test(msg)) {
          void api
            .get<GitAccount[]>('/git/accounts')
            .then((accs) => {
              if (active) errorCause = accountCause(msg, (aid) => accs.find((a) => a.id === aid)?.label || undefined) ?? errorCause;
            })
            .catch(() => {});
        }
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
    const st = listTok;
    refreshing = true;
    try {
      if (manual) prListCache.delete(prListKey(repoId, st, 1));
      const r = await api.get<PrListResp>(prListUrl(repoId, st, 1, PER_PAGE) + (manual ? '&fresh=true' : ''));
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
        refreshError = failText(causeOf(e), loadErrorText(e));
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
      const r = await api.get<PrListResp>(prListUrl(repoId, listTok, next, PER_PAGE));
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
      is_mine: p.is_mine ?? row.is_mine,
      review: p.review !== undefined ? p.review : row.review,
    };
    if (JSON.stringify(next) === JSON.stringify(row)) return;
    const copy = prs.slice();
    copy[i] = next;
    prs = copy;
    const key = prListKey(repoId, listTok, 1);
    const cached = prListCache.get(key);
    if (cached) {
      prListCache.set(key, {
        ...cached.value,
        items: cached.value.items.map((it) => (it.number === next.number ? next : it)),
      });
    }
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
  const needs = (p: PrSummary): boolean => needsYou(p, isMine(p, myHandles));
  const groups: PrGroup[] = $derived(groupPrs(sortPrs(visible, prefs.sort, sizeOf), needs));
  const flat = $derived(groups.flatMap((g) => g.items));
  let firstFor = '';
  $effect(() => {
    if (!onfirst || flat.length === 0 || firstFor === repoId) return;
    firstFor = repoId;
    const rows = flat;
    untrack(() => onfirst?.(rows));
  });
  const activeFilterCount = $derived(activeFilters(prefs));
  const filtering = $derived(hasClientFilter(parsed) || activeFilterCount > 0);
  const openSet = $derived(new Set(openNumbers));

  $effect(() => {
    if (!loading && !error && prs.length === 0 && effState === 'all' && !prefs.review) untrack(() => onempty?.());
  });

  // ── Menus ───────────────────────────────────────────────────────────────
  const paneMenu = (): MenuItem[] => [
    { label: `Sort: ${SORT_LABEL[prefs.sort]}`, icon: 'swap', children: sortItems(prefs, anyStats, setPrefs) },
    { label: 'Refresh', icon: 'refresh', hint: lastUpdated ? `updated ${rel(lastUpdated)}` : undefined, disabled: refreshing, action: () => void refresh(true) },
    ...gitPhoneMenu.items(),
  ];

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
    ctxMenu.show(e, filterItems(prefs, { hasHandles: myHandles.size > 0 || prs.some((p) => p.is_mine), authors, pickAuthor, stateLocked: parsed.state !== null }, setPrefs));
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
    // While picking several, a click (a tap) adds or removes the row.
    if (selected.size > 0 && !e.metaKey && !e.ctrlKey) {
      toggleSelect(pr, e.shiftKey);
      return;
    }
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

  // The pane walks the rows (use:listNav on .tr-main); ⌘Enter opens one in the background.
  function bgOpenKey(list: HTMLElement): () => void {
    list.addEventListener('keydown', onListKey);
    return () => list.removeEventListener('keydown', onListKey);
  }
  function onListKey(e: KeyboardEvent): void {
    if (e.key !== 'Enter' || !(e.metaKey || e.ctrlKey)) return;
    const n = Number((e.target as HTMLElement).closest<HTMLElement>('[data-pr]')?.dataset.pr);
    const pr = flat.find((p) => p.number === n);
    if (pr) {
      e.preventDefault();
      open(pr, true);
    }
  }
  function clearSearch(): void {
    query = '';
    parsedSource = '';
  }
</script>

{#snippet selBar()}
  <span class="pr-selcount">{selected.size} selected</span>
  <span class="grow"></span>
  <button class="btn small ghost" onclick={() => selected.clear()}>Clear</button>
  <button class="btn small primary" onclick={openSelected}>Open {selected.size} together</button>
{/snippet}

<ListPane
  count={loading || error ? null : visible.length}
  primary={{ icon: 'plus', label: 'New pull request', onclick: () => (createOpen = true) }}
  menu={paneMenu}
  selection={selected.size > 0 ? selBar : undefined}
  {loading}
  error={error ? failText(errorCause, error) : null}
  onretry={() => retryRev++}
  rows=".tr-main"
  testid="pr-list"
>
  {#snippet searchRow()}
    <div class="pr-search-row">
      <div class="side-search pr-search-field">
      <Icon name="search" size={14} />
      <input
        dir="auto"
        class="pr-search"
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
      <button class="icon-btn pr-filters" class:on={activeFilterCount > 0} aria-haspopup="menu" onclick={filterMenu}
        title={activeFilterCount > 0 ? `${plural(activeFilterCount, 'filter')} on` : 'Filter pull requests'}
        aria-label={activeFilterCount > 0 ? `Filters (${activeFilterCount} on)` : 'Filters'}>
        <Icon name="filter" size={14} />
        {#if activeFilterCount > 0}<span class="pr-badge">{activeFilterCount}</span>{/if}
      </button>
    </div>
  {/snippet}

  <!-- Always mounted: a status region inserted with its text is usually silent. -->
  <div class="pr-refresh-err" role="status" class:empty={!refreshError}>{refreshError ? `Couldn’t refresh: ${refreshError}` : ''}</div>

  {#if prs.length === 0 && prefs.review}
    <EmptyState icon="eye" title="No reviews requested from you" body="No open pull request asks you for a review right now."
      actionLabel="Show every pull request" actionIcon="pr" onaction={() => setPrefs({ review: false })} />
  {:else if prs.length === 0}
    <div class="pr-nomatch">
      <span class="dim">No {effState} pull requests.</span>
      <button class="btn small ghost" onclick={() => setPrefs({ state: 'all' })}>Show every state</button>
    </div>
  {:else}
    <div class="pr-rows" role="list" aria-label="Pull requests" {@attach bgOpenKey}>
      {#snippet rows(items: PrSummary[], strong: boolean)}
        {#each items as pr (pr.number)}
          <PrListRow
            {pr}
            stat={stats?.get(pr) ?? null}
            active={selected.size === 0 && activeNumber === pr.number}
            selected={selected.has(pr.number)}
            needs={strong}
            openTab={openSet.has(pr.number)}
            observe={stats ? stats.observe(pr) : null}
            ontoggle={(shift) => toggleSelect(pr, shift)}
            onrowclick={(e) => onRowClick(e, pr)}
            onopen={(background) => open(pr, background)}
          />
        {/each}
      {/snippet}
      <!-- Headings only when something needs you: "Needs you", then the rest. -->
      {#each groups as g (g.id)}
        {#if g.id === 'all'}
          {@render rows(g.items, false)}
        {:else}
          <SideSection title={g.label} count={g.items.length}>{@render rows(g.items, g.id === 'needs')}</SideSection>
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
        {#if moreError}
          <LoadState variant="compact" what="more pull requests" error={moreError} loading={loadingMore} empty onretry={loadMore} />
        {:else}
          <button class="btn small" disabled={loadingMore} onclick={loadMore}>
            {loadingMore ? 'Loading more pull requests…' : 'Load more'}
          </button>
        {/if}
        {#if filtering}<span class="dim pr-more-hint">Search covers the {prs.length} loaded</span>{/if}
      </div>
    {/if}
  {/if}
</ListPane>

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
  .pr-search-row {
    display: flex;
    align-items: center;
    gap: 4px;
  }
  .pr-search-field {
    flex: 1;
  }
  .pr-filters {
    position: relative;
    flex: none;
  }
  .pr-filters.on {
    color: var(--accent-text);
  }
  .pr-badge {
    position: absolute;
    inset-block-start: -2px;
    inset-inline-end: -2px;
    min-width: 14px;
    height: 14px;
    padding: 0 4px;
    border-radius: 999px;
    background: var(--accent-solid);
    color: var(--accent-contrast);
    font-size: var(--fs-xs);
    line-height: 14px;
    text-align: center;
  }
  .pr-refresh-err.empty {
    padding: 0;
  }
  .pr-refresh-err {
    padding: 6px 12px;
    font-size: var(--fs-xs);
    color: var(--danger);
  }
  .pr-selcount {
    font-size: var(--fs-s);
    font-weight: 600;
  }
  .pr-rows {
    display: flex;
    flex-direction: column;
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
</style>
