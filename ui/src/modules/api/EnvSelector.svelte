<script lang="ts">
  // Environment picker: dropdown of environments with an active indicator,
  // create/activate/delete, and an inline variable editor (key/value rows).
  import Icon from '../../lib/components/Icon.svelte';
  import { apiClient } from '../../lib/stores/apiClient.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import type { ApiEnvironment } from '../../lib/api/types';

  interface Props {
    /** Compact mode hides the variable editor (panel use). */
    compact?: boolean;
  }
  let { compact = false }: Props = $props();

  const canEdit = $derived(ws.myRole !== 'viewer');
  let editing: string | null = $state(null);

  // Local key/value rows for the env being edited.
  interface VarRow { key: string; value: string; }
  let rows: VarRow[] = $state([]);

  function startEdit(env: ApiEnvironment): void {
    editing = editing === env.id ? null : env.id;
    if (editing) {
      rows = Object.entries(env.variables).map(([key, value]) => ({ key, value }));
    }
  }

  function addRow(): void {
    rows = [...rows, { key: '', value: '' }];
  }
  function updateRow(i: number, patch: Partial<VarRow>): void {
    rows = rows.map((r, idx) => (idx === i ? { ...r, ...patch } : r));
  }
  function removeRow(i: number): void {
    rows = rows.filter((_, idx) => idx !== i);
  }

  async function saveVars(env: ApiEnvironment): Promise<void> {
    const variables: Record<string, string> = {};
    for (const r of rows) if (r.key.trim() !== '') variables[r.key.trim()] = r.value;
    const saved = await apiClient.saveEnvironment({ name: env.name, variables }, env.id);
    if (saved) editing = null;
  }

  async function create(): Promise<void> {
    if (!canEdit) return;
    const name = await confirmer.promptText('Environment name', {
      title: 'New environment',
      confirmLabel: 'Create',
    });
    if (!name) return;
    await apiClient.saveEnvironment({ name }, undefined);
  }

  async function rename(env: ApiEnvironment): Promise<void> {
    if (!canEdit) return;
    const name = await confirmer.promptText('Rename environment', {
      title: 'Rename environment',
      confirmLabel: 'Rename',
      initial: env.name,
    });
    if (!name || name === env.name) return;
    await apiClient.saveEnvironment({ name, variables: env.variables }, env.id);
  }

  async function remove(env: ApiEnvironment): Promise<void> {
    if (!canEdit) return;
    if (!(await confirmer.ask(`Delete environment “${env.name}”?`, { title: 'Delete environment' }))) return;
    if (editing === env.id) editing = null;
    await apiClient.deleteEnvironment(env.id);
  }

  function activate(env: ApiEnvironment): void {
    void apiClient.activateEnvironment(env.id);
  }
</script>

<div class="env-wrap">
  <div class="env-head">
    <span class="env-title">Environments</span>
    {#if canEdit}
      <button class="icon-btn" title="New environment" aria-label="New environment" onclick={create}><Icon name="plus" size={13} /></button>
    {/if}
  </div>

  {#if apiClient.environments.length === 0}
    <div class="empty-mini">No environments. Variables let you reuse {'{{base_url}}'} etc.</div>
  {:else}
    <div class="env-list">
      {#each apiClient.environments as env (env.id)}
        <div class="env-item">
          <div class="env-row">
            <button
              class="env-pick"
              class:active={env.is_active}
              onclick={() => activate(env)}
              title={env.is_active ? 'Active' : 'Set active'}
            >
              <span class="radio" class:on={env.is_active}>
                {#if env.is_active}<Icon name="check" size={10} />{/if}
              </span>
              <span class="env-name ellipsis grow">{env.name}</span>
              <span class="env-count">{Object.keys(env.variables).length} vars</span>
            </button>
            {#if !compact && canEdit}
              <button class="icon-btn" title="Edit variables" aria-label="Edit variables" onclick={() => startEdit(env)}><Icon name="edit" size={12} /></button>
              <button class="icon-btn" title="Rename" aria-label="Rename" onclick={() => rename(env)}><Icon name="note" size={12} /></button>
              <button class="icon-btn" title="Delete" aria-label="Delete" onclick={() => remove(env)}><Icon name="trash" size={12} /></button>
            {/if}
          </div>

          {#if !compact && editing === env.id}
            <div class="var-editor">
              {#each rows as row, i (i)}
                <div class="var-row">
                  <input class="input var-key mono" placeholder="key" value={row.key} oninput={(e) => updateRow(i, { key: (e.currentTarget as HTMLInputElement).value })} />
                  <input class="input var-val mono" placeholder="value" value={row.value} oninput={(e) => updateRow(i, { value: (e.currentTarget as HTMLInputElement).value })} />
                  <button class="icon-btn" title="Remove" aria-label="Remove variable" onclick={() => removeRow(i)}><Icon name="x" size={12} /></button>
                </div>
              {/each}
              <div class="var-actions">
                <button class="btn small ghost" onclick={addRow}><Icon name="plus" size={11} />Add</button>
                <span class="grow"></span>
                <button class="btn small primary" onclick={() => saveVars(env)}>Save vars</button>
              </div>
            </div>
          {/if}
        </div>
      {/each}
    </div>
  {/if}
</div>

<style>
  .env-wrap {
    display: flex;
    flex-direction: column;
    min-height: 0;
  }
  .env-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 0 2px 6px;
  }
  .env-title {
    font-size: 11px;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--text-dim);
  }
  .env-list {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .env-row {
    display: flex;
    align-items: center;
    gap: 2px;
  }
  .env-pick {
    display: flex;
    align-items: center;
    gap: 8px;
    flex: 1;
    min-width: 0;
    height: 28px;
    padding: 0 6px;
    border: none;
    background: transparent;
    color: var(--text);
    cursor: pointer;
    border-radius: var(--radius-s);
    text-align: left;
  }
  .env-pick:hover {
    background: color-mix(in srgb, var(--text-dim) 8%, transparent);
  }
  .env-pick.active {
    background: color-mix(in srgb, var(--accent) 12%, transparent);
  }
  .radio {
    display: grid;
    place-items: center;
    width: 14px;
    height: 14px;
    border-radius: 50%;
    border: 1.5px solid var(--text-dim);
    color: var(--accent-contrast);
    flex-shrink: 0;
  }
  .radio.on {
    background: var(--accent);
    border-color: var(--accent);
  }
  .env-name {
    font-size: 12.5px;
    font-weight: 500;
    min-width: 0;
  }
  .env-count {
    font-size: 10px;
    color: var(--text-dim);
    flex-shrink: 0;
  }
  .var-editor {
    display: flex;
    flex-direction: column;
    gap: 5px;
    padding: 6px 4px 10px 22px;
  }
  .var-row {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .var-key {
    flex: 0 1 40%;
    min-width: 0;
  }
  .var-val {
    flex: 1;
    min-width: 0;
  }
  .var-actions {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .empty-mini {
    font-size: 12px;
    color: var(--text-dim);
    padding: 8px 2px;
    line-height: 1.5;
  }
  .ellipsis {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>
