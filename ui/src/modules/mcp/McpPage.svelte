<script lang="ts">
  import Badge from '../../lib/components/Badge.svelte';
  import { NO_WORKSPACE } from '../../lib/labels';
  import { plural } from '../../lib/plural';
  // MCP Control Plane — three focused sections: Otto's built-in server,
  // governed external servers, and approval/audit activity.
  import { resourceAccess } from '../../lib/stores/resource-access.svelte';
  import { onTabKey } from '../../lib/tabKeys';
  import PageHeader from '../../lib/components/PageHeader.svelte';
  import PageBody from '../../lib/components/PageBody.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import { router } from '../../lib/router.svelte';
  import { registry } from '../../lib/commands.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { mcpCpApi } from '../../lib/api/mcp';
  import { liveQuery } from '../../lib/live';
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
      // The count route, not 200 whole rows just to take their length.
      pending = (await mcpCpApi.cpApprovalsCount('pending')).count;
    } catch {
      pending = 0;
    }
  }

  $effect(() => {
    void wsId;
    selectedServerId = null;
    void loadServers();
  });

  // Pending-approvals badge: event-fed (`mcp_approval_changed`); the 15 s
  // poll chain only while the event socket is down.
  $effect(() => {
    const p = liveQuery({ run: () => loadPending(), on: ['mcp_approval_changed'], fallbackMs: 15_000 });
    return () => p.stop();
  });

  function patchServer(updated: McpServerDetail): void {
    servers = servers.map((server) => (server.id === updated.id ? updated : server));
  }

  function go(id: Section): void {
    router.go('mcp/' + id);
  }

  const SECTIONS: { id: Section; label: string }[] = [
    { id: 'otto', label: 'Otto server' },
    { id: 'servers', label: 'External servers' },
    { id: 'activity', label: 'Activity' },
  ];

  // ⌘K: the page's verbs.
  $effect(() =>
    registry.register('mcp', [
      { id: 'mcp.otto', title: 'Show the Otto MCP server', group: 'MCP', keywords: 'built-in gateway tools expose attach sessions token', run: () => go('otto') },
      { id: 'mcp.servers', title: 'Show external MCP servers', group: 'MCP', keywords: 'governed servers tools discover allowlist policy', run: () => go('servers') },
      { id: 'mcp.activity', title: 'Show MCP approvals and audit', group: 'MCP', keywords: 'activity pending approve deny audit log', run: () => go('activity') },
      { id: 'mcp.refresh', title: 'Refresh MCP servers', group: 'MCP', keywords: 'reload list', disabled: !wsId, run: () => void loadServers() },
    ]),
  );
</script>

<div class="mcp-page">
  <PageHeader title="MCP Control Plane">
    {#snippet badge()}
      {#if ws.current}<span class="wsname" title="Workspace: {ws.current.name}">{ws.current.name}</span>{/if}
    {/snippet}
    {#snippet tabs()}
      <!-- A route-backed tablist (←/→/Home/End move between sections). -->
      <div class="segmented tabs" role="tablist" aria-label="MCP sections" tabindex="-1" onkeydown={onTabKey}>
        {#each SECTIONS as s (s.id)}
          <button
            class:active={section === s.id}
            role="tab"
            aria-selected={section === s.id}
            tabindex={section === s.id ? 0 : -1}
            data-testid="mcp-nav-{s.id}"
            onclick={() => go(s.id)}
          >
            {s.label}
            {#if s.id === 'servers' && wsId && !loading && !loadError}<span class="count">{servers.length}</span>{/if}
            {#if s.id === 'activity' && pending > 0}<Badge tone="warn" testid="mcp-pending-badge" title="{plural(pending, 'approval')} waiting for you" label={String(pending)} />{/if}
          </button>
        {/each}
      </div>
    {/snippet}
  </PageHeader>

  <!-- The Otto-server panel is a readable page and takes the shared body
       gutter; the server and activity sections are full-bleed lists whose own
       toolbars span the pane, so they manage their edges themselves. -->
  <PageBody padded={section === 'otto'}>
    {#key accessRevision}
      {#if section === 'otto'}
        <OttoServerHome {wsId} />
      {:else if section === 'servers'}
        {#if !wsId}
          <EmptyState
            variant="page"
            icon="plug"
            title={NO_WORKSPACE}
            body="Governed MCP servers and their tools belong to a workspace."
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
          <ApprovalsTab ondecided={() => void loadPending()} />
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
    font-size: var(--fs-xs);
    color: var(--text-dim);
    background: var(--surface-2);
    border: 1px solid var(--border);
    border-radius: 999px;
    padding: 1px 8px;
    max-width: 180px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  /* Section switch: the shared segmented control in the page header. */
  .tabs {
    flex-wrap: nowrap;
    flex: none;
  }
  .tabs button {
    display: flex;
    align-items: center;
    gap: 6px;
    white-space: nowrap;
    flex: none;
  }
  .count {
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
  }
  .activity {
    display: flex;
    flex-direction: column;
    gap: 18px;
  }
  @media (max-width: 640px) {
    .tabs button {
      height: 30px;
      font-size: var(--fs-m);
    }
  }
</style>
