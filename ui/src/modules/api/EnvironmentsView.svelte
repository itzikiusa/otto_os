<script lang="ts">
  // Environments, edited in the main area (they used to be squeezed into the
  // 280 px sidebar). A list of environments on the left — the active one is
  // what {{variables}} resolve against when you send — and the selected
  // environment's variables as a full-width table on the right.
  import Icon from '../../lib/components/Icon.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import { apiClient } from '../../lib/stores/apiClient.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import { ctxMenu } from '../../lib/contextmenu.svelte';
  import type { ApiEnvironment, Id } from '../../lib/api/types';

  interface Props {
    /** Preselect this environment (e.g. from the header switcher). */
    initialId?: Id | null;
  }
  let { initialId = null }: Props = $props();

  const canEdit = $derived(ws.myRole !== 'viewer');
  // svelte-ignore state_referenced_locally
  let selectedId = $state<Id | null>(initialId);
  const selected = $derived(
    apiClient.environments.find((e) => e.id === selectedId) ??
      apiClient.environments.find((e) => e.is_active) ??
      apiClient.environments[0] ??
      null,
  );

  // Local key/value rows for the selected env. A `secret` row's value lives in
  // the macOS Keychain: it renders masked; `touched` marks a newly typed
  // replacement (the only case where a value is sent on save). `storedKey` is
  // the Keychain name (null for new rows) — renaming sends `secret_renames` so
  // the stored value follows the new name instead of being deleted.
  interface VarRow { key: string; value: string; secret: boolean; touched: boolean; storedKey: string | null; }
  let rows: VarRow[] = $state([]);
  let dirty = $state(false);
  let loadedFor = $state<string | null>(null);

  function seed(env: ApiEnvironment | null): void {
    loadedFor = env?.id ?? null;
    dirty = false;
    rows = env
      ? [
          ...Object.entries(env.variables).map(([key, value]) => ({ key, value, secret: false, touched: false, storedKey: null })),
          ...env.secret_keys.map((key) => ({ key, value: '', secret: true, touched: false, storedKey: key })),
        ]
      : [];
    if (env && canEdit) rows.push(blankRow());
  }
  function blankRow(): VarRow {
    return { key: '', value: '', secret: false, touched: false, storedKey: null };
  }
  $effect(() => {
    const env = selected;
    if ((env?.id ?? null) !== loadedFor) seed(env);
  });

  async function pick(env: ApiEnvironment): Promise<void> {
    if (env.id === selected?.id) return;
    if (dirty && !(await confirmer.ask(`Discard your unsaved changes to “${selected?.name}”?`, { title: 'Discard changes', confirmLabel: 'Discard' }))) return;
    selectedId = env.id;
    seed(env);
  }

  function updateRow(i: number, patch: Partial<VarRow>): void {
    rows = rows.map((r, idx) => (idx === i ? { ...r, ...patch, ...(patch.value !== undefined ? { touched: true } : {}) } : r));
    dirty = true;
    // Keep one empty row at the end so adding a variable is just typing.
    const last = rows[rows.length - 1];
    if (canEdit && last && (last.key !== '' || last.value !== '')) rows = [...rows, blankRow()];
  }
  function removeRow(i: number): void {
    rows = rows.filter((_, idx) => idx !== i);
    dirty = true;
    if (canEdit && !rows.some((r) => r.key === '' && r.value === '')) rows = [...rows, blankRow()];
  }
  /** Marking secret moves the value to the Keychain on save; making a STORED
   *  secret plain deletes its Keychain value unless it is typed again — ask. */
  async function toggleSecret(i: number): Promise<void> {
    const row = rows[i];
    if (!row) return;
    if (row.secret && row.storedKey !== null && !(row.touched && row.value !== '')) {
      const ok = await confirmer.ask(
        `“${row.key || row.storedKey}” is stored in the Keychain and can’t be shown. Making it a plain variable deletes the stored value when you save, so type the value again to keep it.`,
        { title: 'Make variable plain?', confirmLabel: 'Make plain' },
      );
      if (!ok) return;
    }
    rows = rows.map((r, idx) => (idx === i ? { ...r, secret: !r.secret } : r));
    dirty = true;
  }

  let saving = $state(false);
  async function save(): Promise<void> {
    const env = selected;
    if (!env) return;
    const variables: Record<string, string> = {};
    const secret_keys: string[] = [];
    const secret_values: Record<string, string> = {};
    const secret_renames: Record<string, string> = {};
    for (const r of rows) {
      const key = r.key.trim();
      if (key === '') continue;
      if (r.secret) {
        secret_keys.push(key);
        // Only new/changed values travel; untouched secrets keep their stored value.
        if (r.touched && r.value !== '') secret_values[key] = r.value;
        // A renamed stored secret: its Keychain value moves to the new name.
        else if (r.storedKey !== null && r.storedKey !== key) secret_renames[r.storedKey] = key;
      } else {
        variables[key] = r.value;
      }
    }
    saving = true;
    const saved = await apiClient.saveEnvironment({ name: env.name, variables, secret_keys, secret_values, secret_renames }, env.id);
    saving = false;
    if (saved) seed(saved);
  }

  async function create(): Promise<void> {
    const name = await confirmer.promptText('Name', { title: 'New environment', confirmLabel: 'Create', initial: '' });
    if (!name) return;
    const saved = await apiClient.saveEnvironment({ name }, undefined);
    if (saved) {
      selectedId = saved.id;
      seed(saved);
    }
  }
  async function rename(env: ApiEnvironment): Promise<void> {
    const name = await confirmer.promptText('Name', { title: 'Rename environment', confirmLabel: 'Rename', initial: env.name });
    if (!name || name === env.name) return;
    // Carry secret_keys through — omitting them would un-mark every secret.
    await apiClient.saveEnvironment({ name, variables: env.variables, secret_keys: env.secret_keys }, env.id);
  }
  async function remove(env: ApiEnvironment): Promise<void> {
    const secrets = env.secret_keys.length ? ` Its ${env.secret_keys.length} secret value${env.secret_keys.length === 1 ? ' is' : 's are'} removed from the Keychain.` : '';
    if (!(await confirmer.ask(`Delete the environment “${env.name}”?${secrets} Requests that use its {{variables}} will send them unresolved.`, { title: 'Delete environment' }))) return;
    await apiClient.deleteEnvironment(env.id);
    selectedId = null;
  }
  async function secureAll(): Promise<void> {
    if (!(await confirmer.ask(
      'Move every plaintext credential in this workspace into the macOS Keychain: auth tokens and passwords in saved requests, and environment variables whose names look secret (token, secret, password, api key…). They are masked from then on.',
      { title: 'Secure plaintext secrets', confirmLabel: 'Secure secrets', danger: false },
    ))) return;
    await apiClient.secureAll();
  }
  function menu(e: MouseEvent, env: ApiEnvironment): void {
    ctxMenu.show(e, [
      { label: 'Rename…', icon: 'edit', action: () => void rename(env), disabled: !canEdit },
      { label: 'Secure plaintext secrets…', icon: 'shield', action: () => void secureAll(), disabled: !canEdit },
      { separator: true },
      { label: 'Delete…', icon: 'trash', danger: true, action: () => void remove(env), disabled: !canEdit },
    ]);
  }
  const varCount = (env: ApiEnvironment) => Object.keys(env.variables).length + env.secret_keys.length;
