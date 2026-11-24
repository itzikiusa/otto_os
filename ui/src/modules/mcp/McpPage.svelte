<script lang="ts">
  import { untrack } from 'svelte';
  import Badge from '../../lib/components/Badge.svelte';
  import { NO_WORKSPACE } from '../../lib/labels';
  import { plural } from '../../lib/plural';
  // MCP Control Plane. It opens on what needs a person (McpHome: waiting
  // approvals, else four sentences); Tools is Otto's own server and its
  // catalog; then Profiles, External servers and Activity (approvals history +
  // audit). Approvals are ONE event-fed list for the page (the badge and the
  // landing read it): `mcp_approval_changed` fires on every approval write —
  // created, decided, consumed, expiry sweep — so no poll runs while the event
  // socket is up; the fallback poll is visible-only and runs only while it is down.
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
  import { initialSelection, rememberSelection } from '../../lib/lastSelection';
  import type { McpServerDetail } from '../../lib/api/types';
  import { mcpUiState } from './mcpUiState.svelte';
  import McpHome from './McpHome.svelte';
  import { pendingOf } from './mcpHome';
  import type { McpApproval } from '../../lib/api/types';
  import { EVENT_COMPLETE_SAFETY_MS } from '../../lib/live';
  import LazyMount from '../../lib/components/LazyMount.svelte';
  import { lazyComponent } from '../../lib/lazy-component.svelte';
  // Profiles is its own section, behind a dynamic import so the editor and
  // tool-group tables don't ride on every MCP page open (bundle budget).
  const ProfilesTab = lazyComponent(() => import('./ProfilesTab.svelte'));
  // Every section but the landing loads on first open: first paint is the
  // Overview (what waits on you) only.
  const OttoServerHome = lazyComponent(() => import('./OttoServerHome.svelte'));
  const ServersTab = lazyComponent(() => import('./ServersTab.svelte'));
  const ApprovalsTab = lazyComponent(() => import('./ApprovalsTab.svelte'));
  const AuditTab = lazyComponent(() => import('./AuditTab.svelte'));

  type Section = 'home' | 'otto' | 'profiles' | 'servers' | 'activity';
  const section = $derived<Section>(
    (['otto', 'profiles', 'servers', 'activity'] as const).includes(router.parts[1] as Exclude<Section, 'home'>)
      ? (router.parts[1] as Section)
      : 'home',
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
      mcpUiState.loaded = false;
      mcpUiState.selectedServerId = null;
      mcpUiState.expandedServerId = null;
      void loadServers();
    }),
  );
  let servers = $state<McpServerDetail[]>(
    untrack(() => (ws.currentId && mcpUiState.wsId === ws.currentId && mcpUiState.loaded ? mcpUiState.servers : [])),
  );
  let loading = $state(false);
  /** Failed server-list load — ServersTab renders it inline with Retry. */
  let loadError = $state<string | null>(null);
  /** The pending requests (only those, never 200 decided rows): the badge
   *  and the landing's rows read this one list. */
  let approvals = $state<McpApproval[] | null>(null);
  let approvalsError = $state<string | null>(null);
  const pending = $derived(pendingOf(approvals ?? []).length);

  async function loadServers(background = false): Promise<void> {
    const generation = ++loadGeneration;
    const id = wsId;
    if (!id) {
      servers = [];
      loading = false;
      return;
    }
    if (!background) loading = true;
    try {
      const result = await mcpCpApi.cpList(id);
      if (generation !== loadGeneration || wsId !== id) return;
      servers = result;
      loadError = null;
      mcpUiState.setServers(result, (items) => initialSelection('mcp-servers', items, (s) => s.id));
    } catch (e) {
      if (generation === loadGeneration && wsId === id && (!background || servers.length === 0)) {
        loadError = loadErrorText(e);
      }
    } finally {
      if (!background && generation === loadGeneration && wsId === id) loading = false;
    }
  }

  $effect(() => {
    if (mcpUiState.selectedServerId) rememberSelection('mcp-servers', mcpUiState.selectedServerId);
  });

  async function loadApprovals(): Promise<boolean> {
    try {
      approvals = await mcpCpApi.cpApprovals('pending');
      approvalsError = null;
      return true;
    } catch (e) {
      // Keep the last list on a transient failure; the landing says why.
      approvalsError = loadErrorText(e);
      return false;
    }
  }

  $effect(() => {
    const id = wsId;
    const changed = untrack(() => mcpUiState.syncWorkspace(id));
    if (!id) {
      servers = [];
      loading = false;
      loadError = null;
      return;
    }
    if (!changed && untrack(() => mcpUiState.loaded)) {
      servers = untrack(() => mcpUiState.servers);
      loading = false;
      void loadServers(true);
      return;
    }
    void loadServers(false);
  });

  // Event-fed (`mcp_approval_changed`, every approval write); the 15 s poll
  // only while the event socket is down, paused while the page is hidden.
  let approvalsQuery: ReturnType<typeof liveQuery> | null = null;
  $effect(() => {
    const p = liveQuery({ run: () => loadApprovals(), on: ['mcp_approval_changed'], fallbackMs: 15_000, safetyMs: EVENT_COMPLETE_SAFETY_MS });
    approvalsQuery = p;
    return () => p.stop();
  });
  const reloadApprovals = (): void => approvalsQuery?.now();

  function patchServer(updated: McpServerDetail): void {
    servers = servers.map((server) => (server.id === updated.id ? updated : server));
    mcpUiState.patchServer(updated);
  }

  function go(id: Section): void {
    router.go(id === 'home' ? 'mcp' : 'mcp/' + id);
  }

  const SECTIONS: { id: Section; label: string }[] = [
    { id: 'home', label: 'Overview' },
    { id: 'otto', label: 'Tools' },
    { id: 'profiles', label: 'Profiles' },
    { id: 'servers', label: 'External servers' },
    { id: 'activity', label: 'Activity' },
  ];

  // ⌘K: the page's verbs.
  $effect(() =>
    registry.register('mcp', [
      { id: 'mcp.home', title: 'Show what waits on you in MCP', group: 'MCP', keywords: 'overview approvals pending summary', run: () => go('home') },
      { id: 'mcp.otto', title: 'Show Otto’s MCP tools', group: 'MCP', keywords: 'built-in gateway tools catalog expose attach sessions token', run: () => go('otto') },
      { id: 'mcp.profiles', title: 'Show MCP profiles', group: 'MCP', keywords: 'profile tool set subset database issues default session connection', run: () => go('profiles') },
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
            {#if s.id === 'home' && pending > 0}<Badge tone="warn" testid="mcp-pending-badge" title="{plural(pending, 'approval')} waiting for you" label={String(pending)} />{/if}
          </button>
        {/each}
      </div>
    {/snippet}
  </PageHeader>

  <!-- The Otto-server panel is a readable page and takes the shared body
       gutter; the server and activity sections are full-bleed lists whose own
       toolbars span the pane, so they manage their edges themselves. -->
  <PageBody padded={section === 'otto' || section === 'profiles'}>
    {#key accessRevision}
      {#if section === 'home'}
        <McpHome
          {wsId}
          {approvals}
          {approvalsError}
          {servers}
          serversLoaded={mcpUiState.loaded}
          serversError={loadError}
          onretry={() => { reloadApprovals(); void loadServers(false); }}
          ondecided={reloadApprovals}
        />
      {:else if section === 'otto'}
        <LazyMount lazy={OttoServerHome} what="Otto’s tools" props={{ wsId }} />
      {:else if section === 'profiles'}
        <LazyMount lazy={ProfilesTab} what="MCP profiles" />
      {:else if section === 'servers'}
        {#if !wsId}
          <EmptyState
            variant="page"
            icon="plug"
            title={NO_WORKSPACE}
            body="MCP servers that run through Otto, and their tools, belong to a workspace."
          />
        {:else}
          <LazyMount lazy={ServersTab} what="the external servers" props={{ wsId, servers, loading, error: loadError, selectedServerId: mcpUiState.selectedServerId, onReload: () => loadServers(false), onPatch: patchServer, onSelect: (id: string | null) => (mcpUiState.selectedServerId = id) }} />
        {/if}
      {:else}
        <div class="activity">
          <LazyMount lazy={ApprovalsTab} what="approvals" variant="panel" props={{ ondecided: reloadApprovals }} />
          <LazyMount lazy={AuditTab} what="the audit log" variant="panel" props={{ servers }} />
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
