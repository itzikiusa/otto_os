<script lang="ts">
  // Import Story dialog — choose a Jira/Confluence account, pick source kind,
  // search for and pick a source, optionally set cwd + watch. Calls product.importStory()
  // then selects the new story and closes.
  import Modal from '../../lib/components/Modal.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import FolderPicker from '../../lib/components/FolderPicker.svelte';
  import { api } from '../../lib/api/client';
  import { product } from '../../lib/stores/product.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import type { IssueAccount } from '../../lib/api/types';
  import SourceSearch from './SourceSearch.svelte';

  interface Props {
    onclose: () => void;
  }
  let { onclose }: Props = $props();

  // ── form state ────────────────────────────────────────────────────────────
  let accounts: IssueAccount[] = $state([]);
  let accountsLoading = $state(true);
  let accountsError = $state('');

  let accountId = $state('');
  let sourceKind: 'jira' | 'confluence' = $state('jira');

  // Picked via SourceSearch
  let selectedKey = $state('');
  let selectedLabel = $state('');

  // Manual fallback
  let showManual = $state(false);
  let manualKey = $state('');

  let cwd = $state('');
  let watchEnabled = $state(false);

  let submitting = $state(false);
  let formError = $state('');

  let folderPickerOpen = $state(false);

  // ── load accounts on mount ────────────────────────────────────────────────
  $effect(() => {
    void loadAccounts();
  });

  async function loadAccounts(): Promise<void> {
    accountsLoading = true;
    accountsError = '';
    try {
      accounts = await api.get<IssueAccount[]>('/issue/accounts');
      if (accounts.length > 0) accountId = accounts[0].id;
    } catch (e) {
      accountsError = e instanceof Error ? e.message : String(e);
    } finally {
      accountsLoading = false;
    }
  }

  // ── reset the picked selection when the account or source kind changes ─────
  // Explicit handler (NOT an $effect): an $effect that writes selectedKey could
  // re-run and spuriously clear a valid pick. This only fires on a real change.
  function resetSelection(): void {
    selectedKey = '';
    selectedLabel = '';
    manualKey = '';
    showManual = false;
  }

  // ── helpers ───────────────────────────────────────────────────────────────

  // Normalise a raw user-entered key or URL:
  //  · Jira: extract PROJ-123 from a Jira browse URL (e.g. .../browse/PROJ-123)
  //          or from arbitrary pasted text that contains an issue key pattern.
  //  · Confluence: strip to numeric pageId from ?pageId=NNN or /pages/NNN/.
  function normaliseSourceKey(raw: string): string {
    const s = raw.trim();
    if (sourceKind === 'jira') {
      // https://xxx.atlassian.net/browse/PROJ-123 or text containing PROJ-123
      const m = s.match(/(?:\/browse\/|^|\s)([A-Z][A-Z0-9]{1,9}-\d+)(?:[/?#\s]|$)/);
      if (m) return m[1];
      return s;
    }
    // Confluence: Match ?pageId=NNN or /pages/NNN/
    const m = s.match(/(?:pageId=|\/pages\/)(\d+)/);
    if (m) return m[1];
    return s;
  }

  // The effective key submitted: manual field wins if filled, else selectedKey.
  const effectiveKey = $derived(
    showManual && manualKey.trim() ? normaliseSourceKey(manualKey) : selectedKey,
  );

  async function submit(): Promise<void> {
    formError = '';
    const key = effectiveKey;
    if (!accountId) { formError = 'Please select an account.'; return; }
    if (!key) { formError = 'Please pick an issue/page or enter an ID manually.'; return; }

    submitting = true;
    try {
      const story = await product.importStory({
        source_kind: sourceKind,
        account_id: accountId,
        source_key: key,
        cwd: cwd.trim() || null,
        watch_enabled: watchEnabled,
      });
      // Select the newly imported story and switch to Overview tab.
      await product.select(story.id);
      product.tab = 'overview';
      onclose();
    } catch (e) {
      formError = e instanceof Error ? e.message : String(e);
    } finally {
      submitting = false;
    }
  }
</script>

<Modal title="Import story" width={480} {onclose}>
  {#snippet children()}
    {#if accountsLoading}
      <div class="loading">Loading accounts…</div>
    {:else if accountsError}
      <div class="field-error">Could not load accounts: {accountsError}</div>
    {:else if accounts.length === 0}
      <div class="no-accounts">
        <Icon name="ticket" size={16} />
        <span>No issue accounts configured. Add one in <strong>Settings → Jira / Confluence</strong>.</span>
      </div>
    {:else}
      <!-- Account -->
      <div class="field">
        <label class="label" for="import-account">Account</label>
        <select id="import-account" class="select" bind:value={accountId} onchange={resetSelection}>
          {#each accounts as a (a.id)}
            <option value={a.id}>{a.label} ({a.base_url})</option>
          {/each}
        </select>
      </div>

      <!-- Source kind -->
      <div class="field">
        <span class="label">Source</span>
        <div class="kind-row">
          <label class="kind-opt" class:active={sourceKind === 'jira'}>
            <input type="radio" bind:group={sourceKind} value="jira" onchange={resetSelection} />
            <Icon name="ticket" size={13} />
            Jira issue
          </label>
          <label class="kind-opt" class:active={sourceKind === 'confluence'}>
            <input type="radio" bind:group={sourceKind} value="confluence" onchange={resetSelection} />
            <Icon name="globe" size={13} />
            Confluence page
          </label>
        </div>
      </div>

      <!-- Selected chip (when a result has been picked) -->
      {#if selectedKey}
        <div class="selected-chip">
          <span class="selected-label">{selectedLabel || selectedKey}</span>
          <button
            class="change-btn"
            onclick={() => { selectedKey = ''; selectedLabel = ''; }}
          >
            Change
          </button>
        </div>
      {:else}
        <!-- SourceSearch picker -->
        <div class="field search-field">
          <span class="label">
            {sourceKind === 'jira' ? 'Issue' : 'Page'}
          </span>
          <SourceSearch
            {accountId}
            {sourceKind}
            onpick={(s) => { selectedKey = s.key; selectedLabel = s.label; }}
          />
        </div>
      {/if}

      <!-- Manual fallback toggle -->
      <div class="manual-toggle-row">
        <button
          class="manual-toggle"
          type="button"
          onclick={() => (showManual = !showManual)}
        >
          {showManual ? '▾' : '▸'}
          Enter {sourceKind === 'jira' ? 'issue key' : 'page ID'} manually
        </button>
      </div>

      {#if showManual}
        <div class="field">
          <label class="label" for="import-key">
            {sourceKind === 'jira' ? 'Issue key' : 'Page ID or URL'}
          </label>
          <input
            id="import-key"
            class="input"
            bind:value={manualKey}
            placeholder={sourceKind === 'jira' ? 'e.g. PROJ-123' : 'e.g. 123456789 or page URL'}
            spellcheck="false"
            autocomplete="off"
          />
        </div>
      {/if}

      <!-- Repo path (cwd) -->
      <div class="field">
        <label class="label" for="import-cwd">Repo path <span class="dim">(optional)</span></label>
        <div class="cwd-row">
          <input
            id="import-cwd"
            class="input"
            bind:value={cwd}
            placeholder="/path/to/repo"
            spellcheck="false"
            autocomplete="off"
          />
          <button
            class="icon-btn"
            title="Browse folder"
            aria-label="Browse folder"
            onclick={() => (folderPickerOpen = true)}
          >
            <Icon name="folder" size={13} />
          </button>
        </div>
      </div>

      <!-- Watch toggle -->
      <label class="watch-row">
        <input type="checkbox" bind:checked={watchEnabled} />
        <span>Watch this story for changes</span>
      </label>

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
      disabled={submitting || accountsLoading || accounts.length === 0 || !effectiveKey}
    >
      {submitting ? 'Importing…' : 'Import'}
    </button>
  {/snippet}
</Modal>

{#if folderPickerOpen}
  <FolderPicker
    title="Select repo folder"
    onpick={(p) => { cwd = p; folderPickerOpen = false; }}
    onclose={() => (folderPickerOpen = false)}
  />
{/if}

<style>
  .loading {
    padding: 12px 0;
    font-size: 12.5px;
    color: var(--text-dim);
  }
  .no-accounts {
    display: flex;
    align-items: flex-start;
    gap: 10px;
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
  .search-field {
    margin-bottom: 4px;
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
  .kind-row {
    display: flex;
    gap: 8px;
  }
  .kind-opt {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 6px 12px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    font-size: 12.5px;
    cursor: pointer;
    color: var(--text-dim);
    transition: border-color 110ms, color 110ms, background 110ms;
    user-select: none;
  }
  .kind-opt input {
    display: none;
  }
  .kind-opt.active {
    border-color: var(--accent);
    color: var(--accent);
    background: color-mix(in srgb, var(--accent) 10%, transparent);
  }
  .kind-opt:hover:not(.active) {
    border-color: color-mix(in srgb, var(--accent) 40%, transparent);
    color: var(--text);
  }
  /* Selected chip */
  .selected-chip {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
    padding: 8px 12px;
    margin-bottom: 10px;
    background: color-mix(in srgb, var(--accent) 10%, transparent);
    border: 1px solid color-mix(in srgb, var(--accent) 35%, transparent);
    border-radius: var(--radius-s);
    font-size: 12.5px;
  }
  .selected-label {
    color: var(--text);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    flex: 1;
    min-width: 0;
  }
  .change-btn {
    flex-shrink: 0;
    font-size: 11.5px;
    color: var(--accent);
    background: transparent;
    border: none;
    cursor: pointer;
    padding: 0 4px;
    text-decoration: underline;
  }
  .change-btn:hover {
    opacity: 0.75;
  }
  /* Manual toggle */
  .manual-toggle-row {
    margin-bottom: 10px;
  }
  .manual-toggle {
    background: transparent;
    border: none;
    cursor: pointer;
    font-size: 11.5px;
    color: var(--text-dim);
    padding: 0;
    display: flex;
    align-items: center;
    gap: 4px;
  }
  .manual-toggle:hover {
    color: var(--text);
  }
  .cwd-row {
    display: flex;
    gap: 6px;
    align-items: center;
  }
  .cwd-row .input {
    flex: 1;
  }
  .watch-row {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 12.5px;
    color: var(--text);
    cursor: pointer;
    margin-bottom: 14px;
    user-select: none;
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
  .icon-btn {
    display: grid;
    place-items: center;
    width: 32px;
    height: 32px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--text-dim);
    cursor: pointer;
    flex-shrink: 0;
  }
  .icon-btn:hover {
    background: color-mix(in srgb, var(--text-dim) 12%, transparent);
    color: var(--text);
  }
</style>
