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

  const SECTIONS: { id: Section; label: string }[] = [
    { id: 'otto', label: 'Otto server' },
    { id: 'servers', label: 'External servers' },
    { id: 'activity', label: 'Activity' },
  ];
  function onTabKey(e: KeyboardEvent): void {
    const i = SECTIONS.findIndex((s) => s.id === section);
    let next = -1;
    if (e.key === 'ArrowRight') next = (i + 1) % SECTIONS.length;
    else if (e.key === 'ArrowLeft') next = (i - 1 + SECTIONS.length) % SECTIONS.length;
    else if (e.key === 'Home') next = 0;
    else if (e.key === 'End') next = SECTIONS.length - 1;
    if (next < 0) return;
    e.preventDefault();
    go(SECTIONS[next].id);
    queueMicrotask(() =>
      (e.currentTarget as HTMLElement | null)?.querySelector<HTMLButtonElement>(`[data-testid="mcp-nav-${SECTIONS[next].id}"]`)?.focus(),
    );
  }
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
            {#if s.id === 'activity' && pending > 0}<span class="badge" data-testid="mcp-pending-badge" title="{pending} approval{pending === 1 ? '' : 's'} waiting for you">{pending}</span>{/if}
          </button>
        {/each}
      </div>
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
  .badge {
    min-width: 16px;
    padding: 1px 5px;
    border-radius: 999px;
    /* A pending queue is "needs you" — the amber state, not an error (patterns §5). */
    background: var(--warning-soft);
    color: var(--warning);
    font-weight: 600;
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
      font-size: var(--fs-m);
    }
  }
</style>
