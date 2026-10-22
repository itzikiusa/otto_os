<script lang="ts">
  // Shared issue picker block (Jira or Linear, per account) — used by both ReviewPanel and AttachIssue.
  // Handles account selection, project selection, and debounced search.
  import { listIssueProjects, searchIssues } from '../../lib/api/issues';
  import { issueAccounts } from '../../lib/stores/issueAccounts.svelte';
  import { loadErrorText } from '../../lib/loadError';
  import LoadState from '../../lib/components/LoadState.svelte';
  import type { IssueAccount, IssueProject, IssueProviderKind, IssueSummary } from '../../lib/api/types';
  import { truncatedNote } from '../../lib/listingPage';
  import Skeleton from '../../lib/components/Skeleton.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import { router } from '../../lib/router.svelte';

  /** What a pick hands back: enough to persist a session's `AttachedIssue` without a second
   *  lookup — the tracker link and status from the search hit, and the picked account's provider. */
  interface PickedIssue {
    account_id: string;
    provider: IssueProviderKind;
    key: string;
    summary: string;
    url: string;
    status: string;
  }

  interface Props {
    onpick: (issue: PickedIssue) => void;
  }
  let { onpick }: Props = $props();

  // --- state ---
  let accounts: IssueAccount[] = $state([]);
  let accountsLoading = $state(true);
  /** A failed account load shows inline with Retry — not as “no accounts”. */
  let accountsError = $state<string | null>(null);
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
  // Why the last search failed — shown in the results area with Retry (a toast
  // plus "No issues found." read as a real empty result).
  let searchError = $state<string | null>(null);
  let searchSeq = 0;

  // --- account loading ---
  $effect(() => {
    void loadAccounts();
  });

  async function loadAccounts(): Promise<void> {
    accountsLoading = true;
    accountsError = null;
    try {
      accounts = await issueAccounts.load();
      if (accounts.length > 0) selectedAccountId = accounts[0].id;
    } catch (e) {
      accountsError = loadErrorText(e);
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

  let projectsSeq = 0;
  async function loadProjects(accountId: string): Promise<void> {
    // Switching accounts quickly: only the newest account's projects land.
    const seq = ++projectsSeq;
    const current = () => seq === projectsSeq && selectedAccountId === accountId;
    projectsLoading = true;
    projects = [];
    projectsTruncated = false;
    selectedProjectKey = '';
    resetSearch();
    try {
      const page = await listIssueProjects(accountId, { meta: true });
      if (!current()) return;
      projects = page.items;
      projectsTruncated = page.truncated;
    } catch {
      // Non-fatal: user can still search without a project filter.
      if (current()) projects = [];
    } finally {
      if (current()) projectsLoading = false;
    }
  }

  // --- reset results when project changes ---
  $effect(() => {
    void selectedProjectKey;
    resetSearch();
  });

  /** Drop the results AND anything still on its way for them: a search (or a
   *  pending debounce) started under the previous account/project would land
   *  after the switch — and picking from it attached that issue key under the
   *  newly selected account. */
  function resetSearch(): void {
    ++searchSeq;
    if (debounceTimer) { clearTimeout(debounceTimer); debounceTimer = null; }
    searching = false;
    searchError = null;
    results = [];
    query = '';
  }

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


  async function search(q: string): Promise<void> {
    if (!selectedAccountId) return;
    const seq = ++searchSeq;
    searching = true;
    searchError = null;
    try {
      const found = await searchIssues({
        accountId: selectedAccountId,
        q,
        project: selectedProjectKey || undefined,
      });
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
    const provider = accounts.find((a) => a.id === selectedAccountId)?.provider ?? 'jira';
    onpick({
      account_id: selectedAccountId,
      provider,
      key: issue.key,
      summary: issue.summary,
      url: issue.url ?? '',
      status: issue.status ?? '',
    });
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

{#if accountsLoading || accountsError}
  <LoadState what="issue-tracker accounts" variant="compact" loading={accountsLoading} error={accountsError} empty rows={2} onretry={() => void loadAccounts()} />
{:else if accounts.length === 0}
  <div class="picker-empty">
    <Icon name="ticket" size={14} />
    <span>No Jira or Linear accounts configured.</span>
    <button class="btn small ghost" onclick={() => goToJiraSettings()}>
      Settings → Jira &amp; Linear
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
