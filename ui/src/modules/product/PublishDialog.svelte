<script lang="ts">
  // PublishDialog — shared modal for both "Publish as Jira Story" and
  // "Publish as Confluence RFC" actions. Also used for "Convert RFC → Story".
  import Modal from '../../lib/components/Modal.svelte';
  import { api } from '../../lib/api/client';
  import { product } from '../../lib/stores/product.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import type { IssueAccount, IssueProject } from '../../lib/api/types';
  import type { ConfluenceSpace } from './types';

  interface Props {
    mode: 'story' | 'rfc';
    onclose: () => void;
  }
  let { mode, onclose }: Props = $props();

  // ── Accounts (shared) ─────────────────────────────────────────────────────
  let accounts: IssueAccount[] = $state([]);
  let accountsLoading = $state(true);
  let accountId = $state('');

  // ── Story-mode: projects + issue types ───────────────────────────────────
  let projects: IssueProject[] = $state([]);
  let projectsLoading = $state(false);
  let projectKey = $state('');

  let issueTypes: string[] = $state([]);
  let issueTypesLoading = $state(false);
  let issueType = $state('Story');

  // ── RFC-mode: spaces + optional parent + optional title ──────────────────
  let spaces: ConfluenceSpace[] = $state([]);
  let spacesLoading = $state(false);
  let spaceKey = $state('');
  let parentId = $state('');
  let rfcTitle = $state('');

  // ── Submit ────────────────────────────────────────────────────────────────
  let submitting = $state(false);
  let formError = $state('');

  const title = $derived(mode === 'story' ? 'Publish as Jira Story' : 'Publish as Confluence RFC');

  // Load accounts on mount.
  $effect(() => {
    void loadAccounts();
  });

  async function loadAccounts(): Promise<void> {
    accountsLoading = true;
    try {
      accounts = await api.get<IssueAccount[]>('/issue/accounts');
      if (accounts.length > 0) {
        accountId = accounts[0].id;
        await onAccountChange();
      }
    } catch (e) {
      formError = e instanceof Error ? e.message : String(e);
    } finally {
      accountsLoading = false;
    }
  }

  async function onAccountChange(): Promise<void> {
    if (!accountId) return;
    if (mode === 'story') {
      await loadProjects();
    } else {
      await loadSpaces();
    }
  }

  async function loadProjects(): Promise<void> {
    projectsLoading = true;
    projectKey = '';
    issueTypes = [];
    issueType = 'Story';
    try {
      projects = await api.get<IssueProject[]>(`/issue/projects?account_id=${accountId}`);
      if (projects.length > 0) {
        projectKey = projects[0].key;
        await loadIssueTypes();
      }
    } catch (e) {
      formError = e instanceof Error ? e.message : String(e);
    } finally {
      projectsLoading = false;
    }
  }

  async function loadIssueTypes(): Promise<void> {
    if (!accountId || !projectKey) return;
    issueTypesLoading = true;
    try {
      issueTypes = await api.get<string[]>(`/issue/${accountId}/${projectKey}/issue-types`);
      issueType = issueTypes.includes('Story')
        ? 'Story'
        : issueTypes[0] ?? 'Story';
    } catch {
      // Non-fatal — default to 'Story'.
      issueTypes = [];
      issueType = 'Story';
    } finally {
      issueTypesLoading = false;
    }
  }

  async function loadSpaces(): Promise<void> {
    spacesLoading = true;
    spaceKey = '';
    try {
      spaces = await api.get<ConfluenceSpace[]>(
        `/issue/confluence/spaces?account_id=${accountId}`,
      );
      if (spaces.length > 0) spaceKey = spaces[0].key;
    } catch (e) {
      formError = e instanceof Error ? e.message : String(e);
    } finally {
      spacesLoading = false;
    }
  }

  async function submit(): Promise<void> {
    formError = '';
    if (!accountId) { formError = 'Select an account.'; return; }

    submitting = true;
    try {
      if (mode === 'story') {
        if (!projectKey) { formError = 'Select a project.'; submitting = false; return; }
        const detail = await product.publishAsStory({
          account_id: accountId,
          project_key: projectKey,
          issue_type: issueType || 'Story',
        });
        toasts.success('Published as Jira Story', detail.story.title);
        // Select the resulting story.
        if (detail.story.id !== product.selectedId) {
          await product.select(detail.story.id);
        }
      } else {
        if (!spaceKey) { formError = 'Select a Confluence space.'; submitting = false; return; }
        const detail = await product.publishAsRfc({
          account_id: accountId,
          space_key: spaceKey,
          parent_id: parentId.trim() || null,
          title: rfcTitle.trim() || null,
        });
        toasts.success('Published as Confluence RFC', detail.story.title);
      }
      onclose();
    } catch (e) {
      formError = e instanceof Error ? e.message : String(e);
    } finally {
      submitting = false;
    }
  }
</script>

