<script lang="ts">
  // MCP Control Plane — Otto as the governed path between agents and MCP tools.
  // A workspace-scoped registry of MCP servers (stdio/http), with health,
  // discovery, per-tool permissions, allowlists, policy-as-code, an approval
  // queue, an audit ledger, per-tool stats, and Otto's own outward MCP server.
  //
  // The active workspace comes from the shared workspace store (same as Brokers);
  // Servers + Tools are workspace-scoped, the rest are governed globally and
  // filtered server-side to the workspaces the caller can access.
  import Icon from '../../lib/components/Icon.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { mcpCpApi } from '../../lib/api/mcp';
  import { toasts } from '../../lib/toast.svelte';
  import type { McpServerDetail } from '../../lib/api/types';
  import ServersTab from './ServersTab.svelte';
  import ToolsTab from './ToolsTab.svelte';
  import AllowlistsTab from './AllowlistsTab.svelte';
  import PoliciesTab from './PoliciesTab.svelte';
  import ApprovalsTab from './ApprovalsTab.svelte';
  import AuditTab from './AuditTab.svelte';
  import StatsTab from './StatsTab.svelte';
  import OttoServerTab from './OttoServerTab.svelte';

  type Tab =
    | 'servers'
    | 'tools'
    | 'allowlists'
    | 'policies'
    | 'approvals'
    | 'audit'
    | 'stats'
    | 'otto';
  let tab = $state<Tab>('servers');

  const wsId = $derived(ws.currentId);

  // The registry is shared between the Servers and Tools tabs (and feeds the
  // server pickers in Allowlists / Policies / Audit), so it's loaded once here.
  let servers = $state<McpServerDetail[]>([]);
  let loading = $state(false);
  let selectedServerId = $state<string | null>(null);

  async function loadServers(): Promise<void> {
    const id = wsId;
    if (!id) {
      servers = [];
      return;
    }
    loading = true;
    try {
      servers = await mcpCpApi.cpList(id);
      // Keep a sensible selection for the Tools tab.
      if (selectedServerId && !servers.some((s) => s.id === selectedServerId)) {
        selectedServerId = null;
      }
      if (!selectedServerId && servers.length > 0) selectedServerId = servers[0].id;
    } catch (e) {
      toasts.error('Failed to load MCP servers', e instanceof Error ? e.message : String(e));
    } finally {
      loading = false;
    }
  }

  $effect(() => {
    void wsId; // re-load when the active workspace changes
    selectedServerId = null;
    void loadServers();
  });

  function patchServer(updated: McpServerDetail): void {
    servers = servers.map((s) => (s.id === updated.id ? updated : s));
  }

  const tabs: { id: Tab; label: string }[] = [
    { id: 'servers', label: 'Servers' },
    { id: 'tools', label: 'Tools' },
    { id: 'allowlists', label: 'Allowlists' },
    { id: 'policies', label: 'Policies' },
    { id: 'approvals', label: 'Approvals' },
    { id: 'audit', label: 'Audit' },
    { id: 'stats', label: 'Stats' },
    { id: 'otto', label: 'Otto Server' },
  ];
</script>

<div class="mcp-page">
  <header class="mcp-head">
    <div class="title">
      <Icon name="plug" size={16} />
      <span class="h">MCP Control Plane</span>
      {#if ws.current}<span class="wsname">{ws.current.name}</span>{/if}
    </div>
  </header>

  {#if !wsId}
    <div class="empty">
      <Icon name="plug" size={30} />
      <h3>No workspace selected</h3>
      <p>Select a workspace to manage its governed MCP servers and tools.</p>
    </div>
  {:else}
    <nav class="tabs" aria-label="MCP sections">
      {#each tabs as t (t.id)}
        <button class:on={tab === t.id} onclick={() => (tab = t.id)}>{t.label}</button>
      {/each}
    </nav>

    <div class="tab-body">
      {#if tab === 'servers'}
        <ServersTab
          {wsId}
          {servers}
          {loading}
          {selectedServerId}
          onReload={loadServers}
          onPatch={patchServer}
          onSelect={(id) => {
            selectedServerId = id;
            tab = 'tools';
          }}
        />
      {:else if tab === 'tools'}
        <ToolsTab
          {wsId}
          {servers}
          {selectedServerId}
          onSelect={(id) => (selectedServerId = id)}
        />
      {:else if tab === 'allowlists'}
        <AllowlistsTab {wsId} {servers} />
      {:else if tab === 'policies'}
        <PoliciesTab {wsId} {servers} />
      {:else if tab === 'approvals'}
        <ApprovalsTab />
      {:else if tab === 'audit'}
        <AuditTab {servers} />
      {:else if tab === 'stats'}
        <StatsTab />
      {:else if tab === 'otto'}
        <OttoServerTab />
      {/if}
    </div>
  {/if}
</div>

<style>
  .mcp-page {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }
  .mcp-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 12px 16px;
    border-bottom: 1px solid var(--border);
    flex: none;
  }
  .title {
    display: flex;
    align-items: center;
    gap: 8px;
    color: var(--text-dim);
  }
  .title .h {
    font-size: 15px;
    font-weight: 600;
    color: var(--text);
  }
  .wsname {
    font-size: 11px;
    color: var(--text-dim);
    background: color-mix(in srgb, var(--text-dim) 14%, transparent);
    border-radius: 6px;
    padding: 1px 8px;
  }
  .tabs {
    display: flex;
    gap: 2px;
    padding: 6px 14px 0;
    border-bottom: 1px solid var(--border);
    overflow-x: auto;
    flex-wrap: nowrap;
    flex: none;
    -webkit-overflow-scrolling: touch;
  }
  .tabs button {
    border: none;
    background: transparent;
    color: var(--text-dim);
    padding: 8px 14px;
    cursor: pointer;
    font-size: 13px;
    border-bottom: 2px solid transparent;
    white-space: nowrap;
    flex: none;
  }
  .tabs button.on {
    color: var(--text);
    border-bottom-color: var(--accent);
  }
  .tab-body {
    flex: 1;
    min-height: 0;
    overflow: auto;
  }
  .empty {
    height: 100%;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 10px;
    color: var(--text-dim);
    text-align: center;
    padding: 24px;
  }
  .empty h3 {
    margin: 4px 0 0;
    color: var(--text);
  }

  @media (max-width: 640px) {
    .tabs button {
      padding: 10px 12px;
      font-size: 14px;
    }
  }
</style>
