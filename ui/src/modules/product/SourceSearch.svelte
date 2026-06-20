<script lang="ts">
  // SourceSearch — unified Jira issue / Confluence page picker.
  // Mirrors the structure and CSS of JiraIssuePicker.svelte.
  import { api } from '../../lib/api/client';
  import type { IssueProject, IssueSummary } from '../../lib/api/types';
  import type { ConfluenceSpace, ConfluencePageSummary } from './types';
  import { toasts } from '../../lib/toast.svelte';
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

  // --- load projects / spaces when accountId or sourceKind changes ---
  $effect(() => {
    const aid = accountId;
    const kind = sourceKind;
    if (!aid) return;
    resetSearch();
    if (kind === 'jira') {
      void loadProjects(aid).then(() => {
        // Fire an empty-query search immediately so the picker shows recent issues
        // even before the user types anything.
        void search('', 0, false);
      });
    } else {
      void loadSpaces(aid);
    }
  });

  function resetSearch(): void {
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
    projectsLoading = true;
    projects = [];
    try {
      projects = await api.get<IssueProject[]>(
        `/issue/projects?account_id=${encodeURIComponent(aid)}`,
      );
    } catch {
      projects = [];
    } finally {
      projectsLoading = false;
    }
  }

  async function loadSpaces(aid: string): Promise<void> {
    spacesLoading = true;
    spaces = [];
    try {
      spaces = await api.get<ConfluenceSpace[]>(
        `/issue/confluence/spaces?account_id=${encodeURIComponent(aid)}`,
      );
    } catch {
      spaces = [];
    } finally {
      spacesLoading = false;
    }
  }

  // --- reset results when project/space selector changes ---
  $effect(() => {
    const proj = selectedProjectKey;
    const space = selectedSpaceKey;
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
        );
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
        confluenceResults = await api.get<ConfluencePageSummary[]>(
          `/issue/confluence/search?account_id=${encodeURIComponent(accountId)}&q=${encodeURIComponent(q)}${spaceParam}`,
        );
      }
    } catch (e) {
      toasts.error('Search failed', e instanceof Error ? e.message : String(e));
      if (!append) {
        jiraResults = [];
        confluenceResults = [];
      }
    } finally {
      searching = false;
      loadingMore = false;
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
      <div class="picker-loading">Loading projects…</div>
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
      <div class="picker-loading">Loading spaces…</div>
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

<!-- Search input -->
<div class="picker-field">
  <label class="picker-label" for="ss-query">
    {sourceKind === 'jira' ? 'Search issues' : 'Search pages'}
  </label>
  <input
    id="ss-query"
    class="picker-input"
    bind:value={query}
    oninput={onQueryInput}
    placeholder={searchPlaceholder}
    spellcheck="false"
    autocomplete="off"
  />
</div>

<!-- Results -->
<div class="picker-results">
  {#if searching}
    <Skeleton rows={3} height={44} />
  {:else if !hasResults && query.trim() !== ''}
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
        {loadingMore ? 'Loading…' : 'Load more'}
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
    gap: 3px;
    margin-bottom: 10px;
  }
  .picker-label {
    font-size: 11px;
    color: var(--text-dim);
    font-weight: 500;
  }
  .picker-select,
  .picker-input {
    width: 100%;
    background: var(--input-bg, var(--surface-raised));
    border: 1px solid var(--border);
    border-radius: var(--radius-s, 4px);
    color: var(--text);
    font-size: 12.5px;
    padding: 4px 8px;
    box-sizing: border-box;
  }
  .picker-loading {
    font-size: 11.5px;
    color: var(--text-dim);
    padding: 4px 0;
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
    padding: 9px 10px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: transparent;
    cursor: pointer;
    text-align: left;
    transition: background 120ms ease-out, border-color 120ms ease-out;
    width: 100%;
  }
  .issue-row:hover {
    background: color-mix(in srgb, var(--accent) 10%, transparent);
    border-color: color-mix(in srgb, var(--accent) 40%, transparent);
  }
  .issue-left {
    display: flex;
    align-items: baseline;
    gap: 8px;
    min-width: 0;
  }
  .issue-key {
    font-size: 12px;
    font-weight: 700;
    font-family: var(--font-mono);
    color: var(--accent);
    flex-shrink: 0;
  }
  .issue-summary {
    font-size: 12.5px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    color: var(--text);
  }
  .status-chip {
    flex-shrink: 0;
    font-size: 10px;
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }
  .no-results {
    padding: 16px;
    text-align: center;
    font-size: 12.5px;
  }
  .load-more-btn {
    width: 100%;
    padding: 7px 0;
    margin-top: 2px;
    font-size: 12px;
    font-weight: 600;
    color: var(--accent);
    background: transparent;
    border: 1px dashed color-mix(in srgb, var(--accent) 40%, transparent);
    border-radius: var(--radius-s);
    cursor: pointer;
    transition: background 120ms ease-out;
  }
  .load-more-btn:hover:not(:disabled) {
    background: color-mix(in srgb, var(--accent) 8%, transparent);
  }
  .load-more-btn:disabled {
    opacity: 0.55;
    cursor: default;
  }
</style>
