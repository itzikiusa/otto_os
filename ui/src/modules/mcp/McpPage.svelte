<script lang="ts">
  // MCP Control Plane — three focused sections: Otto's built-in server,
  // governed external servers, and approval/audit activity.
  import { resourceAccess } from '../../lib/stores/resource-access.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import { router } from '../../lib/router.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { mcpCpApi } from '../../lib/api/mcp';
  import { toasts } from '../../lib/toast.svelte';
  import type { McpServerDetail } from '../../lib/api/types';
  import ServersTab from './ServersTab.svelte';
  import ApprovalsTab from './ApprovalsTab.svelte';
  import AuditTab from './AuditTab.svelte';
  import OttoServerHome from './OttoServerHome.svelte';

  type Section = 'otto' | 'servers' | 'activity';
  const section = $derived<Section>(
    (['otto', 'servers', 'activity'] as const).includes(router.parts[1] as Section)
      ? (router.parts[1] as Section)
      : 'otto',
  );
  const wsId = $derived(ws.currentId);

  // The registry is shared by the external-server view and activity filters.
  let accessRevision = $state(0);
  let loadGeneration = 0;
  $effect(() =>
    resourceAccess.subscribe((change) => {
      if (
        change.type === 'decision' &&
        (change.kind !== 'mcp_server' ||
          !change.before ||
          !Object.keys(change.before.operations).some(
            (operation) =>
              change.before?.operations[operation]?.allowed &&
              !change.after?.operations[operation]?.allowed,
          ))
      )
        return;
      accessRevision++;
      loadGeneration++;
      servers = [];
      selectedServerId = null;
      void loadServers();
    }),
  );
  let servers = $state<McpServerDetail[]>([]);
  let loading = $state(false);
  let selectedServerId = $state<string | null>(null);
  let pending = $state(0);

  async function loadServers(): Promise<void> {
    const generation = ++loadGeneration;
    const id = wsId;
    if (!id) {
      servers = [];
      loading = false;
      return;
    }
    loading = true;
    try {
      const result = await mcpCpApi.cpList(id);
      if (generation !== loadGeneration) return;
      servers = result;
      if (selectedServerId && !servers.some((server) => server.id === selectedServerId)) {
        selectedServerId = null;
      }
      if (!selectedServerId && servers.length > 0) selectedServerId = servers[0].id;
    } catch (e) {
      toasts.error('Failed to load MCP servers', e instanceof Error ? e.message : String(e));
    } finally {
      if (generation === loadGeneration) loading = false;
    }
  }

  async function loadPending(): Promise<void> {
    try {
      pending = (await mcpCpApi.cpApprovals('pending')).length;
    } catch {
      pending = 0;
    }
  }

  $effect(() => {
    void wsId;
    selectedServerId = null;
    void loadServers();
  });

  $effect(() => {
    void loadPending();
    const interval = window.setInterval(() => void loadPending(), 15_000);
    return () => window.clearInterval(interval);
  });

  function patchServer(updated: McpServerDetail): void {
    servers = servers.map((server) => (server.id === updated.id ? updated : server));
  }

  function go(id: Section): void {
    router.go('mcp/' + id);
  }
</script>

<div class="mcp-page">
  <header class="mcp-head">
    <div class="title">
      <Icon name="plug" size={16} />
      <span class="h">MCP Control Plane</span>
      {#if ws.current}<span class="wsname">{ws.current.name}</span>{/if}
    </div>
  </header>

  <nav class="tabs" aria-label="MCP sections">
    <button
      class:on={section === 'otto'}
      data-testid="mcp-nav-otto"
      onclick={() => go('otto')}
    >Otto server</button>
    <button
      class:on={section === 'servers'}
      data-testid="mcp-nav-servers"
      onclick={() => go('servers')}
    >External servers{wsId ? ` (${servers.length})` : ''}</button>
    <button
      class:on={section === 'activity'}
      data-testid="mcp-nav-activity"
      onclick={() => go('activity')}
    >
      Activity
      {#if pending > 0}<span class="badge" data-testid="mcp-pending-badge">{pending}</span>{/if}
    </button>
  </nav>

  <div class="tab-body">
    {#key accessRevision}
      {#if section === 'otto'}
        <OttoServerHome {wsId} />
      {:else if section === 'servers'}
        {#if !wsId}
          <div class="empty">
            <Icon name="plug" size={30} />
            <h3>No workspace selected</h3>
            <p>Select a workspace to manage its governed MCP servers and tools.</p>
          </div>
        {:else}
          <ServersTab
            {wsId}
            {servers}
            {loading}
            selectedServerId={null}
            onReload={loadServers}
            onPatch={patchServer}
            onSelect={() => {}}
          />
        {/if}
      {:else}
        <div class="activity">
          <ApprovalsTab />
          <AuditTab {servers} />
        </div>
      {/if}
    {/key}
  </div>
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
    display: flex;
    align-items: center;
    gap: 6px;
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
  .badge {
    min-width: 16px;
    padding: 1px 5px;
    border-radius: 999px;
    background: var(--danger, #c0392b);
    color: white;
    font-size: 10px;
    line-height: 14px;
    text-align: center;
  }
  .tab-body {
    flex: 1;
    min-height: 0;
    overflow: auto;
  }
  .activity {
    display: flex;
    flex-direction: column;
    gap: 18px;
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
