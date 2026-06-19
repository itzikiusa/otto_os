<script lang="ts">
  // MCP Servers settings page: per-workspace, user-managed MCP servers that Otto
  // merges into the workspace's `.mcp.json` when an agent session spawns there
  // (alongside Otto's own managed entries, e.g. the browser server). Nothing is
  // auto-enabled — each server is off until you flip it on, and it's only written
  // to `.mcp.json` the next time a session spawns in the workspace.
  import { mcpApi } from '../../lib/api/mcp';
  import type { McpServer, CreateMcpServerReq } from '../../lib/api/types';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import Skeleton from '../../lib/components/Skeleton.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';

  let servers: McpServer[] = $state([]);
  let loading = $state(false);
  let busyId: string | null = $state(null);

  // Add/edit form state. `editing` holds the id being edited (null = creating a
  // new one once the form is open).
  let formOpen = $state(false);
  let editing: string | null = $state(null);
  let fName = $state('');
  let fCommand = $state('');
  let fArgs = $state(''); // one arg per line
  let fEnv = $state(''); // KEY=value per line
  let fEnabled = $state(false);
  let saving = $state(false);

  const wsId = $derived(ws.currentId);

  function errMsg(e: unknown): string {
    return e instanceof Error ? e.message : String(e);
  }

  $effect(() => {
    if (wsId) void load(wsId);
  });

  async function load(id: string): Promise<void> {
    loading = true;
    try {
      servers = await mcpApi.list(id);
    } catch (e) {
      toasts.error('Could not load MCP servers', errMsg(e));
    } finally {
      loading = false;
    }
  }

  function resetForm(): void {
    editing = null;
    fName = '';
    fCommand = '';
    fArgs = '';
    fEnv = '';
    fEnabled = false;
  }

  function openCreate(): void {
    resetForm();
    formOpen = true;
  }

  function openEdit(s: McpServer): void {
    editing = s.id;
    fName = s.name;
    fCommand = s.command;
    fArgs = s.args.join('\n');
    fEnv = Object.entries(s.env)
      .map(([k, v]) => `${k}=${v}`)
      .join('\n');
    fEnabled = s.enabled;
    formOpen = true;
  }

  function closeForm(): void {
    formOpen = false;
    resetForm();
  }

  // Split a multi-line textarea into trimmed, non-empty lines.
  function lines(text: string): string[] {
    return text
      .split('\n')
      .map((l) => l.trim())
      .filter(Boolean);
  }

  // Parse "KEY=value" lines into an object (first '=' splits; later ones kept).
  function parseEnv(text: string): Record<string, string> {
    const out: Record<string, string> = {};
    for (const line of lines(text)) {
      const eq = line.indexOf('=');
      if (eq <= 0) continue; // skip lines without a key
      out[line.slice(0, eq).trim()] = line.slice(eq + 1).trim();
    }
    return out;
  }

  async function save(): Promise<void> {
    if (!wsId) return;
    const name = fName.trim();
    const command = fCommand.trim();
    if (!name || !command) {
      toasts.error('Name and command are required');
      return;
    }
    const body: CreateMcpServerReq = {
      name,
      command,
      args: lines(fArgs),
      env: parseEnv(fEnv),
      enabled: fEnabled,
    };
    saving = true;
    try {
      if (editing) {
        await mcpApi.update(editing, body);
        toasts.success('MCP server updated', name);
      } else {
        await mcpApi.create(wsId, body);
        toasts.success('MCP server added', name);
      }
      closeForm();
      await load(wsId);
    } catch (e) {
      toasts.error('Save failed', errMsg(e));
    } finally {
      saving = false;
    }
  }

  async function toggleEnabled(s: McpServer): Promise<void> {
    if (!wsId) return;
    busyId = s.id;
    try {
      await mcpApi.update(s.id, { enabled: !s.enabled });
      await load(wsId);
    } catch (e) {
      toasts.error('Could not update MCP server', errMsg(e));
    } finally {
      busyId = null;
    }
  }

  async function remove(s: McpServer): Promise<void> {
    if (!wsId) return;
    if (!confirm(`Remove MCP server "${s.name}"?`)) return;
    busyId = s.id;
    try {
      await mcpApi.remove(s.id);
      toasts.info('MCP server removed', s.name);
      await load(wsId);
    } catch (e) {
      toasts.error('Could not remove MCP server', errMsg(e));
    } finally {
      busyId = null;
    }
  }
