<script lang="ts">
  // SourceSearch — unified tracker issue (Jira / Linear) / Confluence page picker.
  // Mirrors the structure and CSS of JiraIssuePicker.svelte.
  import { api } from '../../lib/api/client';
  import type { IssueProject, IssueProviderKind, IssueSummary, ListingPage } from '../../lib/api/types';
  import { truncatedNote, unwrapListing } from '../../lib/listingPage';
  import type { ConfluenceSpace, ConfluencePageSummary } from './types';
  import { loadErrorText } from '../../lib/loadError';
  import Skeleton from '../../lib/components/Skeleton.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import { ctxMenu } from '../../lib/contextmenu.svelte';
  import { untrack } from 'svelte';
  import { openExternal } from '../../lib/external';
  import {
    CQL_CHEATSHEET, CQL_DOCS, CQL_TEMPLATES, JQL_CHEATSHEET, JQL_DOCS, JQL_TEMPLATES, fillTemplate,
    LINEAR_CHEATSHEET, LINEAR_DOCS, LINEAR_TEMPLATES, fillLinearTemplate,
  } from './queryTemplates';

  interface PickedItem {
    key: string;
    label: string;
  }

  interface Props {
    accountId: string;
    sourceKind: 'jira' | 'confluence';
    /** The account's tracker: Linear's raw query is an IssueFilter JSON
     *  object and its "projects" are teams. */
    provider?: IssueProviderKind;
    onpick: (sel: PickedItem) => void;
  }
  let { accountId, sourceKind, provider = 'jira', onpick }: Props = $props();
  const linear = $derived(provider === 'linear' && sourceKind === 'jira');
  const projectNoun = $derived(linear ? 'team' : 'project');

  // --- Jira state ---
  let projects: IssueProject[] = $state([]);
  let projectsLoading = $state(false);
  /** The project / space list stopped at its page cap (S5-22). */
  let listTruncated = $state(false);
  let selectedProjectKey = $state('');

  // --- Confluence state ---
  let spaces: ConfluenceSpace[] = $state([]);
  let spacesLoading = $state(false);
  let selectedSpaceKey = $state('');

  // --- shared search state ---
  let query = $state('');
  // How the query is read: free text (Jira) / title (Confluence), title +
  // body (Confluence), or a raw JQL / CQL expression run verbatim. Raw modes
  // search on Enter — a half-typed expression is a guaranteed 400.
  type SearchMode = 'text' | 'body' | 'raw';
  let mode = $state<SearchMode>('text');
  const rawMode = $derived(mode === 'raw');
  let jiraResults: IssueSummary[] = $state([]);
  let confluenceResults: ConfluencePageSummary[] = $state([]);
  let searching = $state(false);
  let loadingMore = $state(false);
  // Tracks how many Jira results have been fetched so far (cursor for "load more").
  let jiraOffset = $state(0);
  // Whether more Jira results are likely available (last page returned a full 25).
  let jiraHasMore = $state(false);
  // The query + mode whose results are listed (the cursor for Load more).
  let lastRun: { q: string; mode: SearchMode; project: string } | null = null;
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
    mode = 'text';
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
    listTruncated = false;
    try {
      const page = unwrapListing(
        await api.get<ListingPage<IssueProject>>(
          `/issue/projects?account_id=${encodeURIComponent(aid)}&meta=1`,
        ),
      );
      if (current()) {
        projects = page.items;
        listTruncated = page.truncated;
      }
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
    listTruncated = false;
    try {
      const page = unwrapListing(
        await api.get<ListingPage<ConfluenceSpace>>(
          `/issue/confluence/spaces?account_id=${encodeURIComponent(aid)}&meta=1`,
        ),
      );
      if (current()) {
        spaces = page.items;
        listTruncated = page.truncated;
      }
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
      // untrack: search() reads the Match mode — switching it must not re-run
      // this reset (it wiped the typed query and fired an empty search).
      untrack(() => void search('', 0, false, proj || undefined));
    } else if (accountId && sourceKind === 'confluence' && space) {
      // Confluence: wait for user to type (spaces are wide).
    }
  });

  // --- search ---
  function onQueryInput(): void {
    if (debounceTimer) clearTimeout(debounceTimer);
    if (rawMode) return;
    const q = query.trim();
    if (!accountId) return;
    // Empty query re-triggers the recency default; no early return.
    debounceTimer = setTimeout(() => void search(q, 0, false), 350);
  }

  // Raw-mode help: examples to start from + a one-screen syntax sheet.
  let syntaxOpen = $state(false);
  let queryInput: HTMLInputElement | undefined = $state();
  const rawLang = $derived(linear ? 'Linear filter' : sourceKind === 'jira' ? 'JQL' : 'CQL');
  const rawDocs = $derived(linear ? LINEAR_DOCS : sourceKind === 'jira' ? JQL_DOCS : CQL_DOCS);
  const rawSheet = $derived(linear ? LINEAR_CHEATSHEET : sourceKind === 'jira' ? JQL_CHEATSHEET : CQL_CHEATSHEET);
  function showExamples(e: MouseEvent): void {
    const key = sourceKind === 'jira' ? selectedProjectKey : selectedSpaceKey;
    const list = linear ? LINEAR_TEMPLATES : sourceKind === 'jira' ? JQL_TEMPLATES : CQL_TEMPLATES;
    ctxMenu.show(e, list.map((t) => {
      const q = linear ? fillLinearTemplate(t.query, key) : fillTemplate(t.query, key);
      return {
        label: t.label,
        title: q,
        action: () => {
          query = q;
          // Placed, not run: most need a value edited (version, epic, label).
          queueMicrotask(() => queryInput?.focus());
        },
      };
    }));
  }

  function onQueryKey(e: KeyboardEvent): void {
    if (e.key !== 'Enter' || !rawMode || !accountId) return;
    e.preventDefault();
    const q = query.trim();
    if (q) void search(q, 0, false);
  }

  function onModeChange(): void {
    if (debounceTimer) clearTimeout(debounceTimer);
    const q = query.trim();
    // Switching INTO a raw mode leaves the results until Enter; switching out
    // re-runs the text search (empty = the Jira recency default).
    if (!rawMode && accountId && (q || sourceKind === 'jira')) void search(q, 0, false);
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
    runMode: SearchMode = mode,
  ): Promise<void> {
    if (!accountId) return;
    const raw = runMode === 'raw';
    // An empty raw query is not "recent issues" — it waits for Enter.
    if (raw && !q.trim()) return;
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
        const queryParam = raw
          ? `${linear ? 'filter' : 'jql'}=${encodeURIComponent(q)}`
          : `q=${encodeURIComponent(q)}${projectParam}`;
        const page = await api.get<IssueSummary[]>(
          `/issue/search?account_id=${encodeURIComponent(accountId)}&${queryParam}&start_at=${startAt}`,
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
        // Load more continues THIS query: raw modes don't search while
        // typing, so the box can already hold an unrun expression.
        lastRun = { q, mode: runMode, project };
      } else {
        const spaceParam = selectedSpaceKey
          ? `&space=${encodeURIComponent(selectedSpaceKey)}`
          : '';
        const queryParam = raw
          ? `cql=${encodeURIComponent(q)}`
          : `q=${encodeURIComponent(q)}${spaceParam}${runMode === 'body' ? '&mode=text' : ''}`;
        const pages = await api.get<ConfluencePageSummary[]>(
          `/issue/confluence/search?account_id=${encodeURIComponent(accountId)}&${queryParam}`,
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
        if (accountId === aid && sourceKind === kind && selectedSpaceKey === space && selectedProjectKey === project) void search(q, startAt, append, project, runMode);
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
    if (lastRun) void search(lastRun.q, jiraOffset, true, lastRun.project, lastRun.mode);
  }

  function pickIssue(issue: IssueSummary): void {
    onpick({ key: issue.key, label: `${issue.key} — ${issue.summary}` });
  }

  function pickPage(page: ConfluencePageSummary): void {
    onpick({ key: page.id, label: page.title });
  }

  const searchPlaceholder = $derived(
    rawMode
      ? (linear
        ? '{"state":{"type":{"eq":"started"}}} — Enter to run'
        : sourceKind === 'jira'
        ? 'fixVersion = "6.23.0" ORDER BY key — Enter to run'
        : 'space = DEV AND label = "release" — Enter to run')
      : sourceKind === 'jira'
        ? (selectedProjectKey ? 'Number (5218) or name…' : linear ? 'ENG-123 or text…' : 'PROJ-123, number, or text…')
        : mode === 'body' ? 'Words in the title or page body…' : 'Page title or id…',
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
    <label class="picker-label" for="ss-project">{linear ? 'Team' : 'Project'}</label>
    {#if listLoading}
      <Skeleton rows={1} height={27} label="{projectNoun}s" />
    {:else}
      <select id="ss-project" class="picker-select" bind:value={selectedProjectKey}>
        <option value="">All {projectNoun}s</option>
        {#each projects as p (p.key)}
          <option value={p.key}>{p.name} ({p.key})</option>
        {/each}
      </select>
      {#if listTruncated}
        <p class="trunc-note" role="note">{truncatedNote(`${projectNoun}s`, projects.length)}</p>
      {/if}
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
      {#if listTruncated}
        <p class="trunc-note" role="note">{truncatedNote('spaces', spaces.length)}</p>
      {/if}
    {/if}
  </div>
{/if}

{#if filterError}
  <div role="alert">Couldn’t load the filters. {filterError}</div>
  <button class="btn small" onclick={() => sourceKind === 'jira' ? void loadProjects(accountId) : void loadSpaces(accountId)}>Retry filters</button>
{/if}

<!-- Search input -->
<div class="picker-field">
  <div class="query-head">
    <label class="picker-label" for="ss-query">
      {sourceKind === 'jira' ? 'Search issues' : 'Search pages'}
    </label>
    <select
      class="mode-select"
      aria-label="Match"
      title="How the search box is read"
      bind:value={mode}
      onchange={onModeChange}
    >
      {#if sourceKind === 'jira'}
        <option value="text">Text or key</option>
        <option value="raw">{linear ? 'Linear filter' : 'JQL'}</option>
      {:else}
        <option value="text">Title</option>
        <option value="body">Title + body</option>
        <option value="raw">CQL</option>
      {/if}
    </select>
  </div>
  <input dir="auto"
    id="ss-query"
    class="picker-input"
    class:mono={rawMode}
    bind:this={queryInput}
    bind:value={query}
    oninput={onQueryInput}
    onkeydown={onQueryKey}
    placeholder={searchPlaceholder}
    spellcheck="false"
    autocomplete="off"
  />
  {#if rawMode}
    <div class="raw-help">
      <button type="button" class="help-btn" aria-haspopup="menu" onclick={showExamples}>
        <Icon name="note" size={12} /> Examples…
      </button>
      <button type="button" class="help-btn" aria-expanded={syntaxOpen} onclick={() => (syntaxOpen = !syntaxOpen)}>
        <Icon name={syntaxOpen ? 'chevronDown' : 'chevronRight'} size={12} /> {rawLang} syntax
      </button>
      <button type="button" class="help-btn" title="Open the {rawLang} reference in the browser"
        onclick={() => void openExternal(rawDocs)}>
        <Icon name="external" size={12} /> Reference
      </button>
    </div>
    {#if syntaxOpen}
      <dl class="cheatsheet" aria-label="{rawLang} cheat sheet">
        {#each rawSheet as [term, text] (term)}
          <dt>{term}</dt>
          <dd class="mono">{text}</dd>
        {/each}
      </dl>
    {/if}
    <p class="raw-note">
      Enter runs it as written — the {sourceKind === 'jira' ? projectNoun : 'space'} filter above only fills the examples.
    </p>
  {/if}
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
  .trunc-note {
    margin: 4px 0 0;
    font-size: var(--fs-xs);
    color: var(--warning);
  }
  .picker-field {
    display: flex;
    flex-direction: column;
    gap: 2px;
    margin-bottom: 10px;
  }
  .query-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
  }
  .mode-select {
    background: transparent;
    border: none;
    color: var(--text-dim);
    font-size: var(--fs-xs);
    padding: 0;
    cursor: pointer;
  }
  .mode-select:focus-visible {
    outline: 2px solid var(--accent-text);
    outline-offset: 1px;
  }
  .picker-input.mono {
    font-family: var(--font-mono);
  }
  .raw-help {
    display: flex;
    flex-wrap: wrap;
    gap: 2px 4px;
    margin-top: 4px;
  }
  .help-btn {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    height: 22px;
    padding: 0 6px;
    border: none;
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--accent-text);
    font-size: var(--fs-xs);
    cursor: pointer;
  }
  .help-btn:hover {
    background: var(--hover);
  }
  .help-btn:focus-visible {
    outline: 2px solid var(--accent-text);
  }
  .cheatsheet {
    display: grid;
    grid-template-columns: max-content 1fr;
    gap: 4px 10px;
    margin: 6px 0 0;
    padding: 8px 10px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--surface-2);
    font-size: var(--fs-xs);
  }
  .cheatsheet dt {
    color: var(--text-dim);
    font-weight: 600;
  }
  .cheatsheet dd {
    margin: 0;
    color: var(--text);
    overflow-wrap: anywhere;
  }
  .mono {
    font-family: var(--font-mono);
  }
  .raw-note {
    margin: 2px 0 0;
    font-size: var(--fs-xs);
    color: var(--text-dim);
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
    opacity: var(--disabled-opacity);
    cursor: default;
  }
</style>