</script>

<div class="envs">
  <header class="view-head">
    <div class="vh-text">
      <h2>Environments</h2>
      <p>
        Reusable values such as a base URL or a token. Write <code>{'{{name}}'}</code> in a URL, header, body or auth
        field and Otto fills in the value from the <strong>active</strong> environment when it sends. Secret values
        live in the macOS Keychain: they’re never shown again, and only sent to hosts they were used with.
      </p>
    </div>
    {#if canEdit}
      <button class="btn small" onclick={create}><Icon name="plus" size={12} />New environment</button>
    {/if}
  </header>

  {#if apiClient.environments.length === 0 && (apiClient.loadError || apiClient.loading)}
    <!-- A failed/in-flight load is not "No environments yet". -->
    <LoadState
      what="environments"
      loading={apiClient.loading}
      error={apiClient.loadError}
      empty
      onretry={() => void apiClient.loadAll()}
    />
  {:else if apiClient.environments.length === 0}
    <EmptyState
      icon="globe"
      title="No environments yet"
      body="Create one per place you call, for example Staging and Production, each with its own base_url and api_token."
      actionLabel={canEdit ? 'New environment' : undefined}
      actionIcon="plus"
      onaction={canEdit ? create : undefined}
    />
  {:else}
    <div class="split">
      <ul class="env-list" aria-label="Environments">
        {#each apiClient.environments as env (env.id)}
          <li class="env-item" class:sel={env.id === selected?.id}>
            <button class="env-pick" onclick={() => void pick(env)} aria-current={env.id === selected?.id ? 'true' : undefined}>
              <span class="env-name">{env.name}</span>
              <span class="env-meta">
                {varCount(env)} variable{varCount(env) === 1 ? '' : 's'}{env.secret_keys.length ? ` · ${env.secret_keys.length} secret` : ''}
              </span>
            </button>
            {#if env.is_active}<span class="chip ok active-chip" title="Used when you send">Active</span>{/if}
          </li>
        {/each}
      </ul>

      {#if selected}
        <section class="editor" aria-label="Variables in {selected.name}">
          <div class="ed-head">
            <h3>{selected.name}</h3>
            {#if selected.is_active}
              <span class="dim">Active: requests use these values</span>
            {:else}
              <button class="btn small" onclick={() => void apiClient.activateEnvironment(selected.id)} disabled={!canEdit}>Make active</button>
            {/if}
            <span class="grow"></span>
            <button class="icon-btn" onclick={(e) => menu(e, selected)} aria-label="Environment options" title="Environment options"><Icon name="more" size={14} /></button>
            <button class="btn small primary" onclick={save} disabled={!canEdit || !dirty || saving}>{saving ? 'Saving…' : 'Save changes'}</button>
          </div>

          <div class="var-table">
            <div class="vt-row vt-head">
              <span>Name</span>
              <span>Value</span>
              <span class="vt-secret" aria-hidden="true">Secret</span>
              <span></span>
            </div>
            {#each rows as row, i (i)}
              <div class="vt-row var-row">
                <input class="input mono var-key" placeholder={i === rows.length - 1 ? 'new_variable' : 'name'} value={row.key} disabled={!canEdit}
                  aria-label="Variable name" oninput={(e) => updateRow(i, { key: (e.currentTarget as HTMLInputElement).value })} />
                <input
                  class="input mono var-val"
                 
                  type={row.secret ? 'password' : 'text'}
                  placeholder={row.secret && !row.touched ? '•••••• stored in Keychain — type to replace' : 'value'}
                  value={row.value}
                  disabled={!canEdit}
                  aria-label="Value of {row.key || 'new variable'}"
                  oninput={(e) => updateRow(i, { value: (e.currentTarget as HTMLInputElement).value })}
                />
                <span class="vt-secret">
                  <button class="icon-btn" class:on={row.secret} disabled={!canEdit}
                    title={row.secret ? 'Secret: the value is kept in the Keychain. Click to make it plain.' : 'Make secret: move the value to the Keychain when you save'}
                    aria-label="Toggle secret" aria-pressed={row.secret} onclick={() => void toggleSecret(i)}>
                    <Icon name={row.secret ? 'lock' : 'unlock'} size={14} />
                  </button>
                </span>
                <span>
                  {#if canEdit && !(i === rows.length - 1 && row.key === '' && row.value === '')}
                    <button class="icon-btn" title="Remove variable" aria-label="Remove variable" onclick={() => removeRow(i)}><Icon name="x" size={12} /></button>
                  {/if}
                </span>
              </div>
            {/each}
          </div>
          <p class="foot">Use a variable as <code>{'{{base_url}}'}</code>. Session variables (set by scripts or in a request’s ⋯ menu) override these for the current session only.</p>
        </section>
      {/if}
    </div>
  {/if}
</div>

<style>
  .envs {
    display: flex;
    flex-direction: column;
    gap: 16px;
    padding: 16px 20px 24px;
    min-height: 0;
    overflow-y: auto;
    flex: 1;
    container-type: inline-size;
  }
  .view-head {
    display: flex;
    align-items: flex-start;
    gap: 16px;
  }
  .vh-text {
    flex: 1;
    min-width: 0;
  }
  h2 {
    margin: 0 0 4px;
    font-size: var(--fs-l);
    font-weight: 600;
  }
  .view-head p {
    margin: 0;
    max-width: 760px;
    font-size: var(--fs-s);
    line-height: 1.5;
    color: var(--text-dim);
  }
  code {
    font-family: var(--font-mono);
    font-size: var(--fs-s);
    color: var(--text);
  }
  .split {
    display: grid;
    grid-template-columns: 220px minmax(0, 1fr);
    gap: 16px;
    align-items: start;
  }
  @container (max-width: 640px) {
    .split {
      grid-template-columns: minmax(0, 1fr);
    }
  }
  .env-list {
    list-style: none;
    margin: 0;
    padding: 4px;
    display: flex;
    flex-direction: column;
    gap: 2px;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface);
  }
  .env-item {
    display: flex;
    align-items: center;
    gap: 4px;
    padding-inline-end: 6px;
    border-radius: var(--radius-s);
  }
  .env-item:hover {
    background: var(--hover);
  }
  .env-item.sel {
    background: var(--accent-soft);
  }
  .env-pick {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 1px;
    padding: 6px 8px;
    border: none;
    background: transparent;
    color: var(--text);
    cursor: pointer;
    text-align: start;
    border-radius: var(--radius-s);
  }
  .env-name {
    font-size: var(--fs-m);
    font-weight: 500;
  }
  .env-meta {
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .active-chip {
    flex-shrink: 0;
  }
  .editor {
    display: flex;
    flex-direction: column;
    gap: 10px;
    min-width: 0;
  }
  .ed-head {
    display: flex;
    align-items: center;
    gap: 10px;
    flex-wrap: wrap;
  }
  h3 {
    margin: 0;
    font-size: var(--fs-m);
    font-weight: 600;
  }
  .dim {
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .var-table {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .vt-row {
    display: grid;
    grid-template-columns: minmax(120px, 30%) minmax(0, 1fr) 52px 28px;
    gap: 6px;
    align-items: center;
  }
  .vt-head {
    font-size: var(--fs-xs);
    font-weight: 600;
    color: var(--text-dim);
    padding: 0 2px;
  }
  .vt-secret {
    display: flex;
    justify-content: center;
  }
  .var-key,
  .var-val {
    min-width: 0;
    width: 100%;
    font-size: var(--fs-s);
  }
  .icon-btn.on {
    color: var(--accent-text);
    background: var(--accent-soft);
  }
  .foot {
    margin: 4px 0 0;
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  @media (max-width: 640px) {
    .envs {
      padding: 12px 14px 24px;
    }
    .view-head {
      flex-direction: column;
    }
  }
</style>
