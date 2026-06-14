<script lang="ts">
  // Git accounts settings page: provider, label, username, token (write-only),
  // api_base_url for self-hosted GitLab.
  import { api } from '../../lib/api/client';
  import { confirmer } from '../../lib/confirm.svelte';
  import type { GitAccount, GitProviderKind } from '../../lib/api/types';
  import { toasts } from '../../lib/toast.svelte';
  import Skeleton from '../../lib/components/Skeleton.svelte';
  import Modal from '../../lib/components/Modal.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import { ctxMenu } from '../../lib/contextmenu.svelte';

  let accounts: GitAccount[] = $state([]);
  let loading = $state(true);
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
    try {
      accounts = await api.get<GitAccount[]>('/git/accounts');
    } catch (e) {
      toasts.error('Could not load git accounts', e instanceof Error ? e.message : String(e));
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
      toasts.error('Add failed', e instanceof Error ? e.message : String(e));
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
      closeModal();
      toasts.success('Git account updated', updated.label);
    } catch (e) {
      toasts.error('Update failed', e instanceof Error ? e.message : String(e));
    } finally {
      busy = false;
    }
  }

  async function remove(a: GitAccount): Promise<void> {
    if (!(await confirmer.ask(`Delete account "${a.label}"? Its token is removed from the Keychain.`, { title: 'Delete account' }))) return;
    try {
      await api.del(`/git/accounts/${a.id}`);
      accounts = accounts.filter((x) => x.id !== a.id);
      toasts.info('Account deleted', a.label);
    } catch (e) {
      toasts.error('Delete failed', e instanceof Error ? e.message : String(e));
    }
  }
</script>

<div class="page">
  <div class="page-header">
    <div>
      <h1>Git Accounts</h1>
      <div class="sub">Tokens live in the macOS Keychain and authenticate PR actions + https pushes.</div>
    </div>
    <button class="btn primary" onclick={openAdd}>Add Account</button>
  </div>

  {#if loading}
    <Skeleton rows={2} height={48} />
  {:else if accounts.length === 0}
    <div class="card" style="padding: 24px; text-align: center; max-width: 520px">
      <p class="dim" style="margin: 0 0 10px">
        No git accounts yet. Add one to list pull requests and push over https.
      </p>
      <button class="btn primary" onclick={openAdd}>Add Account</button>
    </div>
  {:else}
    <div class="acct-list">
      {#each accounts as a (a.id)}
        <!-- svelte-ignore a11y_no_static_element_interactions -->
        <div
          class="acct card"
          oncontextmenu={(e) => ctxMenu.show(e, [
            { label: 'Edit', icon: 'edit', action: () => openEdit(a) },
            { separator: true },
            { label: 'Delete', icon: 'trash', danger: true, action: () => remove(a) },
          ])}
        >
          <span class="acct-icon"><Icon name="key" size={14} /></span>
          <div class="grow">
            <div class="acct-label">
              {a.label}
              <span class="chip">{a.provider}</span>
            </div>
            <div class="acct-sub dim">
              {a.username}
              {#if a.namespace}· <span class="mono">{a.namespace}</span>{/if}
              {#if a.api_base_url}· <span class="mono">{a.api_base_url}</span>{/if}
              · token ••••••
              {#if a.token_expires_at}
                · <span class="expiry" class:expired={new Date(a.token_expires_at).getTime() <= Date.now()}>{expiryLabel(a.token_expires_at)}</span>
              {/if}
            </div>
          </div>
          <button class="icon-btn" title="Edit" onclick={() => openEdit(a)}>
            <Icon name="edit" size={13} />
          </button>
          <button class="icon-btn" title="Delete" onclick={() => remove(a)}>
            <Icon name="trash" size={13} />
          </button>
        </div>
      {/each}
    </div>
  {/if}
</div>

{#if addOpen}
  <Modal title={isEdit ? 'Edit Git Account' : 'Add Git Account'} onclose={closeModal}>
    <div class="field">
      <label for="ga-provider">Provider</label>
      {#if isEdit}
        <div class="input" style="background: var(--surface-2, #1e1e1e); cursor: default; opacity: 0.7;">
          {provider}
        </div>
        <span class="hint">Provider cannot be changed after creation.</span>
      {:else}
        <div class="segmented" id="ga-provider">
          {#each ['github', 'bitbucket', 'gitlab'] as p (p)}
            <button
              class:active={provider === p}
              onclick={() => (provider = p as GitProviderKind)}
            >
              {p}
            </button>
          {/each}
        </div>
        <span class="hint">{providerHints[provider]}</span>
      {/if}
    </div>
    <div class="field">
      <label for="ga-label">Label</label>
      <input id="ga-label" class="input" bind:value={label} placeholder="work github" />
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

    {#snippet footer()}
      <button class="btn" onclick={closeModal}>Cancel</button>
      {#if isEdit}
        <button
          class="btn primary"
          disabled={busy || label.trim() === '' || username.trim() === ''}
          onclick={save}
        >
          {busy ? 'Saving…' : 'Save Changes'}
        </button>
      {:else}
        <button
          class="btn primary"
          disabled={busy || label.trim() === '' || username.trim() === '' || token === ''}
          onclick={create}
        >
          {busy ? 'Adding…' : 'Add Account'}
        </button>
      {/if}
    {/snippet}
  </Modal>
{/if}

<style>
  .acct-list {
    display: flex;
    flex-direction: column;
    gap: 8px;
    max-width: 560px;
  }
  .acct {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 12px 14px;
  }
  .acct-icon {
    width: 30px;
    height: 30px;
    border-radius: var(--radius-s);
    background: color-mix(in srgb, var(--accent) 14%, transparent);
    color: var(--accent);
    display: grid;
    place-items: center;
  }
  .acct-label {
    font-size: 13px;
    font-weight: 600;
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .acct-sub {
    font-size: 11.5px;
    margin-top: 2px;
  }
  .expiry {
    color: var(--text-dim);
  }
  .expiry.expired {
    color: #d9534f;
    font-weight: 600;
  }
</style>
