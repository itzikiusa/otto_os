<script lang="ts">
  // SourceSearch — unified Jira issue / Confluence page picker.
  // Mirrors the structure and CSS of JiraIssuePicker.svelte.
  import { api } from '../../lib/api/client';
  import type { IssueProject, IssueSummary } from '../../lib/api/types';
  import type { ConfluenceSpace, ConfluencePageSummary } from './types';
  import { loadErrorText } from '../../lib/loadError';
  import Skeleton from '../../lib/components/Skeleton.svelte';

  interface PickedItem {
    key: string;
    label: string;
  }

  interface Props {
    accountId: string;
    sourceKind: 'jira' | 'confluence';
    onpick: (sel: PickedItem) => void;
  }
  let { accountId, sourceKind, onpick }: Props = $props();

  // --- Jira state ---
  let projects: IssueProject[] = $state([]);
  let projectsLoading = $state(false);
  let selectedProjectKey = $state('');

  // --- Confluence state ---
  let spaces: ConfluenceSpace[] = $state([]);
  let spacesLoading = $state(false);
  let selectedSpaceKey = $state('');

  // --- shared search state ---
  let query = $state('');
  let jiraResults: IssueSummary[] = $state([]);
  let confluenceResults: ConfluencePageSummary[] = $state([]);
  let searching = $state(false);
  let loadingMore = $state(false);
  // Tracks how many Jira results have been fetched so far (cursor for "load more").
  let jiraOffset = $state(0);
  // Whether more Jira results are likely available (last page returned a full 25).
  let jiraHasMore = $state(false);
  let debounceTimer: ReturnType<typeof setTimeout> | null = null;
  // Only the newest search may land: a slow earlier query (or a multi-second
  // "load more") must never overwrite / append to a newer one. The superseded
  // request is aborted too, so it stops holding a webview connection.
  let searchError = $state('');
  let filterError = $state('');
  let searched = $state(false);
  let retrySearch: (() => void) | null = null;
  let filterSeq = 0;
  let searchSeq = 0;
  let searchCtl: AbortController | null = null;
  $effect(() => () => searchCtl?.abort());

  // --- load projects / spaces when accountId or sourceKind changes ---
  $effect(() => {
    const aid = accountId;
    const kind = sourceKind;
    if (!aid) return;
    resetSearch();
    if (kind === 'jira') {
      // The empty-query "recent issues" search is fired by the filter effect
      // below (it re-runs on account/kind too) — firing it here as well made
      // two identical Jira searches on every open (backlog B6 / SE-11).
      void loadProjects(aid);
    } else {
      void loadSpaces(aid);
    }
  });

  function resetSearch(): void {
    ++searchSeq; searchCtl?.abort();
    searching = false; loadingMore = false; searchError = ''; searched = false; retrySearch = null;
    query = '';
    jiraResults = [];
    confluenceResults = [];
    jiraOffset = 0;
    jiraHasMore = false;
    selectedProjectKey = '';
    selectedSpaceKey = '';
    if (debounceTimer) { clearTimeout(debounceTimer); debounceTimer = null; }
  }

  async function loadProjects(aid: string): Promise<void> {
    const seq = ++filterSeq;
    const current = () => seq === filterSeq && accountId === aid && sourceKind === 'jira';
    filterError = '';
    projectsLoading = true;
    projects = [];
    try {
      const rows = await api.get<IssueProject[]>(
        `/issue/projects?account_id=${encodeURIComponent(aid)}`,
      );
      if (current()) projects = rows;
    } catch (e) {
      if (current()) filterError = loadErrorText(e);
    } finally {
      if (current()) projectsLoading = false;
    }
  }

  async function loadSpaces(aid: string): Promise<void> {
    const seq = ++filterSeq;
    const current = () => seq === filterSeq && accountId === aid && sourceKind === 'confluence';
    filterError = '';
    spacesLoading = true;
    spaces = [];
    try {
      const rows = await api.get<ConfluenceSpace[]>(
        `/issue/confluence/spaces?account_id=${encodeURIComponent(aid)}`,
      );
      if (current()) spaces = rows;
    } catch (e) {
      if (current()) filterError = loadErrorText(e);
    } finally {
      if (current()) spacesLoading = false;
    }
  }

  // --- reset results when project/space selector changes ---
  $effect(() => {
    const proj = selectedProjectKey;
    const space = selectedSpaceKey;
    ++searchSeq; searchCtl?.abort();
    searching = false; loadingMore = false; searchError = ''; searched = false; retrySearch = null;
    jiraResults = [];
    confluenceResults = [];
    jiraOffset = 0;
    jiraHasMore = false;
    query = '';
    // Reload with the new project/space filter.
    if (accountId && sourceKind === 'jira') {
      void search('', 0, false, proj || undefined);
    } else if (accountId && sourceKind === 'confluence' && space) {
      // Confluence: wait for user to type (spaces are wide).
    }
  });

  // --- search ---
  function onQueryInput(): void {
    if (debounceTimer) clearTimeout(debounceTimer);
    const q = query.trim();
    if (!accountId) return;
    // Empty query re-triggers the recency default; no early return.
    debounceTimer = setTimeout(() => void search(q, 0, false), 350);
  }

  /**
   * Perform a search.
   *
   * @param q - The search string (empty = recency default for Jira).
   * @param startAt - Cursor offset for pagination.
   * @param append - When true, new results are appended (load-more); otherwise replaces.
   * @param projectOverride - Optional project key override (used on filter change before
   *   selectedProjectKey reactive state has settled).
   */
  async function search(
    q: string,
    startAt: number,
    append: boolean,
    projectOverride?: string,
  ): Promise<void> {
    if (!accountId) return;
    searchCtl?.abort();
    const ctl = new AbortController();
    searchCtl = ctl;
    const seq = ++searchSeq;
    const aid = accountId, kind = sourceKind, project = projectOverride ?? selectedProjectKey, space = selectedSpaceKey;
    const current = () => seq === searchSeq && accountId === aid && sourceKind === kind;
    searchError = ''; retrySearch = null;
    if (append) {
      loadingMore = true;
    } else {
      searching = true;
    }
    try {
      if (sourceKind === 'jira') {
        const proj = projectOverride ?? selectedProjectKey;
        const projectParam = proj ? `&project=${encodeURIComponent(proj)}` : '';
        const page = await api.get<IssueSummary[]>(
          `/issue/search?account_id=${encodeURIComponent(accountId)}&q=${encodeURIComponent(q)}${projectParam}&start_at=${startAt}`,
          ctl.signal,
        );
        if (!current()) return;
        if (append) {
          jiraResults = [...jiraResults, ...page];
        } else {
          jiraResults = page;
        }
        jiraOffset = startAt + page.length;
        // If a full page came back, assume there may be more.
        jiraHasMore = page.length >= 25;
      } else {
        const spaceParam = selectedSpaceKey
          ? `&space=${encodeURIComponent(selectedSpaceKey)}`
          : '';
        const pages = await api.get<ConfluencePageSummary[]>(
          `/issue/confluence/search?account_id=${encodeURIComponent(accountId)}&q=${encodeURIComponent(q)}${spaceParam}`,
          ctl.signal,
        );
        if (!current()) return;
        confluenceResults = pages;
      }
      if (current()) searched = true;
    } catch (e) {
      if (!current() || ctl.signal.aborted) return;
      searchError = loadErrorText(e);
      retrySearch = () => {
        if (accountId === aid && sourceKind === kind && selectedSpaceKey === space && selectedProjectKey === project) void search(q, startAt, append, project);
      };
      if (!append) {
        jiraResults = [];
        confluenceResults = [];
      }
    } finally {
      if (current()) {
        searching = false;
        loadingMore = false;
        searchCtl = null;
      }
    }
  }

  function loadMoreJira(): void {
    void search(query.trim(), jiraOffset, true);
  }

  function pickIssue(issue: IssueSummary): void {
    onpick({ key: issue.key, label: `${issue.key} — ${issue.summary}` });
  }

  function pickPage(page: ConfluencePageSummary): void {
    onpick({ key: page.id, label: page.title });
  }

  const searchPlaceholder = $derived(
    sourceKind === 'jira'
      ? (selectedProjectKey ? 'Number (5218) or name…' : 'PROJ-123, number, or text…')
      : 'Page title or id…',
  );

  const hasResults = $derived(
    sourceKind === 'jira' ? jiraResults.length > 0 : confluenceResults.length > 0,
  );

  const listLoading = $derived(
    sourceKind === 'jira' ? projectsLoading : spacesLoading,
  );

  // cleanup on destroy
  $effect(() => {
    return () => {
      if (debounceTimer) clearTimeout(debounceTimer);
    };
  });
