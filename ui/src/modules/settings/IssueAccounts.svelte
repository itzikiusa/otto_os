<script lang="ts">
  // Jira / issue-tracking accounts settings page.
  import { api } from '../../lib/api/client';
  import { confirmer } from '../../lib/confirm.svelte';
  import type { IssueAccount } from '../../lib/api/types';
  import { toasts } from '../../lib/toast.svelte';
  import Skeleton from '../../lib/components/Skeleton.svelte';
  import Modal from '../../lib/components/Modal.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import { ctxMenu } from '../../lib/contextmenu.svelte';

  let accounts: IssueAccount[] = $state([]);
  let loading = $state(true);
  let addOpen = $state(false);
  let busy = $state(false);

  // Edit state — null means "Add" mode, non-null means "Edit" mode.
  let editing: IssueAccount | null = $state(null);

  let label = $state('');
  let baseUrl = $state('');
  let email = $state('');
  let token = $state('');
  // User-entered token expiry as a yyyy-mm-dd value for the date input; '' = unset/clear.
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

  $effect(() => {
    void load();
  });

  async function load(): Promise<void> {
    loading = true;
    try {
      accounts = await api.get<IssueAccount[]>('/issue/accounts');
    } catch (e) {
      toasts.error('Could not load Jira accounts', e instanceof Error ? e.message : String(e));
    } finally {
      loading = false;
    }
  }

  function openAdd(): void {
    editing = null;
    label = '';
    baseUrl = '';
    email = '';
    token = '';
    tokenExpiresAt = '';
    addOpen = true;
  }

  function openEdit(a: IssueAccount): void {
    editing = a;
    label = a.label;
    baseUrl = a.base_url;
    email = a.email;
    token = '';
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
      const a = await api.post<IssueAccount>('/issue/accounts', {
        provider: 'jira',
        label: label.trim(),
        base_url: baseUrl.trim(),
        email: email.trim(),
        token,
        token_expires_at: fromDateInput(tokenExpiresAt),
      });
      accounts = [...accounts, a];
      closeModal();
      toasts.success('Jira account added', a.label);
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
        email: email.trim(),
        base_url: baseUrl.trim(),
        token_expires_at: fromDateInput(tokenExpiresAt),
      };
      if (token !== '') body.token = token;
      const updated = await api.patch<IssueAccount>(`/issue/accounts/${editing.id}`, body);
      accounts = accounts.map((x) => (x.id === updated.id ? updated : x));
      closeModal();
      toasts.success('Jira account updated', updated.label);
    } catch (e) {
      toasts.error('Update failed', e instanceof Error ? e.message : String(e));
    } finally {
      busy = false;
    }
  }

  async function remove(a: IssueAccount): Promise<void> {
    if (!(await confirmer.ask(`Delete account "${a.label}"? Its token is removed from the Keychain.`, { title: 'Delete account' }))) return;
    try {
      await api.del(`/issue/accounts/${a.id}`);
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
      <h1>Jira Accounts</h1>
      <div class="sub">Connect Jira to attach issues to sessions and track work in progress.</div>
    </div>
    <button class="btn primary" onclick={openAdd}>Add Account</button>
  </div>

  {#if loading}
    <Skeleton rows={2} height={48} />
  {:else if accounts.length === 0}
    <div class="card" style="padding: 24px; text-align: center; max-width: 520px">
      <p class="dim" style="margin: 0 0 10px">
        No Jira accounts yet. Add one to search and attach issues to your sessions.
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
          <span class="acct-icon"><Icon name="ticket" size={14} /></span>
          <div class="grow">
            <div class="acct-label">
              {a.label}
              <span class="chip">jira</span>
            </div>
            <div class="acct-sub dim">
              {a.email} · <span class="mono">{a.base_url}</span> · token ••••••
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
  <Modal title={isEdit ? 'Edit Jira Account' : 'Add Jira Account'} onclose={closeModal}>
    <div class="field">
      <label for="ia-label">Label</label>
      <input id="ia-label" class="input" bind:value={label} placeholder="work jira" />
    </div>
    <div class="field">
      <label for="ia-base">Base URL</label>
      <input
        id="ia-base"
        class="input mono"
        bind:value={baseUrl}
        placeholder="https://yourcompany.atlassian.net"
        spellcheck="false"
        autocomplete="off"
      />
    </div>
    <div class="field">
      <label for="ia-email">Email</label>
      <input
        id="ia-email"
        class="input"
        type="email"
        bind:value={email}
        placeholder="you@company.com"
        spellcheck="false"
        autocomplete="off"
      />
    </div>
    <div class="field">
      <label for="ia-token">API Token</label>
      <input
        id="ia-token"
        class="input"
        type="password"
        bind:value={token}
        autocomplete="off"
        placeholder={isEdit ? '•••••• (leave blank to keep)' : ''}
      />
      {#if isEdit}
        <span class="hint">Leave blank to keep the existing token.</span>
      {:else}
        <span class="hint">
          Write-only — never shown again after saving. Create a token at
          <strong>id.atlassian.com → Security → API tokens</strong>.
        </span>
      {/if}
    </div>
    <div class="field">
      <label for="ia-expiry">Token expiry <span class="dim">(optional)</span></label>
      <input id="ia-expiry" class="input" type="date" bind:value={tokenExpiresAt} />
      <span class="hint">Set the token's expiry date to get a reminder before it lapses.</span>
    </div>

    {#snippet footer()}
      <button class="btn" onclick={closeModal}>Cancel</button>
      {#if isEdit}
        <button
          class="btn primary"
          disabled={busy || label.trim() === '' || baseUrl.trim() === '' || email.trim() === ''}
          onclick={save}
        >
          {busy ? 'Saving…' : 'Save Changes'}
        </button>
      {:else}
        <button
          class="btn primary"
          disabled={busy || label.trim() === '' || baseUrl.trim() === '' || email.trim() === '' || token === ''}
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
