<script lang="ts">
  // Shared Jira issue picker block — used by both ReviewPanel and AttachIssue.
  // Handles account selection, project selection, and debounced search.
  import { api } from '../../lib/api/client';
  import { toastError } from '../../lib/toastError';
  import type { IssueAccount, IssueProject, IssueSummary, ListingPage } from '../../lib/api/types';
  import { truncatedNote, unwrapListing } from '../../lib/listingPage';
  import Skeleton from '../../lib/components/Skeleton.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import { router } from '../../lib/router.svelte';

  interface PickedIssue {
    account_id: string;
    key: string;
    summary: string;
  }

  interface Props {
    onpick: (issue: PickedIssue) => void;
  }
  let { onpick }: Props = $props();

  // --- state ---
  let accounts: IssueAccount[] = $state([]);
  let accountsLoading = $state(true);
  let selectedAccountId = $state('');

  let projects: IssueProject[] = $state([]);
  let projectsLoading = $state(false);
  /** The project list stopped at its page cap (S5-22). */
  let projectsTruncated = $state(false);
  let selectedProjectKey = $state(''); // '' = All projects

  let query = $state('');
  let results: IssueSummary[] = $state([]);
  let searching = $state(false);
  let debounceTimer: ReturnType<typeof setTimeout> | null = null;

  // --- account loading ---
  $effect(() => {
    void loadAccounts();
  });

  async function loadAccounts(): Promise<void> {
    accountsLoading = true;
    try {
      accounts = await api.get<IssueAccount[]>('/issue/accounts');
      if (accounts.length > 0) selectedAccountId = accounts[0].id;
    } catch (e) {
      toastError('Couldn’t load Jira accounts', e);
    } finally {
      accountsLoading = false;
    }
  }

  // --- project loading when account changes ---
  $effect(() => {
    // Track selectedAccountId reactively.
    const aid = selectedAccountId;
    if (!aid) return;
    void loadProjects(aid);
  });

  async function loadProjects(accountId: string): Promise<void> {
    projectsLoading = true;
    projects = [];
    projectsTruncated = false;
    selectedProjectKey = '';
    results = [];
    query = '';
    try {
      const page = unwrapListing(
        await api.get<ListingPage<IssueProject>>(
          `/issue/projects?account_id=${encodeURIComponent(accountId)}&meta=1`,
        ),
      );
      projects = page.items;
      projectsTruncated = page.truncated;
    } catch {
      // Non-fatal: user can still search without a project filter.
      projects = [];
    } finally {
      projectsLoading = false;
    }
  }

  // --- reset results when project changes ---
  $effect(() => {
    selectedProjectKey;
    results = [];
    query = '';
  });

  // --- search ---
  function onQueryInput(): void {
    if (debounceTimer) clearTimeout(debounceTimer);
    const q = query.trim();
    if (!q || !selectedAccountId) {
      results = [];
      return;
    }
    debounceTimer = setTimeout(() => void search(q), 350);
  }

  // Why the last search failed — shown in the results area with Retry (a toast
  // plus "No issues found." read as a real empty result).
  let searchError = $state<string | null>(null);
  let searchSeq = 0;

  async function search(q: string): Promise<void> {
    if (!selectedAccountId) return;
    const seq = ++searchSeq;
    searching = true;
    searchError = null;
    try {
      const projectParam = selectedProjectKey
        ? `&project=${encodeURIComponent(selectedProjectKey)}`
        : '';
      const found = await api.get<IssueSummary[]>(
        `/issue/search?account_id=${encodeURIComponent(selectedAccountId)}&q=${encodeURIComponent(q)}${projectParam}`,
      );
      // A slower, older search must not overwrite the newer one's results.
      if (seq === searchSeq) results = found;
    } catch (e) {
      if (seq !== searchSeq) return;
      searchError = e instanceof Error ? e.message : String(e);
      results = [];
    } finally {
      if (seq === searchSeq) searching = false;
    }
  }

  function pick(issue: IssueSummary): void {
    onpick({ account_id: selectedAccountId, key: issue.key, summary: issue.summary });
  }

  function goToJiraSettings(): void {
    router.go('settings/jira');
  }

  const searchPlaceholder = $derived(
    selectedProjectKey
      ? 'Number (5218) or name…'
      : 'PROJ-123, number, or text…',
  );
</script>

{#if accountsLoading}
  <Skeleton rows={2} height={36} />
{:else if accounts.length === 0}
  <div class="picker-empty">
    <Icon name="ticket" size={14} />
    <span>No Jira accounts configured.</span>
    <button class="btn small ghost" onclick={() => goToJiraSettings()}>
      Settings → Jira
    </button>
  </div>
{:else}
  <!-- Account selector (only shown when >1 account) -->
  {#if accounts.length > 1}
    <div class="picker-field">
      <label class="picker-label" for="jp-account">Account</label>
      <select id="jp-account" class="picker-select" bind:value={selectedAccountId}>
        {#each accounts as a (a.id)}
          <option value={a.id}>{a.label} ({a.base_url})</option>
        {/each}
      </select>
    </div>
  {:else}
    <div class="account-badge">
      <Icon name="ticket" size={13} />
      <span>{accounts[0].label}</span>
      <span class="dim mono" style="font-size: var(--fs-xs)">{accounts[0].base_url}</span>
    </div>
  {/if}

  <!-- Project selector -->
  <div class="picker-field">
    <label class="picker-label" for="jp-project">Project</label>
    {#if projectsLoading}
      <div class="picker-loading"><Skeleton rows={3} height={28} label="projects" /></div>
    {:else}
      <select id="jp-project" class="picker-select" bind:value={selectedProjectKey}>
        <option value="">All projects</option>
        {#each projects as p (p.key)}
          <option value={p.key}>{p.name} ({p.key})</option>
        {/each}
      </select>
      {#if projectsTruncated}
        <p class="trunc-note" role="note">{truncatedNote('projects', projects.length)} Search by issue key to reach the others.</p>
      {/if}
    {/if}
  </div>

  <!-- Search input -->
  <div class="picker-field">
    <label class="picker-label" for="jp-query">Search issues</label>
    <input dir="auto"
      id="jp-query"
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
    {:else if searchError && query.trim() !== ''}
      <div class="no-results" role="alert">
        <span class="err">Search failed: {searchError}</span>
        <button class="btn small" onclick={() => void search(query.trim())}>Retry</button>
      </div>
    {:else if results.length === 0 && query.trim() !== ''}
      <div class="no-results dim">No issues found.</div>
    {:else}
      {#each results as issue (issue.key)}
        <button class="issue-row" onclick={() => pick(issue)}>
          <div class="issue-left">
            <span class="issue-key">{issue.key}</span>
            <span class="issue-summary">{issue.summary}</span>
          </div>
          <span class="chip status-chip">{issue.status}</span>
        </button>
      {/each}
    {/if}
  </div>
{/if}

<style>
  .trunc-note {
    margin: 4px 0 0;
    font-size: var(--fs-xs);
    color: var(--warning);
  }
  .picker-empty {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 14px 0;
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .account-badge {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 6px 10px;
    background: var(--surface-2);
    border-radius: var(--radius-s);
    font-size: var(--fs-s);
    margin-bottom: 12px;
  }
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
  .picker-loading {
    font-size: var(--fs-xs);
    color: var(--text-dim);
    padding: 4px 0;
  }
  .picker-results {
    margin-top: 4px;
    display: flex;
    flex-direction: column;
    gap: 4px;
    max-height: 260px;
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
  .no-results .err {
    display: block;
    margin-bottom: 8px;
    color: var(--danger);
  }
</style>