</script>

<!-- Project / Space selector -->
{#if sourceKind === 'jira'}
  <div class="picker-field">
    <label class="picker-label" for="ss-project">Project</label>
    {#if listLoading}
      <Skeleton rows={1} height={27} label="projects" />
    {:else}
      <select id="ss-project" class="picker-select" bind:value={selectedProjectKey}>
        <option value="">All projects</option>
        {#each projects as p (p.key)}
          <option value={p.key}>{p.name} ({p.key})</option>
        {/each}
      </select>
    {/if}
  </div>
{:else}
  <div class="picker-field">
    <label class="picker-label" for="ss-space">Space</label>
    {#if listLoading}
      <Skeleton rows={1} height={27} label="spaces" />
    {:else}
      <select id="ss-space" class="picker-select" bind:value={selectedSpaceKey}>
        <option value="">All spaces</option>
        {#each spaces as s (s.key)}
          <option value={s.key}>{s.name} ({s.key})</option>
        {/each}
      </select>
    {/if}
  </div>
{/if}

{#if filterError}
  <div role="alert">Couldn’t load the filters. {filterError}</div>
  <button class="btn small" onclick={() => sourceKind === 'jira' ? void loadProjects(accountId) : void loadSpaces(accountId)}>Retry filters</button>
{/if}

<!-- Search input -->
<div class="picker-field">
  <label class="picker-label" for="ss-query">
    {sourceKind === 'jira' ? 'Search issues' : 'Search pages'}
  </label>
  <input dir="auto"
    id="ss-query"
    class="picker-input"
    bind:value={query}
    oninput={onQueryInput}
    placeholder={searchPlaceholder}
    spellcheck="false"
    autocomplete="off"
  />
</div>

{#if searchError}
  <div role="alert">Search failed. {searchError}</div>
  <button class="btn small" onclick={() => retrySearch?.()}>Retry search</button>
{/if}

<!-- Results -->
<div class="picker-results">
  {#if searching}
    <Skeleton rows={3} height={44} />
  {:else if !hasResults && searched && !searchError && query.trim() !== ''}
    <div class="no-results dim">No results found.</div>
  {:else if sourceKind === 'jira'}
    {#each jiraResults as issue (issue.key)}
      <button class="issue-row" onclick={() => pickIssue(issue)}>
        <div class="issue-left">
          <span class="issue-key">{issue.key}</span>
          <span class="issue-summary">{issue.summary}</span>
        </div>
        <span class="chip status-chip">{issue.status}</span>
      </button>
    {/each}
    {#if jiraHasMore}
      <button
        class="load-more-btn"
        onclick={loadMoreJira}
        disabled={loadingMore}
      >
        {loadingMore ? 'Loading more results…' : 'Load more'}
      </button>
    {/if}
  {:else}
    {#each confluenceResults as page (page.id)}
      <button class="issue-row" onclick={() => pickPage(page)}>
        <div class="issue-left">
          <span class="issue-summary">{page.title}</span>
        </div>
        <span class="chip status-chip">{page.space_key}</span>
      </button>
    {/each}
  {/if}
</div>

<style>
  .picker-field {
    display: flex;
    flex-direction: column;
    gap: 2px;
    margin-bottom: 10px;
  }
  .picker-label {
    font-size: var(--fs-xs);
    color: var(--text-dim);
    font-weight: 500;
  }
  .picker-select,
  .picker-input {
    width: 100%;
    background: var(--surface-2);
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    color: var(--text);
    font-size: var(--fs-s);
    padding: 4px 8px;
    box-sizing: border-box;
  }
  .picker-results {
    margin-top: 4px;
    display: flex;
    flex-direction: column;
    gap: 4px;
    max-height: 240px;
    overflow-y: auto;
  }
  .issue-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
    padding: 8px 10px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: transparent;
    cursor: pointer;
    text-align: start;
    transition: background var(--dur-fast) ease-out, border-color var(--dur-fast) ease-out;
    width: 100%;
  }
  .issue-row:hover {
    background: var(--accent-soft);
    border-color: var(--accent-line);
  }
  .issue-left {
    display: flex;
    align-items: baseline;
    gap: 8px;
    min-width: 0;
  }
  .issue-key {
    font-size: var(--fs-s);
    font-weight: 600;
    font-family: var(--font-mono);
    color: var(--accent-text);
    flex-shrink: 0;
  }
  .issue-summary {
    font-size: var(--fs-s);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    color: var(--text);
  }
  .status-chip {
    flex-shrink: 0;
    font-size: var(--fs-xs);
    text-transform: uppercase;
    letter-spacing: .06em;
  }
  .no-results {
    padding: 16px;
    text-align: center;
    font-size: var(--fs-s);
  }
  .load-more-btn {
    width: 100%;
    padding: 6px 0;
    margin-top: 2px;
    font-size: var(--fs-s);
    font-weight: 600;
    color: var(--accent-text);
    background: transparent;
    border: 1px dashed var(--accent-line);
    border-radius: var(--radius-s);
    cursor: pointer;
    transition: background var(--dur-fast) ease-out;
  }
  .load-more-btn:hover:not(:disabled) {
    background: var(--accent-faint);
  }
  .load-more-btn:disabled {
    opacity: 0.55;
    cursor: default;
  }
</style>