</script>

<div class="page">
  <div class="page-header">
    <div>
      <h1>MCP Servers</h1>
      <div class="sub">
        Per-workspace Model Context Protocol servers. Enabled servers are merged into this
        workspace's <code>.mcp.json</code> when an agent session spawns here, alongside Otto's own
        managed entries (e.g. the browser). Nothing is auto-enabled — a server is only written once
        you turn it on.
      </div>
    </div>
    {#if wsId}
      <button class="btn primary" onclick={openCreate}>Add server</button>
    {/if}
  </div>

  {#if !wsId}
    <EmptyState
      icon="gear"
      title="Select a workspace first"
      body="MCP servers are per-workspace. Choose a workspace from the sidebar to configure them."
    />
  {:else if loading && servers.length === 0}
    <Skeleton rows={2} height={64} />
  {:else}
    {#if formOpen}
      <div class="card form">
        <div class="field">
          <label for="mcp-name">Name</label>
          <input
            id="mcp-name"
            class="input"
            bind:value={fName}
            spellcheck="false"
            autocomplete="off"
            placeholder="linear"
          />
          <span class="hint">The key under <code>mcpServers</code> in <code>.mcp.json</code> (unique per workspace).</span>
        </div>
        <div class="field">
          <label for="mcp-command">Command</label>
          <input
            id="mcp-command"
            class="input"
            bind:value={fCommand}
            spellcheck="false"
            autocomplete="off"
            placeholder="npx"
          />
        </div>
        <div class="field">
          <label for="mcp-args">Arguments (one per line)</label>
          <textarea
            id="mcp-args"
            class="input mono"
            rows="3"
            bind:value={fArgs}
            spellcheck="false"
            placeholder={'-y\n@linear/mcp'}
          ></textarea>
        </div>
        <div class="field">
          <label for="mcp-env">Environment (KEY=value, one per line)</label>
          <textarea
            id="mcp-env"
            class="input mono"
            rows="3"
            bind:value={fEnv}
            spellcheck="false"
            placeholder={'API_KEY=...'}
          ></textarea>
          <span class="hint">
            Stored in plaintext for now (like <code>.mcp.json</code> itself, which lives in the
            workspace). Avoid putting long-lived secrets here until Keychain refs land.
          </span>
        </div>
        <div class="field field-row">
          <label for="mcp-enabled">Enabled (write to <code>.mcp.json</code> on next spawn)</label>
          <input id="mcp-enabled" type="checkbox" bind:checked={fEnabled} />
        </div>
        <div class="actions">
          <button class="btn primary" disabled={saving} onclick={save}>
            {saving ? 'Saving…' : editing ? 'Save changes' : 'Add server'}
          </button>
          <button class="btn" disabled={saving} onclick={closeForm}>Cancel</button>
        </div>
      </div>
    {/if}

    {#if servers.length === 0 && !formOpen}
      <EmptyState
        icon="gear"
        title="No MCP servers"
        body="Add a Model Context Protocol server to give agents extra tools in this workspace."
      />
    {:else}
      <div class="server-list">
        {#each servers as s (s.id)}
          <div class="card server" class:off={!s.enabled}>
            <div class="server-main">
              <div class="server-head">
                <span class="server-name mono">{s.name}</span>
                {#if s.enabled}
                  <span class="badge on">enabled</span>
                {:else}
                  <span class="badge">off</span>
                {/if}
              </div>
              <div class="server-cmd mono dim">
                {s.command}{s.args.length ? ' ' + s.args.join(' ') : ''}
              </div>
              {#if Object.keys(s.env).length}
                <div class="server-env dim">env: {Object.keys(s.env).join(', ')}</div>
              {/if}
            </div>
            <div class="server-actions">
              <button class="btn small" disabled={busyId === s.id} onclick={() => toggleEnabled(s)}>
                {s.enabled ? 'Disable' : 'Enable'}
              </button>
              <button class="btn small" disabled={busyId === s.id} onclick={() => openEdit(s)}>
                Edit
              </button>
              <button class="btn small danger" disabled={busyId === s.id} onclick={() => remove(s)}>
                Remove
              </button>
            </div>
          </div>
        {/each}
      </div>
    {/if}
  {/if}
</div>

<style>
  .page {
    padding: 20px 24px;
    max-width: 760px;
  }
  .page-header {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 16px;
    margin-bottom: 16px;
  }
  h1 {
    font-size: 18px;
    margin: 0 0 4px;
  }
  .sub {
    font-size: 12.5px;
    color: var(--text-dim);
    line-height: 1.5;
    max-width: 560px;
  }
  code {
    font-family: var(--font-mono, monospace);
    font-size: 0.92em;
  }
  .card {
    border: 1px solid var(--border);
    border-radius: var(--radius-m, 8px);
    background: var(--surface-1, var(--surface-2));
    padding: 14px 16px;
  }
  .form {
    display: flex;
    flex-direction: column;
    gap: 12px;
    margin-bottom: 16px;
  }
  .field {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .field-row {
    flex-direction: row;
    align-items: center;
    gap: 8px;
  }
  .field-row label {
    flex: 1;
  }
  label {
    font-size: 12px;
    font-weight: 500;
    color: var(--text);
  }
  .input {
    width: 100%;
    box-sizing: border-box;
    background: var(--surface-2);
    border: 1px solid var(--border);
    border-radius: var(--radius-s, 6px);
    color: var(--text);
    font-size: 12.5px;
    padding: 6px 8px;
  }
  textarea.input {
    resize: vertical;
  }
  .mono {
    font-family: var(--font-mono, monospace);
  }
  .hint {
    font-size: 11.5px;
    color: var(--text-dim);
    line-height: 1.45;
  }
  .actions {
    display: flex;
    gap: 8px;
    align-items: center;
  }
  .btn {
    height: 28px;
    padding: 0 12px;
    border: 1px solid var(--border);
    background: var(--surface-2);
    color: var(--text);
    border-radius: var(--radius-s, 6px);
    font-size: 12.5px;
    cursor: pointer;
  }
  .btn:hover {
    background: var(--surface-3, var(--surface-2));
  }
  .btn.primary {
    background: var(--accent);
    border-color: var(--accent);
    color: var(--accent-fg, #fff);
  }
  .btn.small {
    height: 24px;
    padding: 0 8px;
    font-size: 11.5px;
  }
  .btn.danger:hover {
    border-color: var(--danger, #c0392b);
    color: var(--danger, #c0392b);
  }
  .btn:disabled {
    opacity: 0.5;
    cursor: default;
  }
  .server-list {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .server {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
  }
  .server.off {
    opacity: 0.72;
  }
  .server-main {
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .server-head {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .server-name {
    font-size: 13px;
    font-weight: 600;
  }
  .server-cmd,
  .server-env {
    font-size: 11.5px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .dim {
    color: var(--text-dim);
  }
  .badge {
    font-size: 10.5px;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    padding: 1px 6px;
    border-radius: 999px;
    border: 1px solid var(--border);
    color: var(--text-dim);
  }
  .badge.on {
    color: var(--accent);
    border-color: color-mix(in srgb, var(--accent) 50%, transparent);
    background: color-mix(in srgb, var(--accent) 14%, transparent);
  }
  .server-actions {
    display: flex;
    gap: 6px;
    flex-shrink: 0;
  }
</style>
