<script lang="ts">
  import PageHeader from '../../lib/components/PageHeader.svelte';
  import { sectionLabel } from './sections';
  import PageBody from '../../lib/components/PageBody.svelte';
  // Jira / issue-tracking accounts settings page.
  import { api } from '../../lib/api/client';
  import { confirmer } from '../../lib/confirm.svelte';
  import type { IssueAccount } from '../../lib/api/types';
  import { toasts } from '../../lib/toast.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import { loadErrorText } from '../../lib/loadError';
  import Modal from '../../lib/components/Modal.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import { ctxMenu } from '../../lib/contextmenu.svelte';

  let accounts: IssueAccount[] = $state([]);
  let loading = $state(true);
  let loadError = $state('');
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

  /** Returns 'expired', 'soon' (within 14 days), or '' for no warning. */
  function expiryWarning(iso: string | null): '' | 'soon' | 'expired' {
    if (!iso) return '';
    const day = 86_400_000;
    const diff = new Date(iso).getTime() - Date.now();
    const days = Math.ceil(diff / day);
    if (days < 0) return 'expired';
    if (days <= 14) return 'soon';
    return '';
  }

  $effect(() => {
    void load();
  });

  // ── Connection test ────────────────────────────────────────────────────────
  // There's no dedicated test route for Jira; listing the projects the token
  // can see exercises the same base URL + email + token and is read-only.
  type TestResult = { ok: true; projects: number } | { ok: false; error: string } | 'busy';
  let testResults: Record<string, TestResult> = $state({});

  async function testAccount(a: IssueAccount): Promise<void> {
    testResults = { ...testResults, [a.id]: 'busy' };
    try {
      const projects = await api.get<unknown[]>(`/issue/projects?account_id=${encodeURIComponent(a.id)}`);
      testResults = { ...testResults, [a.id]: { ok: true, projects: Array.isArray(projects) ? projects.length : 0 } };
    } catch (e) {
      testResults = { ...testResults, [a.id]: { ok: false, error: loadErrorText(e) } };
    }
  }

  function testLabel(r: Exclude<TestResult, 'busy'>): string {
    return r.ok
      ? `Connected · ${r.projects} project${r.projects === 1 ? '' : 's'} visible`
      : `Couldn't connect: ${r.error}`;
  }

  async function load(): Promise<void> {
    loading = true;
    loadError = '';
    try {
      accounts = await api.get<IssueAccount[]>('/issue/accounts');
    } catch (e) {
      loadError = loadErrorText(e);
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
      toasts.error("Couldn't add the Jira account", loadErrorText(e));
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
      // A changed token/URL makes an earlier verdict stale.
      const { [updated.id]: _stale, ...rest } = testResults;
      testResults = rest;
      closeModal();
      toasts.success('Jira account updated', updated.label);
    } catch (e) {
      toasts.error("Couldn't save the Jira account", loadErrorText(e));
    } finally {
      busy = false;
    }
  }

  async function remove(a: IssueAccount): Promise<void> {
    if (!(await confirmer.ask(`Delete account "${a.label}"? Its token is removed from the Keychain.`, { title: 'Delete account' }))) return;
    try {
      await api.del(`/issue/accounts/${a.id}`);
      accounts = accounts.filter((x) => x.id !== a.id);
      toasts.success('Jira account deleted', a.label);
    } catch (e) {
      toasts.error("Couldn't delete the Jira account", loadErrorText(e));
    }
  }
</script>

<div class="settings-section">
  <PageHeader title={sectionLabel('jira')} subtitle="Search and attach Jira issues">
    {#snippet actions()}
      <!-- While the list is empty the EmptyState owns the one "Add account". -->
      {#if accounts.length > 0}
        <button class="btn primary" onclick={openAdd}><Icon name="plus" size={13} /> Add account</button>
      {/if}
    {/snippet}
  </PageHeader>
  <PageBody width="readable">

  <LoadState what="Jira accounts" variant="page" {loading} error={loadError} empty={accounts.length === 0} onretry={() => void load()} rows={2}>
    {#snippet emptyView()}
      <EmptyState
        variant="page"
        icon="ticket"
        title="No Jira accounts yet"
        body="Add a Jira Cloud account to search issues, attach them to sessions and publish from Product. The API token is stored in the macOS Keychain."
        actionLabel="Add account"
        actionIcon="plus"
        onaction={openAdd}
      />
    {/snippet}
    <div class="acct-list">
      {#each accounts as a (a.id)}
        {@const warn = expiryWarning(a.token_expires_at)}
        {@const r = testResults[a.id]}
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
          <span class="acct-icon"><Icon name="ticket" size={14} /></span>
          <div class="grow">
            <div class="acct-label">
              <span class="acct-name" title={a.label}>{a.label}</span>
              {#if warn === 'expired'}
                <span class="chip bad" title="The token has expired — edit the account and paste a new one">Token expired</span>
              {:else if warn === 'soon'}
                <span class="chip chip-warn" title="Update the token before it lapses">{expiryLabel(a.token_expires_at!).replace(/^./, (c) => c.toUpperCase())}</span>
              {/if}
            </div>
            <div class="acct-sub">
              {a.email} · <span class="mono">{a.base_url}</span>
              {#if a.token_expires_at && !warn}
                · <span class="expiry">{expiryLabel(a.token_expires_at)}</span>
              {/if}
            </div>
            {#if r}
              <div class="test-result" role="status" class:ok={r !== 'busy' && r.ok} class:bad={r !== 'busy' && !r.ok}>
                {#if r !== 'busy'}<Icon name={r.ok ? 'check' : 'warning'} size={12} />{/if}
                {r === 'busy' ? 'Testing…' : testLabel(r)}
              </div>
            {/if}
          </div>
          <button
            class="btn small acct-test"
            title="Check the stored token against {a.base_url}"
            disabled={r === 'busy'}
            onclick={() => testAccount(a)}
          >
            {r === 'busy' ? 'Testing…' : 'Test'}
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
  <Modal title={isEdit ? 'Edit Jira account' : 'Add Jira account'} onclose={closeModal}>
    <div class="field">
      <label for="ia-label">Label</label>
      <input id="ia-label" class="input" bind:value={label} placeholder="Work Jira" />
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
      <label for="ia-token">API token</label>
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
          {busy ? 'Saving…' : 'Save changes'}
        </button>
      {:else}
        <button
          class="btn primary"
          disabled={busy || label.trim() === '' || baseUrl.trim() === '' || email.trim() === '' || token === ''}
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
    container-type: inline-size;
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

  @container (max-width: 480px) {
    .acct { display: grid; grid-template-columns: auto minmax(0, 1fr) auto auto; }
    .acct .grow { grid-column: 2 / -1; }
    .acct-test { grid-column: 2; justify-self: end; }
    .acct-label { flex-wrap: wrap; }
    .acct-name { white-space: normal; overflow-wrap: anywhere; }
  }

  @media (max-width: 1024px) {
    .acct-list {
      max-width: none;
    }
    .acct-tool {
      min-width: 36px;
      min-height: 36px;
    }
  }
</style>
