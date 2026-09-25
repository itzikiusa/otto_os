<script lang="ts">
  // PublishDialog — shared modal for both "Publish as Jira Story" and
  // "Publish as Confluence RFC" actions. Also used for "Convert RFC → Story".
  import Modal from '../../lib/components/Modal.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import { router } from '../../lib/router.svelte';
  import { api, ApiError } from '../../lib/api/client';
  import { product } from '../../lib/stores/product.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import type { IssueAccount, IssueProject } from '../../lib/api/types';
  import type { ConfluenceSpace, ProductStoryVersion } from './types';

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
  // Errors are a human title plus the raw cause as a dim detail line — never
  // the bare exception text on its own.
  let formError = $state('');
  let formErrorDetail = $state('');

  function setError(what: string, e: unknown): void {
    const raw = e instanceof Error ? e.message : String(e);
    let why = '';
    if (e instanceof ApiError) {
      if (e.status === 401 || e.status === 403) why = ' The account was refused — check its token in Settings → Integrations → Jira.';
      else if (e.status === 404) why = ' The project, space or parent page was not found.';
      else if (e.status === 409) why = ' It conflicts with the current state on the server.';
    } else if (e instanceof TypeError) why = " Otto can't reach the daemon.";
    formError = `${what}.${why}`;
    formErrorDetail = raw;
  }

  const title = $derived(mode === 'story' ? 'Publish as Jira story' : 'Publish as Confluence RFC');

  // ── Preview: WHAT is sent (the same version the daemon publishes — newest
  // suggested, else draft, else source; mirrors `best_content_version`). ─────
  const storyTitle = $derived(product.detail?.story.title ?? '');
  const isDraft = $derived(product.detail?.story.source_kind === 'draft');
  let previewBody = $state<string | null>(null); // null = loading
  // Converting a Confluence RFC → story: the daemon prepends a "> RFC: <url>"
  // reference line to the Jira description (`publish_as_story`), so the
  // preview shows it too — the confirm must match what is actually sent.
  const rfcRef = $derived.by(() => {
    const s = product.detail?.story;
    return mode === 'story' && s?.source_kind === 'confluence' && s.url ? `> RFC: ${s.url}` : '';
  });
  const PREVIEW_LINES = 6;
  const previewLines = $derived.by(() => {
    const body = rfcRef ? `${rfcRef}\n\n${previewBody ?? ''}` : (previewBody ?? '');
    const lines = body.split('\n').map((l) => l.trimEnd()).filter((l) => l.trim() !== '');
    return { head: lines.slice(0, PREVIEW_LINES), more: Math.max(0, lines.length - PREVIEW_LINES) };
  });

  async function loadPreview(sid: string | null): Promise<void> {
    if (!sid) { previewBody = ''; return; }
    try {
      const vs = await api.get<ProductStoryVersion[]>(`/product/stories/${sid}/versions`);
      const pick = vs.find((v) => v.kind === 'suggested') ?? vs.find((v) => v.kind === 'draft') ?? vs.find((v) => v.kind === 'source');
      if (!pick) { previewBody = ''; return; }
      previewBody = pick.body_md ? pick.body_md : (await product.getVersion(pick.id)).body_md;
    } catch {
      previewBody = '';
    }
  }

  // ── WHO sees it ───────────────────────────────────────────────────────────
  const visibility = $derived.by(() => {
    if (mode === 'story') {
      const p = projects.find((x) => x.key === projectKey);
      return p
        ? `Creates a ${issueType || 'Story'} in ${p.name} (${p.key}), visible to everyone with access to that Jira project.`
        : '';
    }
    const sp = spaces.find((x) => x.key === spaceKey);
    return sp
      ? `Creates a page in the ${sp.name} (${sp.key}) space${parentId.trim() ? `, under page ${parentId.trim()}` : ''}, visible to everyone who can view that space.`
      : '';
  });

  // Load accounts + the preview on mount.
  $effect(() => {
    void loadAccounts();
    void loadPreview(product.selectedId);
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
      setError("Couldn't load your Jira / Confluence accounts", e);
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
      setError("Couldn't load Jira projects", e);
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
      setError("Couldn't load Confluence spaces", e);
    } finally {
      spacesLoading = false;
    }
  }

  async function submit(): Promise<void> {
    formError = '';
    formErrorDetail = '';
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
        toasts.success('Published as Jira story', detail.story.title);
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
      setError(mode === 'story' ? "Couldn't publish to Jira" : "Couldn't publish to Confluence", e);
    } finally {
      submitting = false;
    }
  }
