<script lang="ts">
  // Shared Jira issue picker block — used by both ReviewPanel and AttachIssue.
  // Handles account selection, project selection, and debounced search.
  import { api } from '../../lib/api/client';
  import type { IssueAccount, IssueProject, IssueSummary } from '../../lib/api/types';
  import { toasts } from '../../lib/toast.svelte';
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
      toasts.error('Could not load Jira accounts', e instanceof Error ? e.message : String(e));
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
    selectedProjectKey = '';
    results = [];
    query = '';
    try {
      projects = await api.get<IssueProject[]>(
        `/issue/projects?account_id=${encodeURIComponent(accountId)}`,
      );
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

  async function search(q: string): Promise<void> {
    if (!selectedAccountId) return;
    searching = true;
    try {
      const projectParam = selectedProjectKey
        ? `&project=${encodeURIComponent(selectedProjectKey)}`
        : '';
      results = await api.get<IssueSummary[]>(
        `/issue/search?account_id=${encodeURIComponent(selectedAccountId)}&q=${encodeURIComponent(q)}${projectParam}`,
      );
    } catch (e) {
      toasts.error('Search failed', e instanceof Error ? e.message : String(e));
      results = [];
    } finally {
      searching = false;
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
      <span class="dim mono" style="font-size:11px">{accounts[0].base_url}</span>
    </div>
  {/if}

  <!-- Project selector -->
  <div class="picker-field">
    <label class="picker-label" for="jp-project">Project</label>
    {#if projectsLoading}
      <div class="picker-loading">Loading projects…</div>
    {:else}
      <select id="jp-project" class="picker-select" bind:value={selectedProjectKey}>
        <option value="">All projects</option>
        {#each projects as p (p.key)}
          <option value={p.key}>{p.name} ({p.key})</option>
        {/each}
      </select>
    {/if}
  </div>

  <!-- Search input -->
  <div class="picker-field">
    <label class="picker-label" for="jp-query">Search issues</label>
    <input
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
  .picker-empty {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 14px 0;
    font-size: 12.5px;
    color: var(--text-dim);
  }
  .account-badge {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 6px 10px;
    background: var(--surface-2);
    border-radius: var(--radius-s);
    font-size: 12.5px;
    margin-bottom: 12px;
  }
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
    max-height: 260px;
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
</style>
