<script lang="ts">
  // Per-workspace allow/deny entries over this workspace's own servers' tools.
  // A `deny` always wins; a tool-less entry covers the whole server; with no
  // match the server's `default_tool_access` applies. Edited as a grid, saved in
  // one bulk PUT (the server replaces the whole workspace allowlist).
  import Icon from '../../lib/components/Icon.svelte';
  import { mcpCpApi } from '../../lib/api/mcp';
  import { toasts } from '../../lib/toast.svelte';
  import type { McpServerDetail, McpToolAccess } from '../../lib/api/types';

  interface Props {
    wsId: string;
    servers: McpServerDetail[];
  }
  let { wsId, servers }: Props = $props();

  interface Row {
    server_id: string;
    tool_name: string;
    mode: McpToolAccess;
  }
  let rows = $state<Row[]>([]);
  let loading = $state(false);
  let saving = $state(false);

  async function load(): Promise<void> {
    loading = true;
    try {
      const entries = await mcpCpApi.cpAllowlist(wsId);
      rows = entries.map((e) => ({
        server_id: e.server_id,
        tool_name: e.tool_name ?? '',
        mode: e.mode,
      }));
    } catch (e) {
      toasts.error('Failed to load allowlist', e instanceof Error ? e.message : String(e));
    } finally {
      loading = false;
    }
  }

  $effect(() => {
    void wsId;
    void load();
  });

  function addRow(): void {
    rows = [...rows, { server_id: servers[0]?.id ?? '', tool_name: '', mode: 'allow' }];
  }
  function removeRow(i: number): void {
    rows = rows.filter((_, idx) => idx !== i);
  }

  async function save(): Promise<void> {
    const entries = rows
      .filter((r) => r.server_id)
      .map((r) => ({
        server_id: r.server_id,
        tool_name: r.tool_name.trim() || null,
        mode: r.mode,
      }));
    saving = true;
    try {
      await mcpCpApi.cpSetAllowlist(wsId, { entries });
      toasts.success('Allowlist saved', `${entries.length} entr${entries.length === 1 ? 'y' : 'ies'}`);
      await load();
    } catch (e) {
      toasts.error('Save failed', e instanceof Error ? e.message : String(e));
    } finally {
      saving = false;
    }
  }
</script>

<div class="al">
  <div class="bar">
    <span class="hint">
      A <strong>deny</strong> wins over an allow; leave the tool blank to cover the whole server. No
      match falls back to the server's default tool access.
    </span>
    <span class="grow"></span>
    <button class="btn small" onclick={addRow} disabled={servers.length === 0}>
      <Icon name="plus" size={13} /> Add entry
    </button>
    <button class="btn primary small" onclick={() => void save()} disabled={saving}>
      {saving ? 'Saving…' : 'Save'}
    </button>
  </div>

  {#if servers.length === 0}
    <p class="muted pad">Add a server first — the allowlist scopes its tools.</p>
  {:else if loading && rows.length === 0}
    <p class="muted pad">Loading…</p>
  {:else if rows.length === 0}
    <div class="empty">
      <Icon name="eye" size={22} />
      <p>No allowlist entries. Without any, each server's default tool access applies.</p>
      <button class="btn primary" onclick={addRow}>Add an entry</button>
    </div>
  {:else}
    <div class="grid">
      <div class="thead">
        <span>Server</span>
        <span>Tool <em>(blank = whole server)</em></span>
        <span>Mode</span>
        <span></span>
      </div>
      {#each rows as r, i (i)}
        <div class="arow">
          <select bind:value={r.server_id}>
            {#each servers as s (s.id)}
              <option value={s.id}>{s.name}</option>
            {/each}
          </select>
          <input bind:value={r.tool_name} placeholder="(whole server)" class="mono" />
          <select bind:value={r.mode}>
            <option value="allow">allow</option>
            <option value="deny">deny</option>
          </select>
          <button class="icon danger" onclick={() => removeRow(i)} title="Remove entry" aria-label="Remove entry">
            <Icon name="trash" size={14} />
          </button>
        </div>
      {/each}
    </div>
  {/if}
</div>

<style>
  .al {
    display: flex;
    flex-direction: column;
    min-height: 0;
  }
  .bar {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 10px 14px;
    border-bottom: 1px solid var(--border);
  }
  .hint {
    font-size: 12px;
    color: var(--text-dim);
    max-width: 540px;
  }
  .grow {
    flex: 1;
  }
  .grid {
    overflow: auto;
  }
  .thead,
  .arow {
    display: grid;
    grid-template-columns: minmax(160px, 1fr) minmax(160px, 1.4fr) 110px 40px;
    align-items: center;
    gap: 8px;
    padding: 7px 14px;
  }
  .thead {
    border-bottom: 1px solid var(--border);
    font-size: 11px;
    text-transform: uppercase;
    letter-spacing: 0.03em;
    color: var(--text-dim);
  }
  .thead em {
    font-style: normal;
    opacity: 0.7;
  }
  .arow {
    border-bottom: 1px solid color-mix(in srgb, var(--border) 60%, transparent);
  }
  select,
  input {
    width: 100%;
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: var(--radius-s, 6px);
    color: var(--text);
    padding: 6px 8px;
    font-size: 12.5px;
  }
  .mono {
    font-family: var(--font-mono);
  }
  .icon {
    border: none;
    background: transparent;
    color: var(--text-dim);
    cursor: pointer;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 4px;
  }
  .icon.danger:hover {
    color: var(--status-exited, #ff5f57);
  }
  .muted {
    color: var(--text-dim);
  }
  .pad {
    padding: 16px;
  }
  .empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 10px;
    color: var(--text-dim);
    text-align: center;
    padding: 36px 24px;
  }
</style>