<Modal {title} width={440} {onclose}>
  {#snippet children()}
    {#if accountsLoading}
      <div class="loading">Loading accounts…</div>
    {:else if accounts.length === 0}
      <div class="no-accounts">No issue accounts configured. Add one in Settings → Jira / Confluence.</div>
    {:else}
      <!-- Account -->
      <div class="field">
        <label class="label" for="pd-account">Account</label>
        <select
          id="pd-account"
          class="select"
          bind:value={accountId}
          onchange={onAccountChange}
          disabled={submitting}
        >
          {#each accounts as a (a.id)}
            <option value={a.id}>{a.label} ({a.base_url})</option>
          {/each}
        </select>
      </div>

      {#if mode === 'story'}
        <!-- Project -->
        <div class="field">
          <label class="label" for="pd-project">Project</label>
          {#if projectsLoading}
            <div class="loading-inline">Loading projects…</div>
          {:else}
            <select
              id="pd-project"
              class="select"
              bind:value={projectKey}
              onchange={loadIssueTypes}
              disabled={submitting || projects.length === 0}
            >
              {#if projects.length === 0}
                <option value="">No projects found</option>
              {:else}
                {#each projects as p (p.key)}
                  <option value={p.key}>{p.name} ({p.key})</option>
                {/each}
              {/if}
            </select>
          {/if}
        </div>

        <!-- Issue type -->
        <div class="field">
          <label class="label" for="pd-issuetype">Issue type</label>
          {#if issueTypesLoading}
            <div class="loading-inline">Loading types…</div>
          {:else}
            <select
              id="pd-issuetype"
              class="select"
              bind:value={issueType}
              disabled={submitting}
            >
              {#if issueTypes.length === 0}
                <option value="Story">Story</option>
              {:else}
                {#each issueTypes as t (t)}
                  <option value={t}>{t}</option>
                {/each}
              {/if}
            </select>
          {/if}
        </div>

      {:else}
        <!-- Space -->
        <div class="field">
          <label class="label" for="pd-space">Space</label>
          {#if spacesLoading}
            <div class="loading-inline">Loading spaces…</div>
          {:else}
            <select
              id="pd-space"
              class="select"
              bind:value={spaceKey}
              disabled={submitting || spaces.length === 0}
            >
              {#if spaces.length === 0}
                <option value="">No spaces found</option>
              {:else}
                {#each spaces as sp (sp.key)}
                  <option value={sp.key}>{sp.name} ({sp.key})</option>
                {/each}
              {/if}
            </select>
          {/if}
        </div>

        <!-- Optional parent page id -->
        <div class="field">
          <label class="label" for="pd-parent">Parent page ID <span class="dim">(optional)</span></label>
          <input
            id="pd-parent"
            class="input"
            bind:value={parentId}
            placeholder="e.g. 123456789"
            spellcheck="false"
            disabled={submitting}
          />
        </div>

        <!-- Optional title override -->
        <div class="field">
          <label class="label" for="pd-title">Title override <span class="dim">(optional)</span></label>
          <input
            id="pd-title"
            class="input"
            bind:value={rfcTitle}
            placeholder="Defaults to story title"
            spellcheck="false"
            disabled={submitting}
          />
        </div>
      {/if}

      {#if formError}
        <div class="field-error">{formError}</div>
      {/if}
    {/if}
  {/snippet}

  {#snippet footer()}
    <button class="btn ghost" onclick={onclose} disabled={submitting}>Cancel</button>
    <button
      class="btn primary"
      onclick={submit}
      disabled={submitting || accountsLoading || accounts.length === 0}
    >
      {submitting ? 'Publishing…' : (mode === 'story' ? 'Publish Story' : 'Publish RFC')}
    </button>
  {/snippet}
</Modal>

<style>
  .loading {
    padding: 12px 0;
    font-size: 12.5px;
    color: var(--text-dim);
  }
  .loading-inline {
    font-size: 12px;
    color: var(--text-dim);
    font-style: italic;
    padding: 4px 0;
  }
  .no-accounts {
    padding: 12px 14px;
    background: color-mix(in srgb, var(--text-dim) 8%, transparent);
    border-radius: var(--radius-s);
    font-size: 12.5px;
    color: var(--text-dim);
    line-height: 1.5;
  }
  .field {
    display: flex;
    flex-direction: column;
    gap: 4px;
    margin-bottom: 14px;
  }
  .label {
    font-size: 11px;
    font-weight: 500;
    color: var(--text-dim);
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }
  .dim {
    font-weight: 400;
    text-transform: none;
    letter-spacing: 0;
    font-size: 10.5px;
  }
  .select,
  .input {
    width: 100%;
    background: var(--surface-raised, var(--surface));
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    color: var(--text);
    font-size: 12.5px;
    padding: 5px 9px;
    box-sizing: border-box;
    outline: none;
  }
  .select:focus,
  .input:focus {
    border-color: var(--accent);
  }
  .select:disabled,
  .input:disabled {
    opacity: 0.55;
    cursor: not-allowed;
  }
  .field-error {
    font-size: 12px;
    color: var(--status-exited, #e53e3e);
    margin-bottom: 8px;
    padding: 6px 10px;
    background: color-mix(in srgb, var(--status-exited, #e53e3e) 10%, transparent);
    border-radius: var(--radius-s);
  }
  .btn {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    height: 32px;
    padding: 0 16px;
    border-radius: var(--radius-s);
    font-size: 12.5px;
    font-weight: 500;
    cursor: pointer;
    border: 1px solid var(--border);
    background: transparent;
    color: var(--text);
    transition: background 110ms, border-color 110ms, color 110ms;
  }
  .btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
  .btn.ghost:hover:not(:disabled) {
    background: color-mix(in srgb, var(--text-dim) 12%, transparent);
  }
  .btn.primary {
    background: var(--accent);
    border-color: var(--accent);
    color: #fff;
  }
  .btn.primary:hover:not(:disabled) {
    opacity: 0.88;
  }
</style>
