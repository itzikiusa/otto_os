<script lang="ts">
  // MCP Control Plane — three focused sections: Otto's built-in server,
  // governed external servers, and approval/audit activity.
  import { resourceAccess } from '../../lib/stores/resource-access.svelte';
  import PageHeader from '../../lib/components/PageHeader.svelte';
  import PageBody from '../../lib/components/PageBody.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import { router } from '../../lib/router.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { mcpCpApi } from '../../lib/api/mcp';
  import { loadErrorText } from '../../lib/loadError';
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
      loadError = null;
      selectedServerId = null;
      void loadServers();
    }),
  );
  let servers = $state<McpServerDetail[]>([]);
  let loading = $state(false);
  /** Failed server-list load — ServersTab renders it inline with Retry. */
  let loadError = $state<string | null>(null);
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
      loadError = null;
      if (selectedServerId && !servers.some((server) => server.id === selectedServerId)) {
        selectedServerId = null;
      }
      if (!selectedServerId && servers.length > 0) selectedServerId = servers[0].id;
    } catch (e) {
      if (generation === loadGeneration) loadError = loadErrorText(e);
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
  <PageHeader title="MCP Control Plane" icon="plug">
    {#snippet badge()}
      {#if ws.current}<span class="wsname">{ws.current.name}</span>{/if}
    {/snippet}
    {#snippet tabs()}
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
    {/snippet}
  </PageHeader>

  <PageBody padded={false}>
    {#key accessRevision}
      {#if section === 'otto'}
        <OttoServerHome {wsId} />
      {:else if section === 'servers'}
        {#if !wsId}
          <EmptyState
            variant="page"
            icon="plug"
            title="No workspace selected"
            body="Select a workspace to manage its governed MCP servers and tools."
          />
        {:else}
          <ServersTab
            {wsId}
            {servers}
            {loading}
            error={loadError}
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
  </PageBody>
</div>

<style>
  .mcp-page {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }
  .wsname {
    font-size: 11px;
    color: var(--text-dim);
    background: color-mix(in srgb, var(--text-dim) 14%, transparent);
    border-radius: 6px;
    padding: 1px 8px;
  }
  /* Section switch: a segmented control in the page header (native toolbar). */
  .tabs {
    display: inline-flex;
    gap: 2px;
    padding: 2px;
    background: var(--surface-2);
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    flex-wrap: nowrap;
    flex: none;
  }
  .tabs button {
    display: flex;
    align-items: center;
    gap: 6px;
    height: 24px;
    border: none;
    border-radius: 4px;
    background: transparent;
    color: var(--text-dim);
    padding: 0 10px;
    cursor: pointer;
    font-size: 12px;
    white-space: nowrap;
    flex: none;
  }
  .tabs button.on {
    background: var(--surface);
    color: var(--text);
    box-shadow: 0 1px 2px rgba(0, 0, 0, 0.18);
  }
  .badge {
    min-width: 16px;
    padding: 1px 5px;
    border-radius: 999px;
    background: var(--danger-solid);
    color: white;
    font-size: var(--fs-xs);
    line-height: 14px;
    text-align: center;
  }
  .activity {
    display: flex;
    flex-direction: column;
    gap: 18px;
  }
  @media (max-width: 640px) {
    .tabs button {
      height: 30px;
      padding: 0 10px;
      font-size: 13px;
    }
  }
</style>
