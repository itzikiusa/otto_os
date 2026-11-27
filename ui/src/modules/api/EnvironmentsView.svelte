<script lang="ts">
  // Environments, edited in the main area (they used to be squeezed into the
  // 280 px sidebar). A list of environments on the left — the active one is
  // what {{variables}} resolve against when you send — and the selected
  // environment's variables as a full-width table on the right.
  import Icon from '../../lib/components/Icon.svelte';
  import { focusAfterRowRemoval, rowListOf } from '../../lib/rowFocus';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import ListDetail from '../../lib/components/side/ListDetail.svelte';
  import ListPane from '../../lib/components/side/ListPane.svelte';
  import SideRow from '../../lib/components/side/SideRow.svelte';
  import { plural } from '../../lib/plural';
  import { apiClient } from '../../lib/stores/apiClient.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import { ctxMenu } from '../../lib/contextmenu.svelte';
  import { router } from '../../lib/router.svelte';
  import type { ApiEnvironment, Id } from '../../lib/api/types';

  interface Props {
    /** Preselect this environment (e.g. from the header switcher). */
    initialId?: Id | null;
  }
  let { initialId = null }: Props = $props();

  const canEdit = $derived(ws.myRole !== 'viewer');
  // svelte-ignore state_referenced_locally
  let selectedId = $state<Id | null>(initialId);
  // Phone: the list is the page until an environment is opened ("‹ Environments").
  // svelte-ignore state_referenced_locally
  let phoneOpen = $state(initialId !== null);
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
  interface VarRow { id: number; key: string; value: string; secret: boolean; touched: boolean; storedKey: string | null; }
  let nextRowId = 1;
  let rows: VarRow[] = $state([]);
  let dirty = $state(false);
  let loadedFor = $state<string | null>(null);

  let leavePending: Promise<boolean> | null = null;
  export function approveLeave(): Promise<boolean> {
    if (!dirty) return Promise.resolve(true);
    if (leavePending) return leavePending;
    leavePending = (async () => {
      const { value } = await confirmer.choose(`“${selected?.name ?? 'This environment'}” has unsaved changes.`, {
        title: 'Save environment changes?',
        options: [{ label: 'Save', value: 'save', kind: 'primary' }, { label: 'Discard', value: 'discard', kind: 'danger' }],
      });
      if (value === 'save') return save();
      if (value !== 'discard') return false;
      dirty = false;
      return true;
    })().finally(() => { leavePending = null; });
    return leavePending;
  }
  // The router asks these same guards before module and workspace changes.
  // Secret drafts stay only in this mounted editor, never browser storage.
  $effect(() => router.guard(() => approveLeave()));

  // Selection/reseed ownership is distinct from edits within the same draft.
  let editorGeneration = 0;
  function seed(env: ApiEnvironment | null): void {
    editorGeneration++;
    loadedFor = env?.id ?? null;
    dirty = false;
    rows = env
      ? [
          ...Object.entries(env.variables).map(([key, value]) => ({ id: nextRowId++, key, value, secret: false, touched: false, storedKey: null })),
          ...env.secret_keys.map((key) => ({ id: nextRowId++, key, value: '', secret: true, touched: false, storedKey: key })),
        ]
      : [];
    if (env && canEdit) rows.push(blankRow());
  }
  function blankRow(): VarRow {
    return { id: nextRowId++, key: '', value: '', secret: false, touched: false, storedKey: null };
  }
  $effect(() => {
    const env = selected;
    if ((env?.id ?? null) !== loadedFor) seed(env);
  });

  async function pick(env: ApiEnvironment): Promise<void> {
    if (env.id === selected?.id) {
      phoneOpen = true;
      return;
    }
    if (!(await approveLeave())) return;
    phoneOpen = true;
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
  function removeRow(i: number, e?: Event): void {
    const list = e ? rowListOf(e, '.var-table') : null;
    rows = rows.filter((_, idx) => idx !== i);
    dirty = true;
    if (canEdit && !rows.some((r) => r.key === '' && r.value === '')) rows = [...rows, blankRow()];
    void focusAfterRowRemoval(list, i, '.var-key');
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
  let savePending: Promise<boolean> | null = null;
  function save(): Promise<boolean> {
    // A leave decision made during a normal Save waits for that same write.
    if (savePending) return savePending;
    savePending = saveDraft().finally(() => { savePending = null; saving = false; });
    return savePending;
  }
  async function saveDraft(): Promise<boolean> {
    const env = selected;
    if (!env) return false;
    const submittedRows = rows;
    const submittedGeneration = editorGeneration;
    const workspaceId = ws.currentId;
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
    if (!saved || selected?.id !== env.id || loadedFor !== env.id || ws.currentId !== workspaceId) return false;
    if (rows === submittedRows || (editorGeneration !== submittedGeneration && !dirty)) {
      seed(saved);
    } else {
      // An unchanged secret row can drop its submitted plaintext. A secret
      // renamed again while saving must now refer to the first persisted name.
      // Keep all newer keys/values and the dirty flag for the next Save.
      rows = rows.map((row) => {
        if (!row.secret) return row;
        const submitted = submittedRows.find((r) => r === row || (r.storedKey !== null && r.storedKey === row.storedKey));
        const storedKey = submitted?.key.trim();
        if (!submitted || !storedKey || !saved.secret_keys.includes(storedKey)) return row;
        return row === submitted
          ? { ...row, storedKey, touched: false, value: '' }
          : { ...row, storedKey };
      });
    }
    // A successful write only approves leaving when it saved the whole
    // current draft; edits made while awaiting it still belong to the user.
    return !dirty;
  }

  export async function create(): Promise<void> {
    if (!(await approveLeave())) return;
    const generation = editorGeneration;
    const workspaceId = ws.currentId;
    const name = await confirmer.promptText('Name', { title: 'New environment', confirmLabel: 'Create', initial: '' });
    if (!name || ws.currentId !== workspaceId) return;
    const saved = await apiClient.saveEnvironment({ name }, undefined);
    // Creating another environment must not overwrite edits or a selection
    // made while its request was in flight.
    if (saved && !dirty && editorGeneration === generation && ws.currentId === workspaceId) {
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

<ListDetail
  module="api"
  pane="environments"
  name="Environments"
  open={phoneOpen}
  onback={() => (phoneOpen = false)}
  empty={apiClient.environments.length === 0 && !apiClient.listsPending && !apiClient.envLoadError}
  legacy={{ w: 'api.envListW' }}
>
  {#snippet list()}
    <!-- A failed/in-flight load is not "No environments yet". -->
    <ListPane
      count={apiClient.environments.length}
      primary={canEdit ? { icon: 'plus', label: 'New environment', onclick: () => void create(), testid: 'api-new-environment' } : undefined}
      loading={apiClient.environments.length === 0 && apiClient.listsPending}
      error={apiClient.environments.length === 0 ? apiClient.envLoadError : null}
      onretry={() => void apiClient.loadAll()}
    >
      {#each apiClient.environments as env (env.id)}
        <SideRow
          title={env.name}
          meta="{plural(varCount(env), 'variable')}{env.secret_keys.length ? ` · ${plural(env.secret_keys.length, 'secret')}` : ''}"
          selected={env.id === selected?.id}
          testid="api-env-row"
          onclick={() => void pick(env)}
        >
          {#snippet trailing()}{#if env.is_active}<span class="chip ok" title="Used when you send">Active</span>{/if}{/snippet}
        </SideRow>
      {/each}
    </ListPane>
  {/snippet}

  {#if apiClient.environments.length === 0}
    {#if !apiClient.listsPending && !apiClient.envLoadError}<EmptyState
      variant="page"
      icon="globe"
      title="No environments yet"
      body="Create one per place you call, for example Staging and Production, each with its own base_url and api_token."
      actionLabel={canEdit ? 'New environment' : undefined}
      actionIcon="plus"
      onaction={canEdit ? create : undefined}
    />{/if}
  {:else}
    <div class="envs">
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

          <p class="intro">
            Reusable values such as a base URL or a token. Write <code>{'{{name}}'}</code> in a URL, header, body or auth
            field and Otto fills in the value from the <strong>active</strong> environment when it sends. Secret values
            live in the macOS Keychain: they’re never shown again, and only sent to hosts they were used with.
          </p>

          <div class="var-table">
            <div class="vt-row vt-head">
              <span>Name</span>
              <span>Value</span>
              <span class="vt-secret" aria-hidden="true">Secret</span>
              <span></span>
            </div>
            {#each rows as row, i (row.id)}
              <div class="vt-row var-row">
                <input dir="ltr" class="input mono var-key" placeholder="base_url" value={row.key} disabled={!canEdit}
                  aria-label="Variable name" oninput={(e) => updateRow(i, { key: (e.currentTarget as HTMLInputElement).value })} />
                <input
                  class="input mono var-val"
                 
                  type={row.secret ? 'password' : 'text'}
                  placeholder={row.secret && !row.touched ? '•••••• stored in Keychain — type to replace' : 'https://api.example.com'}
                  value={row.value}
                  disabled={!canEdit}
                  aria-label="Value of {row.key || 'new variable'}"
                  oninput={(e) => updateRow(i, { value: (e.currentTarget as HTMLInputElement).value })}
                />
                <span class="vt-secret">
                  <button class="icon-btn" class:on={row.secret} disabled={!canEdit}
                    title="Keep the value in the Keychain"
                    aria-label="Keep the value in the Keychain" aria-pressed={row.secret} onclick={() => void toggleSecret(i)}>
                    <Icon name={row.secret ? 'lock' : 'unlock'} size={14} />
                  </button>
                </span>
                <span>
                  {#if canEdit && !(i === rows.length - 1 && row.key === '' && row.value === '')}
                    <button class="icon-btn" data-remove-row title="Remove variable {row.key || i + 1}" aria-label="Remove variable {row.key || i + 1}" onclick={(e) => removeRow(i, e)}><Icon name="x" size={12} /></button>
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
</ListDetail>

<style>
  .envs {
    display: flex;
    flex-direction: column;
    gap: 16px;
    padding: 16px 20px 24px;
    min-height: 0;
    overflow-y: auto;
    flex: 1;
  }
  .intro {
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
  }
</style>
