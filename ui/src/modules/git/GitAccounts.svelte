<script lang="ts">
  import PageHeader from '../../lib/components/PageHeader.svelte';
  import { sectionLabel } from '../settings/sections';
  import PageBody from '../../lib/components/PageBody.svelte';
  // Git accounts settings page: provider, label, username, token (write-only),
  // api_base_url for self-hosted GitLab.
  import { api } from '../../lib/api/client';
  import { confirmer } from '../../lib/confirm.svelte';
  import type { GitAccount, GitAccountTestResp, GitProviderKind } from '../../lib/api/types';
  import { toasts } from '../../lib/toast.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import { loadErrorText } from '../../lib/loadError';
  import Modal from '../../lib/components/Modal.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import { ctxMenu } from '../../lib/contextmenu.svelte';

  let accounts: GitAccount[] = $state([]);
  let loading = $state(true);
  let loadError = $state('');
  let addOpen = $state(false);
  let busy = $state(false);

  // Edit state — null means "Add" mode, non-null means "Edit" mode.
  let editing: GitAccount | null = $state(null);

  let provider: GitProviderKind = $state('github');
  let label = $state('');
  let username = $state('');
  let token = $state('');
  let apiBaseUrl = $state('');
  let namespace = $state('');
  // Token expiry: GitHub/GitLab auto-detect server-side; Bitbucket is manual.
  // Stored as a yyyy-mm-dd value for the date input; '' = unset/clear.
  let tokenExpiresAt = $state('');

  const isEdit = $derived(editing !== null);

  // ── Connection test ────────────────────────────────────────────────────────
  // Per-row results (keyed by account id) + one slot for the add/edit form.
  // 'busy' while in flight; the endpoint returns ok:false inline for auth
  // failures (HTTP 200), so a red row here is the provider's own error text.
  let testResults: Record<string, GitAccountTestResp | 'busy'> = $state({});
  let formTest: GitAccountTestResp | 'busy' | null = $state(null);

  function testLabel(r: GitAccountTestResp): string {
    if (!r.ok) return `Couldn't connect: ${r.error ?? 'the provider refused the token'}`;
    const scopes = r.scopes?.length ? ` · scopes: ${r.scopes.join(', ')}` : '';
    return `Connected as ${r.login ?? 'an unknown user'}${scopes}`;
  }

  /** Provider ids → their proper names (GitHub, not "github"). */
  const PROVIDER_LABELS: Record<GitProviderKind, string> = {
    github: 'GitHub',
    bitbucket: 'Bitbucket',
    gitlab: 'GitLab',
  };
  const PROVIDERS: GitProviderKind[] = ['github', 'bitbucket', 'gitlab'];

  async function testAccount(a: GitAccount): Promise<void> {
    testResults = { ...testResults, [a.id]: 'busy' };
    try {
      const r = await api.post<GitAccountTestResp>(`/git/accounts/${a.id}/test`, {});
      testResults = { ...testResults, [a.id]: r };
    } catch (e) {
      testResults = {
        ...testResults,
        [a.id]: { ok: false, error: loadErrorText(e) },
      };
    }
  }

  /** Form variant: a typed token tests the DRAFT credentials; an edit form with
   *  the token left blank tests the stored one. */
  async function testForm(): Promise<void> {
    formTest = 'busy';
    try {
      formTest =
        token === '' && editing
          ? await api.post<GitAccountTestResp>(`/git/accounts/${editing.id}/test`, {})
          : await api.post<GitAccountTestResp>('/git/accounts/test', {
              provider,
              username: username.trim(),
              token,
              api_base_url:
                provider === 'gitlab' && apiBaseUrl.trim() !== '' ? apiBaseUrl.trim() : null,
            });
    } catch (e) {
      formTest = { ok: false, error: loadErrorText(e) };
    }
  }

  // ── Token-expiry helpers ───────────────────────────────────────────────────
  /** ISO timestamp → yyyy-mm-dd for an <input type="date"> (UTC date part). */
  function toDateInput(iso: string | null): string {
    return iso ? iso.slice(0, 10) : '';
  }
  /** yyyy-mm-dd from the date input → ISO timestamp (or null when cleared). */
  function fromDateInput(d: string): string | null {
    return d.trim() === '' ? null : new Date(`${d}T00:00:00Z`).toISOString();
  }
  /** Human-friendly expiry label, e.g. "expired", "expires today", "expires in 5 days". */
  function expiryLabel(iso: string): string {
    const day = 86_400_000;
    const diff = new Date(iso).getTime() - Date.now();
    const days = Math.ceil(diff / day);
    if (days < 0) return 'expired';
    if (days === 0) return 'expires today';
    if (days === 1) return 'expires in 1 day';
    if (days <= 30) return `expires in ${days} days`;
    return `expires ${new Date(iso).toLocaleDateString()}`;
  }

  /** Same rule as Jira accounts: 'expired', 'soon' (within 14 days) or ''. */
  function expiryWarning(iso: string | null): '' | 'soon' | 'expired' {
    if (!iso) return '';
    const days = Math.ceil((new Date(iso).getTime() - Date.now()) / 86_400_000);
    if (days < 0) return 'expired';
    if (days <= 14) return 'soon';
    return '';
  }

  const providerHints: Record<GitProviderKind, string> = {
    github: 'Personal access token (classic or fine-grained) with repo scope.',
    bitbucket: 'App password with pullrequest read/write scopes.',
    gitlab: 'Personal access token with api scope. Set API base URL for self-hosted.',
  };

  // What the org/namespace is called on each provider (for browsing repos).
  const namespaceLabel: Record<GitProviderKind, string> = {
    github: 'Organisation / user',
    bitbucket: 'Workspace',
    gitlab: 'Group / user',
  };
  const namespaceHint: Record<GitProviderKind, string> = {
    github: 'e.g. your org login. Repos under it can be searched and cloned.',
    bitbucket: 'e.g. your-org. Repos under it can be searched and cloned.',
    gitlab: 'e.g. your group path. Repos under it can be searched and cloned.',
  };

  $effect(() => {
    void load();
  });

  async function load(): Promise<void> {
    loading = true;
    loadError = '';
    try {
      accounts = await api.get<GitAccount[]>('/git/accounts');
    } catch (e) {
      loadError = loadErrorText(e);
    } finally {
      loading = false;
    }
  }

  function openAdd(): void {
    editing = null;
    provider = 'github';
    label = '';
    username = '';
    token = '';
    apiBaseUrl = '';
    namespace = '';
    tokenExpiresAt = '';
    addOpen = true;
  }

  function openEdit(a: GitAccount): void {
    editing = a;
    provider = a.provider;
    label = a.label;
    username = a.username;
    token = '';
    apiBaseUrl = a.api_base_url ?? '';
    namespace = a.namespace ?? '';
    tokenExpiresAt = toDateInput(a.token_expires_at);
    addOpen = true;
  }

  function closeModal(): void {
    addOpen = false;
    editing = null;
    formTest = null;
  }

  async function create(): Promise<void> {
    busy = true;
    try {
      const a = await api.post<GitAccount>('/git/accounts', {
        provider,
        label: label.trim(),
        username: username.trim(),
        token,
        api_base_url: provider === 'gitlab' && apiBaseUrl.trim() !== '' ? apiBaseUrl.trim() : null,
        namespace: namespace.trim() !== '' ? namespace.trim() : null,
        token_expires_at: fromDateInput(tokenExpiresAt),
      });
      accounts = [...accounts, a];
      closeModal();
      toasts.success('Git account added', a.label);
    } catch (e) {
      toasts.error("Couldn't add the Git account", loadErrorText(e));
    } finally {
      busy = false;
    }
  }

  async function save(): Promise<void> {
    if (!editing) return;
    busy = true;
    try {
      const body: Record<string, string | null | undefined> = {
        label: label.trim(),
        username: username.trim(),
        namespace: namespace.trim(),
        api_base_url: provider === 'gitlab' ? apiBaseUrl.trim() : '',
        token_expires_at: fromDateInput(tokenExpiresAt),
      };
      if (token !== '') body.token = token;
      const updated = await api.patch<GitAccount>(`/git/accounts/${editing.id}`, body);
      accounts = accounts.map((x) => (x.id === updated.id ? updated : x));
      // A changed token/username makes an earlier verdict stale.
      const { [updated.id]: _stale, ...rest } = testResults;
      testResults = rest;
      closeModal();
      toasts.success('Git account updated', updated.label);
    } catch (e) {
      toasts.error("Couldn't save the Git account", loadErrorText(e));
    } finally {
      busy = false;
    }
  }

  async function remove(a: GitAccount): Promise<void> {
    if (!(await confirmer.ask(`Delete account "${a.label}"? Its token is removed from the Keychain.`, { title: 'Delete account' }))) return;
    try {
      await api.del(`/git/accounts/${a.id}`);
      accounts = accounts.filter((x) => x.id !== a.id);
      toasts.success('Git account deleted', a.label);
    } catch (e) {
      toasts.error("Couldn't delete the Git account", loadErrorText(e));
    }
  }
