<script lang="ts">
  import { originChanged } from './credentialOrigin';
  import { rowMenu } from '../../lib/rowMenu';
  import { plural } from '../../lib/plural';
  import PageHeader from '../../lib/components/PageHeader.svelte';
  import { sectionLabel } from './sections';
  import PageBody from '../../lib/components/PageBody.svelte';
  // Issue-tracker accounts (Jira, Linear) settings page.
  import { createIssueAccount, deleteIssueAccount, listIssueProjects, updateIssueAccount } from '../../lib/api/issues';
  import { issueAccounts } from '../../lib/stores/issueAccounts.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import type { IssueAccount, IssueProviderKind } from '../../lib/api/types';
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

  let provider = $state<IssueProviderKind>('jira');
  let label = $state('');
  let baseUrl = $state('');
  let email = $state('');
  let token = $state('');
  // User-entered token expiry as a yyyy-mm-dd value for the date input; '' = unset/clear.
  let tokenExpiresAt = $state('');

  const isEdit = $derived(editing !== null);
  const isLinear = $derived(provider === 'linear');
  /** Display name of an account's tracker. */
  function providerName(p: IssueProviderKind): string {
    return p === 'linear' ? 'Linear' : 'Jira';
  }
  /** Jira signs in with site + email + token; Linear with the API key alone
   *  (the base URL is just the workspace link). */
  const formReady = $derived(
    label.trim() !== '' && (isLinear || (baseUrl.trim() !== '' && email.trim() !== '')),
  );
  /** Repointing an account at another origin (scheme, host or port) must not
   *  carry the stored token there (it would be sent as Basic auth to the new
   *  origin): a changed origin needs a fresh token — the daemon's own rule. */
  const hostChanged = $derived.by(() => {
    // (cast: TS narrows the `$state(null)` initializer to `null` at this point)
    const e = editing as IssueAccount | null;
    return !!e && originChanged(e.base_url, baseUrl);
  });
  const needsToken = $derived(hostChanged && token === '');

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
  // There's no dedicated test route; listing the projects (Jira) or teams
  // (Linear) the token can see exercises the same credential and is read-only.
  type TestResult = { ok: true; projects: number } | { ok: false; error: string } | 'busy';
  let testResults: Record<string, TestResult> = $state({});

  async function testAccount(a: IssueAccount): Promise<void> {
    testResults = { ...testResults, [a.id]: 'busy' };
    try {
      const projects = await listIssueProjects(a.id, { meta: false });
      testResults = { ...testResults, [a.id]: { ok: true, projects: projects.items.length } };
    } catch (e) {
      testResults = { ...testResults, [a.id]: { ok: false, error: loadErrorText(e) } };
    }
  }

  function testLabel(a: IssueAccount, r: Exclude<TestResult, 'busy'>): string {
    return r.ok
      ? `Connected · ${plural(r.projects, a.provider === 'linear' ? 'team' : 'project')} visible`
      : `Couldn’t connect: ${r.error}`;
  }

  async function load(): Promise<void> {
    loading = true;
    loadError = '';
    try {
      accounts = await issueAccounts.load(true);
    } catch (e) {
      loadError = loadErrorText(e);
    } finally {
      loading = false;
    }
  }

  function openAdd(): void {
    editing = null;
    provider = 'jira';
    label = '';
    baseUrl = '';
    email = '';
    token = '';
    tokenExpiresAt = '';
    addOpen = true;
  }

  function openEdit(a: IssueAccount): void {
    editing = a;
    provider = a.provider;
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
      const a = await createIssueAccount({
        provider,
        label: label.trim(),
        base_url: baseUrl.trim(),
        email: isLinear ? '' : email.trim(),
        token,
        token_expires_at: fromDateInput(tokenExpiresAt),
      });
      accounts = [...accounts, a];
      issueAccounts.upsert(a);
      closeModal();
      toasts.success(`${providerName(a.provider)} account added`, a.label);
    } catch (e) {
      toasts.error(`Couldn’t add the ${providerName(provider)} account`, loadErrorText(e));
    } finally {
      busy = false;
    }
  }

  async function save(): Promise<void> {
    if (!editing || needsToken) return;
    busy = true;
    try {
      const body: Record<string, string | null | undefined> = {
        label: label.trim(),
        email: email.trim(),
        base_url: baseUrl.trim(),
        token_expires_at: fromDateInput(tokenExpiresAt),
      };
      if (token !== '') body.token = token;
      const updated = await updateIssueAccount(editing.id, body);
      accounts = accounts.map((x) => (x.id === updated.id ? updated : x));
      issueAccounts.upsert(updated);
      // A changed token/URL makes an earlier verdict stale.
      const { [updated.id]: _stale, ...rest } = testResults;
      testResults = rest;
      closeModal();
      toasts.success(`${providerName(updated.provider)} account updated`, updated.label);
    } catch (e) {
      toasts.error(`Couldn’t save the ${providerName(provider)} account`, loadErrorText(e));
    } finally {
      busy = false;
    }
  }

  /** Accounts whose delete is in flight, by id — that row's button disables. */
  let deleting = $state<Record<string, boolean>>({});

  async function remove(a: IssueAccount): Promise<void> {
    if (deleting[a.id]) return;
    if (!(await confirmer.ask(`Delete account “${a.label}”? Its token is removed from the Keychain.`, { title: 'Delete account?', danger: true, confirmLabel: 'Delete account' }))) return;
    deleting = { ...deleting, [a.id]: true };
    try {
      await deleteIssueAccount(a.id);
      accounts = accounts.filter((x) => x.id !== a.id);
      issueAccounts.remove(a.id);
      toasts.success(`${providerName(a.provider)} account deleted`, a.label);
    } catch (e) {
      toasts.error(`Couldn’t delete the ${providerName(a.provider)} account`, loadErrorText(e));
    } finally {
      const next = { ...deleting };
      delete next[a.id];
      deleting = next;
    }
  }