</script>

<Modal {title} width={440} {onclose}>
  {#snippet children()}
    {#if accountsLoading}
      <div class="loading">Loading accounts…</div>
    {:else if accounts.length === 0 && formError}
      <!-- A failed accounts load is an error with Retry, not "no accounts yet". -->
      <div class="field-error">
        {formError}
        {#if formErrorDetail}<div class="pd-error-detail">{formErrorDetail}</div>{/if}
      </div>
      <button class="btn small" onclick={() => { formError = ''; formErrorDetail = ''; void loadAccounts(); }}>Retry</button>
    {:else if accounts.length === 0}
      <div class="no-accounts">No Jira or Confluence account is connected yet — add one to publish.</div>
      <button class="btn small" onclick={() => { onclose(); router.go('settings/jira'); }}>
        <Icon name="plus" size={12} /> Add account in Settings
      </button>
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

      <!-- What is sent + who sees it (outward-facing confirm, patterns.md §5). -->
      <div class="pd-preview" data-testid="publish-preview">
        <div class="label">What is published</div>
        <div class="pd-preview-title">{(mode === 'rfc' && rfcTitle.trim()) || storyTitle || 'Untitled'}</div>
        {#if previewBody === null}
          <div class="loading-inline">Loading the content…</div>
        {:else if previewLines.head.length === 0}
          <div class="loading-inline">No body — only the title is published.</div>
        {:else}
          <pre class="pd-preview-body">{previewLines.head.join('\n')}</pre>
          {#if previewLines.more > 0}
            <div class="pd-preview-more">+{previewLines.more} more line{previewLines.more === 1 ? '' : 's'}</div>
          {/if}
        {/if}
        {#if visibility}
          <div class="pd-visibility" data-testid="publish-visibility">{visibility}</div>
        {/if}
        {#if isDraft}
          <div class="pd-visibility">This draft becomes the published {mode === 'story' ? 'issue' : 'page'}.</div>
        {/if}
      </div>

      {#if formError}
        <div class="field-error">
          {formError}
          {#if formErrorDetail}<div class="pd-error-detail">{formErrorDetail}</div>{/if}
        </div>
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
      {submitting ? 'Publishing…' : (mode === 'story' ? 'Publish story' : 'Publish RFC')}
    </button>
  {/snippet}
</Modal>

<style>
  .loading {
    padding: 12px 0;
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .loading-inline {
    font-size: var(--fs-s);
    color: var(--text-dim);
    font-style: italic;
    padding: 4px 0;
  }
  .no-accounts {
    padding: 12px 14px;
    background: color-mix(in srgb, var(--text-dim) 8%, transparent);
    border-radius: var(--radius-s);
    font-size: var(--fs-s);
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
    font-size: var(--fs-xs);
    font-weight: 500;
    color: var(--text-dim);
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }
  .dim {
    font-weight: 400;
    text-transform: none;
    letter-spacing: 0;
    font-size: var(--fs-xs);
  }
  .select,
  .input {
    width: 100%;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    color: var(--text);
    font-size: var(--fs-s);
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
  .pd-preview {
    display: flex;
    flex-direction: column;
    gap: 4px;
    margin-bottom: 14px;
    padding: 10px 12px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: color-mix(in srgb, var(--text-dim) 5%, transparent);
  }
  .pd-preview-title {
    font-size: var(--fs-m);
    font-weight: 600;
    color: var(--text);
  }
  .pd-preview-body {
    margin: 0;
    font-family: inherit;
    font-size: var(--fs-s);
    line-height: 1.45;
    color: var(--text-dim);
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    max-height: 120px;
    overflow-y: auto;
  }
  .pd-preview-more,
  .pd-visibility,
  .pd-error-detail {
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .pd-visibility {
    margin-top: 4px;
  }
  .pd-error-detail {
    margin-top: 4px;
    overflow-wrap: anywhere;
  }
  .field-error {
    font-size: var(--fs-s);
    color: var(--danger);
    margin-bottom: 8px;
    padding: 6px 10px;
    background: color-mix(in srgb, var(--danger) 10%, transparent);
    border-radius: var(--radius-s);
  }
  .btn {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    height: 32px;
    padding: 0 16px;
    border-radius: var(--radius-s);
    font-size: var(--fs-s);
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
    background: var(--accent-solid);
    border-color: var(--accent-solid);
    color: var(--accent-contrast);
  }
  .btn.primary:hover:not(:disabled) {
    opacity: 0.88;
  }
</style>