</script>

<div class="settings-section">
  <PageHeader title={sectionLabel('git-accounts')} subtitle="Tokens for pull requests and HTTPS pushes">
    {#snippet actions()}
      <!-- While the list is empty the EmptyState owns the one "Add account". -->
      {#if accounts.length > 0}
        <button class="btn primary" onclick={openAdd}><Icon name="plus" size={13} /> Add account</button>
      {/if}
    {/snippet}
  </PageHeader>
  <PageBody width="readable">

  <LoadState what="Git accounts" variant="page" {loading} error={loadError} empty={accounts.length === 0} onretry={() => void load()} rows={2}>
    {#snippet emptyView()}
      <EmptyState
        variant="page"
        icon="branch"
        title="No Git accounts yet"
        body="Add a GitHub, Bitbucket or GitLab token to list and review pull requests and to push over HTTPS. The token is stored in the macOS Keychain."
        actionLabel="Add account"
        actionIcon="plus"
        onaction={openAdd}
      />
    {/snippet}
    <div class="acct-list">
      {#each accounts as a (a.id)}
        {@const warn = expiryWarning(a.token_expires_at)}
        <!-- svelte-ignore a11y_no_static_element_interactions -->
        <div
          class="acct card"
          oncontextmenu={(e) => ctxMenu.show(e, [
            { label: 'Test connection', icon: 'refresh', action: () => testAccount(a) },
            { label: 'Edit…', icon: 'edit', action: () => openEdit(a) },
            { separator: true },
            { label: 'Delete…', icon: 'trash', danger: true, action: () => remove(a) },
          ])}
        >
          <span class="acct-icon"><Icon name="branch" size={14} /></span>
          <div class="grow">
            <div class="acct-label">
              <span class="acct-name" title={a.label}>{a.label}</span>
              <span class="chip">{PROVIDER_LABELS[a.provider] ?? a.provider}</span>
              {#if warn === 'expired'}
                <span class="chip bad" title="The token has expired — edit the account and paste a new one">Token expired</span>
              {:else if warn === 'soon'}
                <span class="chip chip-warn" title="Update the token before it lapses">{expiryLabel(a.token_expires_at!).replace(/^./, (c) => c.toUpperCase())}</span>
              {/if}
            </div>
            <div class="acct-sub">
              {a.username}
              {#if a.namespace}· <span class="mono">{a.namespace}</span>{/if}
              {#if a.api_base_url}· <span class="mono">{a.api_base_url}</span>{/if}
              {#if a.token_expires_at && !warn}
                · <span class="expiry">{expiryLabel(a.token_expires_at)}</span>
              {/if}
            </div>
            {#if testResults[a.id]}
              {@const r = testResults[a.id]}
              <div class="test-result" role="status" class:ok={r !== 'busy' && r.ok} class:bad={r !== 'busy' && !r.ok}>
                {#if r !== 'busy'}<Icon name={r.ok ? 'check' : 'warning'} size={12} />{/if}
                {r === 'busy' ? 'Testing…' : testLabel(r)}
              </div>
            {/if}
          </div>
          <button
            class="btn small"
            title="Check the stored token against {PROVIDER_LABELS[a.provider] ?? a.provider}"
            disabled={testResults[a.id] === 'busy'}
            onclick={() => testAccount(a)}
          >
            {testResults[a.id] === 'busy' ? 'Testing…' : 'Test'}
          </button>
          <button class="icon-btn acct-tool" title="Edit {a.label}" aria-label="Edit {a.label}" onclick={() => openEdit(a)}>
            <Icon name="edit" size={14} />
          </button>
          <button class="icon-btn acct-tool" title="Delete {a.label}" aria-label="Delete {a.label}" onclick={() => remove(a)}>
            <Icon name="trash" size={14} />
          </button>
        </div>
      {/each}
    </div>
  </LoadState>
  </PageBody>
</div>

{#if addOpen}
  <Modal title={isEdit ? 'Edit Git account' : 'Add Git account'} onclose={closeModal}>
    <div class="field">
      <label for="ga-provider">Provider</label>
      {#if isEdit}
        <div class="input provider-fixed" id="ga-provider">{PROVIDER_LABELS[provider]}</div>
        <span class="hint">Provider cannot be changed after creation.</span>
      {:else}
        <div class="segmented" id="ga-provider" role="group" aria-label="Provider">
          {#each PROVIDERS as p (p)}
            <button
              class:active={provider === p}
              aria-pressed={provider === p}
              onclick={() => { provider = p; formTest = null; }}
            >
              {PROVIDER_LABELS[p]}
            </button>
          {/each}
        </div>
        <span class="hint">{providerHints[provider]}</span>
      {/if}
    </div>
    <div class="field">
      <label for="ga-label">Label</label>
      <input id="ga-label" class="input" bind:value={label} placeholder="Work {PROVIDER_LABELS[provider]}" />
    </div>
    <div class="field">
      <label for="ga-user">Username</label>
      <input id="ga-user" class="input" bind:value={username} spellcheck="false" />
    </div>
    <div class="field">
      <label for="ga-token">Token</label>
      <input id="ga-token" class="input" type="password" bind:value={token} autocomplete="off" placeholder={isEdit ? '•••••• (leave blank to keep)' : ''} />
      {#if isEdit}
        <span class="hint">Leave blank to keep the existing token.</span>
      {:else}
        <span class="hint">Write-only — it is never shown again after saving.</span>
      {/if}
    </div>
    <div class="field">
      <label for="ga-ns">{namespaceLabel[provider]} <span class="dim">(optional)</span></label>
      <input id="ga-ns" class="input mono" bind:value={namespace} spellcheck="false" placeholder={provider === 'bitbucket' ? 'your-org' : ''} />
      <span class="hint">{namespaceHint[provider]}</span>
    </div>
    {#if provider === 'gitlab'}
      <div class="field">
        <label for="ga-base">API base URL <span class="dim">(optional, self-hosted)</span></label>
        <input id="ga-base" class="input mono" bind:value={apiBaseUrl} placeholder="https://gitlab.example.com/api/v4" spellcheck="false" />
      </div>
    {/if}
    <div class="field">
      <label for="ga-expiry">Token expiry <span class="dim">(optional)</span></label>
      <input id="ga-expiry" class="input" type="date" bind:value={tokenExpiresAt} />
      <span class="hint">
        {#if provider === 'bitbucket'}
          Bitbucket doesn't expose token expiry — set it here to get an expiry reminder.
        {:else}
          GitHub/GitLab auto-detect expiry; set a value here only to override.
        {/if}
      </span>
    </div>

    {#if formTest}
      <div class="test-result form-test" role="status" class:ok={formTest !== 'busy' && formTest.ok} class:bad={formTest !== 'busy' && !formTest.ok}>
        {#if formTest !== 'busy'}<Icon name={formTest.ok ? 'check' : 'warning'} size={12} />{/if}
        {formTest === 'busy' ? 'Testing…' : testLabel(formTest)}
      </div>
    {/if}

    {#snippet footer()}
      <button
        class="btn"
        style="margin-inline-end: auto"
        title={token === '' && !isEdit ? 'Enter a token to test it' : 'Check the token against the provider without saving'}
        disabled={busy || formTest === 'busy' || (token === '' && !isEdit)}
        onclick={testForm}
      >
        {formTest === 'busy' ? 'Testing…' : 'Test connection'}
      </button>
      <button class="btn" onclick={closeModal}>Cancel</button>
      {#if isEdit}
        <button
          class="btn primary"
          disabled={busy || label.trim() === '' || username.trim() === ''}
          onclick={save}
        >
          {busy ? 'Saving…' : 'Save changes'}
        </button>
      {:else}
        <button
          class="btn primary"
          disabled={busy || label.trim() === '' || username.trim() === '' || token === ''}
          onclick={create}
        >
          {busy ? 'Adding…' : 'Add account'}
        </button>
      {/if}
    {/snippet}
  </Modal>
{/if}

<style>
  /* Section chrome: shared PageHeader bar + scrolling PageBody. */
  .settings-section {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }
  .acct-list {
    display: flex;
    flex-direction: column;
    gap: 8px;
    max-width: var(--settings-col);
  }
  .acct {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 12px 14px;
  }
  .acct .grow {
    min-width: 0;
  }
  .acct-icon {
    width: 32px;
    height: 32px;
    flex-shrink: 0;
    border-radius: var(--radius-s);
    background: var(--surface-2);
    color: var(--text-dim);
    display: grid;
    place-items: center;
  }
  .acct-label {
    font-size: var(--fs-m);
    font-weight: 600;
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
  }
  .acct-name {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .acct-sub {
    font-size: var(--fs-s);
    color: var(--text-dim);
    margin-top: 2px;
    overflow-wrap: anywhere;
  }
  .chip-warn {
    color: var(--warning);
    border-color: color-mix(in srgb, var(--warning) 35%, transparent);
    background: var(--warning-soft);
  }
  /* Inline connection-test verdict (row + form); the provider's own error
     text follows the words. */
  .test-result {
    display: flex;
    align-items: flex-start;
    gap: 5px;
    font-size: var(--fs-s);
    margin-top: 4px;
    overflow-wrap: anywhere;
    color: var(--text-dim);
  }
  .test-result :global(svg) {
    flex-shrink: 0;
    margin-top: 2px;
  }
  .test-result.ok {
    color: var(--success);
  }
  .test-result.bad {
    color: var(--danger);
  }
  .form-test {
    margin: 4px 0 0;
  }
  .provider-fixed {
    display: flex;
    align-items: center;
    color: var(--text-dim);
    cursor: default;
  }

  /* ── Mobile + tablet (≤1024px): full-width account rows + form, and 36px
     tap targets. The modal box is already viewport-clamped by Modal.svelte. ── */
  @media (max-width: 1024px) {
    .acct-list {
      max-width: none;
    }
    .acct {
      gap: 10px;
      padding: 12px;
    }
    .acct-tool {
      min-width: 36px;
      min-height: 36px;
    }
    /* Provider switch fills the row instead of overflowing a narrow modal. */
    .segmented {
      display: flex;
      width: 100%;
    }
    .segmented > button {
      flex: 1;
      height: 32px;
      white-space: nowrap;
    }
  }
</style>