</script>

<div class="settings-section">
  <PageHeader title={sectionLabel('jira')} subtitle="Search and attach Jira and Linear issues">
    {#snippet actions()}
      <!-- While the list is empty the EmptyState owns the one "Add account". -->
      {#if accounts.length > 0}
        <button class="btn small primary" onclick={openAdd}><Icon name="plus" size={13} /> Add account</button>
      {/if}
    {/snippet}
  </PageHeader>
  <PageBody width="readable">

  <LoadState what="issue-tracker accounts" variant="page" {loading} error={loadError} empty={accounts.length === 0} onretry={() => void load()} rows={2}>
    {#snippet emptyView()}
      <EmptyState
        variant="page"
        icon="ticket"
        title="No Jira or Linear accounts yet"
        body="Add a Jira Cloud or Linear account to search issues, attach them to sessions and publish from Product. The token is stored in the macOS Keychain."
        actionLabel="Add account"
        actionIcon="plus"
        onaction={openAdd}
      />
    {/snippet}
    <div class="acct-list">
      {#each accounts as a (a.id)}
        {@const warn = expiryWarning(a.token_expires_at)}
        {@const r = testResults[a.id]}
        <!-- Right-click is a pointer shortcut; Test / Edit / Delete are buttons on the card. -->
        <div use:rowMenu
          role="presentation"
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
              <span class="chip">{providerName(a.provider)}</span>
              {#if warn === 'expired'}
                <span class="chip bad" title="The token has expired — edit the account and paste a new one">Token expired</span>
              {:else if warn === 'soon'}
                <span class="chip chip-warn" title="Update the token before it lapses">{expiryLabel(a.token_expires_at!).replace(/^./, (c) => c.toUpperCase())}</span>
              {/if}
            </div>
            <div class="acct-sub">
              {#if a.email}{a.email} · {/if}<span class="mono">{a.base_url}</span>
              {#if a.token_expires_at && !warn}
                · <span class="expiry">{expiryLabel(a.token_expires_at)}</span>
              {/if}
            </div>
            {#if r}
              <div class="test-result" role="status" class:ok={r !== 'busy' && r.ok} class:bad={r !== 'busy' && !r.ok}>
                {#if r !== 'busy'}<Icon name={r.ok ? 'check' : 'warning'} size={12} />{/if}
                {r === 'busy' ? 'Testing…' : testLabel(a, r)}
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
          <button
            class="icon-btn acct-tool"
            title={deleting[a.id] ? `Deleting ${a.label}…` : `Delete ${a.label}`}
            aria-label={deleting[a.id] ? `Deleting ${a.label}…` : `Delete ${a.label}`}
            aria-busy={deleting[a.id] ? 'true' : undefined}
            disabled={deleting[a.id]}
            onclick={() => remove(a)}
          >
            {#if deleting[a.id]}<span class="spinner" aria-hidden="true"></span>{:else}<Icon name="trash" size={14} />{/if}
          </button>
        </div>
      {/each}
    </div>
  </LoadState>
  </PageBody>
</div>

{#if addOpen}
  <Modal title={isEdit ? `Edit ${providerName(provider)} account` : 'Add issue-tracker account'} onclose={closeModal}>
    {#if !isEdit}
      <div class="field">
        <span class="field-label" id="ia-provider-label">Tracker</span>
        <div class="segmented" role="group" aria-labelledby="ia-provider-label">
          {#each ['jira', 'linear'] as const as p (p)}
            <button class:active={provider === p} aria-pressed={provider === p} onclick={() => (provider = p)}>{providerName(p)}</button>
          {/each}
        </div>
        <span class="hint">Searches, fetches and comments go to the tracker the account belongs to.</span>
      </div>
    {/if}
    <div class="field">
      <label for="ia-label">Label</label>
      <input dir="auto" id="ia-label" class="input" bind:value={label} placeholder={isLinear ? 'Work Linear' : 'Work Jira'} />
    </div>
    <div class="field">
      <label for="ia-base">{isLinear ? 'Workspace URL' : 'Base URL'}{#if isLinear} <span class="dim">(optional)</span>{/if}</label>
      <input dir="ltr"
        id="ia-base"
        class="input mono"
        bind:value={baseUrl}
        placeholder={isLinear ? 'https://linear.app/yourteam' : 'https://yourcompany.atlassian.net'}
        spellcheck="false"
        autocomplete="off"
      />
    </div>
    {#if !isLinear}
    <div class="field">
      <label for="ia-email">Email</label>
      <input dir="ltr"
        id="ia-email"
        class="input"
        type="email"
        bind:value={email}
        placeholder="you@company.com"
        spellcheck="false"
        autocomplete="off"
      />
    </div>
    {/if}
    <div class="field">
      <label for="ia-token">{isLinear ? 'API key' : 'API token'}</label>
      <input
        id="ia-token"
        class="input"
        type="password"
        bind:value={token}
        autocomplete="off"
        placeholder={isEdit ? '•••••• (leave blank to keep)' : ''}
      />
      {#if isEdit && hostChanged}
        <span class="hint warn-hint" role="alert">The site changed — paste a token for the new site. The stored token is never sent to a different host.</span>
      {:else if isEdit}
        <span class="hint">Leave blank to keep the existing token.</span>
      {:else if isLinear}
        <span class="hint">
          Write-only — never shown again after saving. Create a key in Linear at
          <strong>Settings → Security &amp; access → Personal API keys</strong>.
        </span>
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
      <span class="hint">Set the token’s expiry date to get a reminder before it lapses.</span>
    </div>

    {#snippet footer()}
      <button class="btn" onclick={closeModal}>Cancel</button>
      {#if isEdit}
        <button
          class="btn primary"
          disabled={busy || !formReady || needsToken}
          onclick={save}
        >
          {busy ? 'Saving…' : 'Save changes'}
        </button>
      {:else}
        <button
          class="btn primary"
          disabled={busy || !formReady || token === ''}
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
    gap: 4px;
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
  .hint.warn-hint {
    color: var(--danger);
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
