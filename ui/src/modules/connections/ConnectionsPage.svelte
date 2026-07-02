<script lang="ts">
  // Connections hub: the ONE tree for every connection type — SSH, databases,
  // message brokers and custom clients — filed into a shared, nestable section
  // tree. Rows are compact list entries; drag a row onto a section to file it,
  // a section onto another to nest it. DB rows open the Database workbench,
  // cluster rows open Message Brokers; a type-filter narrows the tree.
  import { api } from '../../lib/api/client';
  import { confirmer } from '../../lib/confirm.svelte';
  import type {
    BrokerCluster,
    Connection,
    ConnectionKind,
    ConnectionSection,
    Session,
    TestConnectionResp,
  } from '../../lib/api/types';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { router } from '../../lib/router.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { brokers } from '../../lib/stores/brokers.svelte';
  import { database } from '../../lib/stores/database.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import Skeleton from '../../lib/components/Skeleton.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import ConnectionForm from './ConnectionForm.svelte';
  import SftpBrowser from './SftpBrowser.svelte';
  import ConnectionImportDialog from './ConnectionImportDialog.svelte';
  import ClusterForm from '../brokers/ClusterForm.svelte';

  interface TreeNode {
    sec: ConnectionSection;
    items: Connection[];
    clusters: BrokerCluster[];
    children: TreeNode[];
  }

  // Every connection kind the hub can create + the DB engines that open the
  // Database workbench instead of a terminal.
  const ALL_KINDS: ConnectionKind[] = [
    'ssh', 'mysql', 'postgres', 'redis', 'mongodb', 'clickhouse', 'custom',
  ];
  const DB_KINDS = new Set<ConnectionKind>(['mysql', 'postgres', 'redis', 'mongodb', 'clickhouse']);
  const isDbKind = (k: ConnectionKind): boolean => DB_KINDS.has(k);

  // Toolbar type filter (single-select). Each id matches a connection kind;
  // 'kafka' matches broker clusters. Persisted in localStorage.
  const FILTER_KEY = 'otto_connhub_filter';
  const FILTER_CHIPS: { id: string; label: string }[] = [
    { id: 'all', label: 'All' },
    { id: 'ssh', label: 'SSH' },
    { id: 'mysql', label: 'MySQL' },
    { id: 'postgres', label: 'PostgreSQL' },
    { id: 'redis', label: 'Redis' },
    { id: 'mongodb', label: 'MongoDB' },
    { id: 'clickhouse', label: 'ClickHouse' },
    { id: 'kafka', label: 'Kafka' },
    { id: 'custom', label: 'Custom' },
  ];
  function loadFilter(): string {
    if (typeof localStorage === 'undefined') return 'all';
    return localStorage.getItem(FILTER_KEY) || 'all';
  }
  let filterKind = $state(loadFilter());
  function setFilter(id: string): void {
    filterKind = id;
    if (typeof localStorage !== 'undefined') localStorage.setItem(FILTER_KEY, id);
  }
  // A filter is "active" (narrowing the tree, hiding empty sections) whenever it
  // isn't the catch-all. Search has its own flat-results view below.
  const filtering = $derived(filterKind !== 'all');
  const connMatchesFilter = (c: Connection): boolean => filterKind === 'all' || c.kind === filterKind;
  const clusterMatchesFilter = (): boolean => filterKind === 'all' || filterKind === 'kafka';

  let conns: Connection[] = $state([]);
  let sections: ConnectionSection[] = $state([]);
  let loading = $state(true);
  let formOpen = $state(false);
  let importOpen = $state(false);
  let editing: Connection | null = $state(null);
  // Message-broker cluster form (New Cluster / edit a cluster row).
  let clusterFormOpen = $state(false);
  let editingCluster: BrokerCluster | null = $state(null);
  // The SSH connection whose SFTP file browser is open (null = none).
  let sftpFor: Connection | null = $state(null);
  let testing: Record<string, boolean> = $state({});
  let testResults: Record<string, TestConnectionResp> = $state({});
  let opening: Record<string, boolean> = $state({});
  let openingSplit: Record<string, boolean> = $state({});
  let collapsed: Record<string, boolean> = $state({});
  let draggedConnId: string | null = $state(null);
  let draggedClusterId: string | null = $state(null);
  let draggedSectionId: string | null = $state(null);
  // Which connection's "open in workspace…" menu is showing (null = none).
  let openMenuFor: string | null = $state(null);
  // Which connection's SSH-key-permissions fix popover is open (null = none).
  let keyPermsOpenFor: string | null = $state(null);

  // The exact `chmod 600 <path>` command from a key-perms warning, or '' if
  // the message can't be parsed (popover then just shows the message text).
  function keyPermsFix(msg: string | null | undefined): string {
    if (!msg) return '';
    const m = msg.match(/chmod 600 (.+)$/);
    return m ? `chmod 600 ${m[1].trim()}` : '';
  }

  async function copyKeyPermsFix(cmd: string): Promise<void> {
    try {
      await navigator.clipboard.writeText(cmd);
      toasts.success('Copied', cmd);
    } catch {
      toasts.error('Copy failed', 'Clipboard is not available');
    }
  }

  const secName = (id: string | null): string =>
    id ? (sections.find((s) => s.id === id)?.name ?? '') : '';

  // Free-text search across all rows. When non-empty the page shows a flat result
  // list (by name / host-user / kind / section) instead of the tree; the type
  // filter still applies (chips + search compose with AND).
  let search = $state('');
  const connMatchesSearch = (c: Connection, q: string): boolean =>
    c.name.toLowerCase().includes(q) ||
    describe(c).toLowerCase().includes(q) ||
    c.kind.toLowerCase().includes(q) ||
    secName(c.section_id).toLowerCase().includes(q);
  const clusterMatchesSearch = (cl: BrokerCluster, q: string): boolean =>
    cl.name.toLowerCase().includes(q) ||
    cl.bootstrap_servers.toLowerCase().includes(q) ||
    'kafka'.includes(q) ||
    secName(cl.section_id ?? null).toLowerCase().includes(q);
  const searchResults = $derived.by(() => {
    const q = search.trim().toLowerCase();
    if (!q) return { conns: [] as Connection[], clusters: [] as BrokerCluster[] };
    return {
      conns: conns
        .filter((c) => connMatchesFilter(c) && connMatchesSearch(c, q))
        .sort(sortByName),
      clusters: brokers.clusters
        .filter((cl) => clusterMatchesFilter() && clusterMatchesSearch(cl, q))
        .sort(sortByClusterName),
    };
  });
  const searchCount = $derived(searchResults.conns.length + searchResults.clusters.length);

  const kindIcons: Record<ConnectionKind, string> = {
    ssh: 'key',
    mysql: 'db',
    postgres: 'db',
    redis: 'zap',
    mongodb: 'db',
    clickhouse: 'db',
    custom: 'terminal',
  };
  const sortByName = (a: Connection, b: Connection): number => a.name.localeCompare(b.name);
  const sortByClusterName = (a: BrokerCluster, b: BrokerCluster): number =>
    a.name.localeCompare(b.name);

  // Build the section tree from the flat lists; `parentId = null` is the root.
  // Each node carries both connections and broker clusters filed under it.
  function buildTree(parentId: string | null): TreeNode[] {
    return sections
      .filter((s) => (s.parent_id ?? null) === parentId)
      .sort((a, b) => a.position - b.position || a.name.localeCompare(b.name))
      .map((sec) => ({
        sec,
        items: conns.filter((c) => c.section_id === sec.id).sort(sortByName),
        clusters: brokers.clusters.filter((cl) => cl.section_id === sec.id).sort(sortByClusterName),
        children: buildTree(sec.id),
      }));
  }
  const tree = $derived(buildTree(null));

  // Count of matching descendants (connections + clusters, recursive) under the
  // current filter — drives the per-section count chip and whether an empty
  // section is hidden while filtering.
  function nodeMatchCount(node: TreeNode): number {
    let n =
      node.items.filter(connMatchesFilter).length +
      (clusterMatchesFilter() ? node.clusters.length : 0);
    for (const ch of node.children) n += nodeMatchCount(ch);
    return n;
  }

  // A connection/cluster whose folder is unknown falls back to Ungrouped, so
  // nothing ever vanishes. Everything here is global, so all of it can be filed.
  const knownSectionIds = $derived(new Set(sections.map((s) => s.id)));
  const ungrouped = $derived(
    conns.filter((c) => !c.section_id || !knownSectionIds.has(c.section_id)).sort(sortByName),
  );
  const ungroupedClusters = $derived(
    brokers.clusters
      .filter((cl) => !cl.section_id || !knownSectionIds.has(cl.section_id))
      .sort(sortByClusterName),
  );
  const ungroupedMatchCount = $derived(
    ungrouped.filter(connMatchesFilter).length +
      (clusterMatchesFilter() ? ungroupedClusters.length : 0),
  );

  // "Recent" group: pinned connections first (alphabetical), then those with a
  // last_opened_at timestamp (most-recently-opened first), capped at 6 total.
  // Only surfaces connections that have been opened or pinned — a brand-new
  // un-opened, un-pinned connection does not appear here.
  const RECENT_CAP = 6;
  const recent = $derived.by(() => {
    const pinned = conns
      .filter((c) => c.pinned && connMatchesFilter(c))
      .sort(sortByName);
    const pinnedIds = new Set(pinned.map((c) => c.id));
    const byRecent = conns
      .filter((c) => !pinnedIds.has(c.id) && c.last_opened_at && connMatchesFilter(c))
      .sort((a, b) => {
        // Descending: most recently opened first.
        const ta = new Date(a.last_opened_at!).getTime();
        const tb = new Date(b.last_opened_at!).getTime();
        return tb - ta;
      });
    return [...pinned, ...byRecent].slice(0, RECENT_CAP);
  });

  // Load (or reload) the connection list for a workspace. Extracted so the import
  // dialog can refresh the list after creating connections. The hub shows ALL
  // connection kinds (SSH, DB engines, custom) alongside broker clusters.
  async function loadConns(wsId: string): Promise<void> {
    loading = true;
    try {
      conns = await api.get<Connection[]>(`/workspaces/${wsId}/connections`);
    } catch (e) {
      toasts.error('Could not load connections', e instanceof Error ? e.message : '');
    } finally {
      loading = false;
    }
  }

  $effect(() => {
    const wsId = ws.currentId;
    if (!wsId) return;
    void loadConns(wsId);
    // Broker clusters share this tree — load them through their own store (which
    // owns cluster CRUD + the shared section fetch); we read `brokers.clusters`.
    void brokers.load(wsId);
    void api
      .get<ConnectionSection[]>(`/workspaces/${wsId}/connection-sections`)
      .then((s) => (sections = s))
      .catch(() => {});
  });

  // --- Section operations ---------------------------------------------------

  async function createSection(parentId: string | null): Promise<void> {
    if (!ws.currentId) return;
    const name = await confirmer.promptText(parentId ? 'Sub-section name' : 'Section name', {
      title: parentId ? 'New sub-section' : 'New section',
      confirmLabel: 'Create',
      placeholder: 'e.g. Production',
    });
    if (!name) return;
    try {
      const sec = await api.post<ConnectionSection>(
        `/workspaces/${ws.currentId}/connection-sections`,
        { name, parent_id: parentId },
      );
      sections = [...sections, sec];
    } catch (e) {
      toasts.error('Create section failed', e instanceof Error ? e.message : String(e));
    }
  }

  async function renameSection(sec: ConnectionSection): Promise<void> {
    const name = await confirmer.promptText('Rename section', {
      title: 'Rename section',
      confirmLabel: 'Rename',
      initial: sec.name,
    });
    if (!name || name === sec.name) return;
    try {
      const updated = await api.patch<ConnectionSection>(`/connection-sections/${sec.id}`, { name });
      sections = sections.map((s) => (s.id === sec.id ? updated : s));
    } catch (e) {
      toasts.error('Rename failed', e instanceof Error ? e.message : String(e));
    }
  }

  async function deleteSection(sec: ConnectionSection): Promise<void> {
    if (
      !(await confirmer.ask(
        `Delete section “${sec.name}”? Sub-sections are removed too and their connections become ungrouped.`,
        { title: 'Delete section' },
      ))
    )
      return;
    try {
      await api.del(`/connection-sections/${sec.id}`);
      // Drop the section + all descendants locally; clear their connections.
      const removed = new Set<string>();
      const collect = (id: string) => {
        removed.add(id);
        for (const s of sections) if (s.parent_id === id) collect(s.id);
      };
      collect(sec.id);
      sections = sections.filter((s) => !removed.has(s.id));
      conns = conns.map((c) => (c.section_id && removed.has(c.section_id) ? { ...c, section_id: null } : c));
    } catch (e) {
      toasts.error('Delete failed', e instanceof Error ? e.message : String(e));
    }
  }

  function toggleCollapse(id: string): void {
    collapsed[id] = !collapsed[id];
  }

  // --- Drag & drop ----------------------------------------------------------

  // Move a connection into a section (or null = ungrouped). Global connections
  // are not assignable. Reuses the connection PATCH endpoint.
  async function moveConn(c: Connection, sectionId: string | null): Promise<void> {
    if ((c.section_id ?? null) === sectionId) return;
    try {
      const saved = await api.patch<Connection>(`/connections/${c.id}`, {
        name: c.name,
        kind: c.kind,
        params: c.params,
        first_command: c.first_command,
        section_id: sectionId,
        // Preserve the guardrail flags — omitting them would reset to dev/false.
        environment: c.environment,
        read_only: c.read_only,
      });
      conns = conns.map((x) => (x.id === c.id ? saved : x));
    } catch (e) {
      toasts.error('Move failed', e instanceof Error ? e.message : String(e));
    }
  }

  function isDescendantOf(nodeId: string, ancestorId: string): boolean {
    let cur = sections.find((s) => s.id === nodeId);
    while (cur?.parent_id) {
      if (cur.parent_id === ancestorId) return true;
      cur = sections.find((s) => s.id === cur!.parent_id);
    }
    return false;
  }

  async function reparentSection(id: string, parentId: string | null): Promise<void> {
    const sec = sections.find((s) => s.id === id);
    if (!sec || (sec.parent_id ?? null) === parentId) return;
    if (parentId && (parentId === id || isDescendantOf(parentId, id))) {
      toasts.error('Invalid move', 'Cannot nest a section inside itself');
      return;
    }
    try {
      const updated = await api.post<ConnectionSection>(`/connection-sections/${id}/move`, {
        parent_id: parentId,
      });
      sections = sections.map((s) => (s.id === id ? updated : s));
    } catch (e) {
      toasts.error('Move failed', e instanceof Error ? e.message : String(e));
    }
  }

  // File a cluster into a section (or null = ungrouped) via the brokers store's
  // move (PATCH /brokers/clusters/{id} {section_id}). `brokers.clusters` updates
  // in place, so the tree re-renders.
  async function moveCluster(id: string, sectionId: string | null): Promise<void> {
    try {
      await brokers.moveCluster(id, sectionId);
    } catch (e) {
      toasts.error('Move failed', e instanceof Error ? e.message : String(e));
    }
  }

  // A drop onto a section: a dragged connection/cluster files into it; a dragged
  // section nests under it.
  function onSectionDrop(sectionId: string): void {
    if (draggedConnId) {
      const c = conns.find((x) => x.id === draggedConnId);
      draggedConnId = null;
      if (c) void moveConn(c, sectionId);
    } else if (draggedClusterId) {
      const id = draggedClusterId;
      draggedClusterId = null;
      void moveCluster(id, sectionId);
    } else if (draggedSectionId) {
      const src = draggedSectionId;
      draggedSectionId = null;
      void reparentSection(src, sectionId);
    }
  }

  // A drop onto the "Ungrouped" / top-level zone: connection/cluster → no
  // section; section → top-level.
  function onRootDrop(): void {
    if (draggedConnId) {
      const c = conns.find((x) => x.id === draggedConnId);
      draggedConnId = null;
      if (c) void moveConn(c, null);
    } else if (draggedClusterId) {
      const id = draggedClusterId;
      draggedClusterId = null;
      void moveCluster(id, null);
    } else if (draggedSectionId) {
      const src = draggedSectionId;
      draggedSectionId = null;
      void reparentSection(src, null);
    }
  }

  // --- Connection operations ------------------------------------------------

  function describe(c: Connection): string {
    const p = c.params as Record<string, unknown>;
    switch (c.kind) {
      case 'ssh':
        return `${p.user ?? '?'}@${p.host ?? '?'}${p.port ? `:${p.port}` : ''}`;
      case 'mongodb':
        return String(p.connection_string ?? '');
      case 'custom':
        return String(p.template ?? '');
      default:
        return `${p.host ?? '?'}${p.port ? `:${p.port}` : ''}${p.db !== undefined && p.db !== '' ? ` / ${p.db}` : ''}`;
    }
  }

  async function test(c: Connection): Promise<void> {
    testing[c.id] = true;
    try {
      testResults[c.id] = await api.post<TestConnectionResp>(`/connections/${c.id}/test`);
    } catch (e) {
      testResults[c.id] = {
        ok: false,
        latency_ms: null,
        message: e instanceof Error ? e.message : 'test failed',
        warn_argv: false,
      };
    } finally {
      testing[c.id] = false;
    }
  }

  // Open the connection as a terminal session attached to a workspace. The
  // attachment is per-session and temporary (the connection itself is global):
  // closing the session ends it. `targetWsId` defaults to the current workspace
  // but the row's dropdown can pick any workspace.
  async function open(c: Connection, targetWsId?: string): Promise<void> {
    const wsId = targetWsId ?? ws.currentId;
    openMenuFor = null;
    if (!wsId) {
      toasts.error('No workspace', 'Create or select a workspace to attach the session to');
      return;
    }
    opening[c.id] = true;
    try {
      const session = await api.post<Session>(`/connections/${c.id}/open`, { workspace_id: wsId });
      if (wsId === ws.currentId) {
        ws.addSession(session);
      } else {
        // Switch to the target workspace so its new session is visible.
        await ws.select(wsId);
        ws.navigateToSession(session.id);
      }
      const wsName = ws.workspaces.find((w) => w.id === wsId)?.name;
      toasts.success('Connection opened', wsName ? `${c.name} → ${wsName}` : c.name);
    } catch (e) {
      toasts.error('Open failed', e instanceof Error ? e.message : String(e));
    } finally {
      opening[c.id] = false;
    }
  }

  // Open the connection beside the current session(s): drop its terminal into a
  // new split pane instead of replacing the active tab, so a DB/ssh terminal sits
  // next to an agent. Respects the 1–4 pane cap.
  async function openBeside(c: Connection): Promise<void> {
    openingSplit[c.id] = true;
    if (!ws.currentId) {
      toasts.error('No workspace', 'Select a workspace to attach the session to');
      openingSplit[c.id] = false;
      return;
    }
    try {
      const session = await api.post<Session>(`/connections/${c.id}/open`, {
        workspace_id: ws.currentId,
      });
      // Register + drop into a NEW split pane (not the active tab) in one step,
      // so the connection sits beside the current session rather than replacing it.
      const placed = ws.addSessionInSplit(session);
      router.go('agents');
      if (placed) {
        toasts.success('Opened beside', c.name);
      } else {
        toasts.warn('Up to 4 panes', 'Replaced the focused pane — close one to add more.');
      }
    } catch (e) {
      toasts.error('Open failed', e instanceof Error ? e.message : String(e));
    } finally {
      openingSplit[c.id] = false;
    }
  }

  // Open a DB-engine connection in the Database workbench (the primary action for
  // mysql/postgres/redis/mongodb/clickhouse rows). The terminal client stays
  // available as a secondary row action.
  async function openDb(c: Connection): Promise<void> {
    opening[c.id] = true;
    try {
      await database.openConnection(c.id);
      router.go('database');
    } catch (e) {
      toasts.error('Open failed', e instanceof Error ? e.message : String(e));
    } finally {
      opening[c.id] = false;
    }
  }

  // --- Broker cluster operations --------------------------------------------

  // Open a cluster in Message Brokers (select it there + navigate).
  function openCluster(cl: BrokerCluster): void {
    brokers.select(cl.id);
    router.go('brokers');
  }

  function editCluster(cl: BrokerCluster): void {
    editingCluster = cl;
    clusterFormOpen = true;
  }

  async function removeCluster(cl: BrokerCluster): Promise<void> {
    if (
      !(await confirmer.ask(`Remove cluster “${cl.name}”? Topics on the broker are not touched.`, {
        title: 'Remove cluster',
        confirmLabel: 'Remove',
        danger: true,
      }))
    )
      return;
    try {
      await brokers.remove(cl.id);
      toasts.info('Cluster removed', cl.name);
    } catch (e) {
      toasts.error('Remove failed', e instanceof Error ? e.message : String(e));
    }
  }

  async function remove(c: Connection): Promise<void> {
    if (!(await confirmer.ask(`Delete connection “${c.name}”? Its Keychain secret is removed too.`, { title: 'Delete connection' }))) return;
    try {
      await api.del(`/connections/${c.id}`);
      conns = conns.filter((x) => x.id !== c.id);
      toasts.info('Connection deleted', c.name);
    } catch (e) {
      toasts.error('Delete failed', e instanceof Error ? e.message : String(e));
    }
  }

  async function togglePin(c: Connection): Promise<void> {
    try {
      const updated = await api.patch<Connection>(`/connections/${c.id}/pin`, {
        pinned: !c.pinned,
      });
      conns = conns.map((x) => (x.id === c.id ? updated : x));
    } catch (e) {
      toasts.error('Pin failed', e instanceof Error ? e.message : String(e));
    }
  }

  function onSaved(saved: Connection): void {
    const idx = conns.findIndex((x) => x.id === saved.id);
    conns = idx >= 0 ? conns.map((x) => (x.id === saved.id ? saved : x)) : [...conns, saved];
    formOpen = false;
    editing = null;
  }
</script>

<div class="page">
  <div class="page-header">
    <div>
      <h1>Connections</h1>
      <div class="sub">SSH, databases, message brokers and custom clients — one tree, filter by type.</div>
    </div>
    <div class="header-actions">
      <button class="btn" onclick={() => createSection(null)}>New Section</button>
      <button
        class="btn"
        title="Import connections from MySQL Workbench, DBeaver, DataGrip or NoSQLBooster"
        onclick={() => (importOpen = true)}
      >
        <Icon name="plug" size={12} /> Import
      </button>
      <button
        class="btn"
        title="Add a Kafka cluster to Message Brokers"
        onclick={() => {
          editingCluster = null;
          clusterFormOpen = true;
        }}
      >
        <Icon name="box" size={12} /> New Cluster
      </button>
      <button
        class="btn primary"
        onclick={() => {
          editing = null;
          formOpen = true;
        }}
      >
        New Connection
      </button>
    </div>
  </div>

  {#if loading}
    <Skeleton rows={4} height={40} />
  {:else if conns.length === 0 && brokers.clusters.length === 0 && sections.length === 0}
    <EmptyState
      icon="plug"
      title="No connections yet"
      body="Create profiles for ssh, mysql, postgres, redis, mongodb, clickhouse, Kafka clusters or any custom CLI. Secrets go to the Keychain; open a DB profile in the workbench, a cluster in Message Brokers, or anything as a live terminal."
      actionLabel="New Connection"
      onaction={() => {
        editing = null;
        formOpen = true;
      }}
    />
  {:else}
    <!-- Search across all rows (flat results when active). -->
    <div class="conn-search">
      <Icon name="search" size={13} />
      <input
        class="conn-search-input"
        type="text"
        placeholder="Search — name, host, type, or section…"
        aria-label="Search connections"
        bind:value={search}
      />
      {#if search.trim()}
        <button class="conn-search-clear" onclick={() => (search = '')} aria-label="Clear search" title="Clear">
          <Icon name="x" size={12} />
        </button>
      {/if}
    </div>

    <!-- Type filter — single-select; composes with search (AND). -->
    <div class="filter-chips" role="group" aria-label="Filter by type">
      {#each FILTER_CHIPS as chip (chip.id)}
        <button
          class="filter-chip"
          class:on={filterKind === chip.id}
          data-testid={`connhub-filter-${chip.id}`}
          aria-pressed={filterKind === chip.id}
          onclick={() => setFilter(chip.id)}
        >
          {chip.label}
        </button>
      {/each}
    </div>

    {#if search.trim()}
      <div class="tree">
        <div class="section-head plain">
          <span class="caret-spacer"></span>
          <span class="section-name grow">Results</span>
          <span class="count">{searchCount}</span>
        </div>
        {#if searchCount === 0}
          <div class="search-empty">Nothing matches “{search.trim()}”.</div>
        {:else}
          {#each searchResults.conns as c (c.id)}
            {@render connRow(c, 1)}
          {/each}
          {#each searchResults.clusters as cl (cl.id)}
            {@render clusterRow(cl, 1)}
          {/each}
        {/if}
      </div>
    {:else}
    <div class="tree">
      <!-- Recent: pinned first, then most-recently-opened, capped at 6. Only
           shown when there is at least one pinned or previously-opened connection
           matching the active filter. -->
      {#if recent.length > 0}
        <div class="section-head plain">
          <span class="caret-spacer"></span>
          <span class="section-name grow">Recent</span>
          <span class="count">{recent.length}</span>
        </div>
        {#each recent as c (c.id)}
          {@render connRow(c, 1)}
        {/each}
      {/if}

      {#each tree as node (node.sec.id)}
        {@render sectionNode(node, 0)}
      {/each}

      <!-- Ungrouped doubles as the top-level / no-section drop target. -->
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <div
        class="section-head plain"
        class:drop-target={draggedConnId || draggedClusterId || draggedSectionId}
        ondragover={(e) => {
          if (draggedConnId || draggedClusterId || draggedSectionId) e.preventDefault();
        }}
        ondrop={(e) => {
          e.preventDefault();
          onRootDrop();
        }}
        title="Rows with no section (drop here to remove from a section / make a section top-level)"
      >
        <span class="caret-spacer"></span>
        <span class="section-name grow">Ungrouped</span>
        {#if ungroupedMatchCount > 0}<span class="count">{ungroupedMatchCount}</span>{/if}
      </div>
      {#each ungrouped.filter(connMatchesFilter) as c (c.id)}
        {@render connRow(c, 1)}
      {/each}
      {#if clusterMatchesFilter()}
        {#each ungroupedClusters as cl (cl.id)}
          {@render clusterRow(cl, 1)}
        {/each}
      {/if}
    </div>
    {/if}
  {/if}
</div>

{#if openMenuFor}
  <!-- Click-away backdrop for the open-in-workspace menu. -->
  <button class="menu-backdrop" aria-label="Close menu" onclick={() => (openMenuFor = null)}></button>
{/if}

{#snippet sectionNode(node: TreeNode, depth: number)}
  {@const isOpen = !collapsed[node.sec.id]}
  {@const matchCount = nodeMatchCount(node)}
  <!-- While a type filter is active, sections with no matching descendant vanish. -->
  {#if !filtering || matchCount > 0}
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div
      class="section-head"
      class:drop-target={(draggedSectionId && draggedSectionId !== node.sec.id) || draggedConnId || draggedClusterId}
      style="padding-left: {depth * 16 + 6}px"
      draggable="true"
      ondragstart={(e) => {
        draggedSectionId = node.sec.id;
        e.stopPropagation();
      }}
      ondragend={() => (draggedSectionId = null)}
      ondragover={(e) => {
        if (draggedConnId || draggedClusterId || (draggedSectionId && draggedSectionId !== node.sec.id)) e.preventDefault();
      }}
      ondrop={(e) => {
        e.preventDefault();
        e.stopPropagation();
        onSectionDrop(node.sec.id);
      }}
    >
      <button class="caret" onclick={() => toggleCollapse(node.sec.id)} aria-label="Toggle section">
        <Icon name={isOpen ? 'chevronDown' : 'chevronRight'} size={12} />
      </button>
      <Icon name="folder" size={13} />
      <span class="section-name grow ellipsis">{node.sec.name}</span>
      <span class="count">{matchCount}</span>
      <button class="icon-btn" title="Add sub-section" onclick={() => createSection(node.sec.id)}>
        <Icon name="plus" size={13} />
      </button>
      <button class="icon-btn" title="Rename section" onclick={() => renameSection(node.sec)}>
        <Icon name="edit" size={13} />
      </button>
      <button class="icon-btn" title="Delete section" onclick={() => deleteSection(node.sec)}>
        <Icon name="trash" size={13} />
      </button>
    </div>
    {#if isOpen}
      {#each node.items.filter(connMatchesFilter) as c (c.id)}
        {@render connRow(c, depth + 1)}
      {/each}
      {#if clusterMatchesFilter()}
        {#each node.clusters as cl (cl.id)}
          {@render clusterRow(cl, depth + 1)}
        {/each}
      {/if}
      {#each node.children as child (child.sec.id)}
        {@render sectionNode(child, depth + 1)}
      {/each}
    {/if}
  {/if}
{/snippet}

{#snippet connRow(c: Connection, depth: number)}
  {@const r = testResults[c.id]}
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div
    class="conn-row"
    class:dragging={draggedConnId === c.id}
    style="padding-left: {depth * 16 + 8}px"
    data-testid="conn-row"
    draggable="true"
    ondragstart={(e) => {
      draggedConnId = c.id;
      e.stopPropagation();
    }}
    ondragend={() => (draggedConnId = null)}
    ondblclick={(e) => {
      // Double-click the row to open it — DB engines land in the workbench, the
      // rest attach a terminal session. Not when the dblclick lands on a button.
      if ((e.target as HTMLElement).closest('button')) return;
      if (opening[c.id]) return;
      if (isDbKind(c.kind)) void openDb(c);
      else void open(c);
    }}
    title={c.first_command ? `${c.name} · ▸ ${c.first_command} — double-click to open` : `${c.name} — double-click to open`}
  >
    <!-- Identity line. `display:contents` on desktop → the row stays one flat
         flex line; on phone (≤640px) this becomes the first row above the
         actions strip so the name/host/badges read clearly. -->
    <div class="conn-main">
      <span class="conn-dot"><Icon name={kindIcons[c.kind]} size={13} /></span>
      {#if c.pinned}<span class="pin-glyph" title="Pinned"><Icon name="pin" size={11} /></span>{/if}
      <span class="conn-name ellipsis">{c.name}</span>
      {#if c.environment === 'prod'}
        <span class="env-chip prod" title="Production — writes require typed confirmation">PROD</span>
      {:else if c.read_only}
        <span class="env-chip ro" title="Read-only — writes refused">RO</span>
      {:else if c.environment === 'staging'}
        <span class="env-chip stg" title="Staging">STG</span>
      {/if}
      <span class="conn-desc mono ellipsis">{describe(c)}</span>
      <span class="grow"></span>
      {#if c.kind === 'clickhouse'}
        <span class="chip warn" title="password passes via argv on the host">argv</span>
      {/if}
      {#if r}
        <span class="chip {r.ok ? 'ok' : 'bad'}" title={r.message}>
          {r.ok ? `ok · ${r.latency_ms}ms` : 'failed'}
        </span>
      {/if}
      {#if r?.warn_key_perms}
        <!-- SSH key file is group/other-readable; ssh may refuse it. Click to
             see the message + the exact `chmod 600 <path>` fix with a copy. -->
        <div class="keyperms-wrap">
          <button
            type="button"
            class="chip warn keyperms-chip"
            title={r.warn_key_perms}
            aria-label="SSH key permissions warning — click for the fix"
            aria-expanded={keyPermsOpenFor === c.id}
            onclick={(e) => {
              e.stopPropagation();
              keyPermsOpenFor = keyPermsOpenFor === c.id ? null : c.id;
            }}
          >
            <Icon name="key" size={11} />
            key perms
          </button>
          {#if keyPermsOpenFor === c.id}
            <!-- Lightweight popover: message + copyable fix command. -->
            <div class="keyperms-pop" role="dialog" aria-label="SSH key permissions">
              <div class="keyperms-pop-head">
                <Icon name="key" size={12} />
                <span>Insecure SSH key permissions</span>
                <button
                  type="button"
                  class="btn small icon-only keyperms-close"
                  aria-label="Close"
                  onclick={(e) => {
                    e.stopPropagation();
                    keyPermsOpenFor = null;
                  }}
                >
                  <Icon name="x" size={12} />
                </button>
              </div>
              <p class="keyperms-msg">{r.warn_key_perms}</p>
              {#if keyPermsFix(r.warn_key_perms)}
                {@const fix = keyPermsFix(r.warn_key_perms)}
                <div class="keyperms-fix">
                  <code class="mono ellipsis">{fix}</code>
                  <button
                    type="button"
                    class="btn small"
                    onclick={(e) => {
                      e.stopPropagation();
                      copyKeyPermsFix(fix);
                    }}
                  >
                    <Icon name="file" size={11} /> Copy
                  </button>
                </div>
              {/if}
            </div>
          {/if}
        </div>
      {/if}
    </div>
    <!-- Actions strip. `display:contents` on desktop (no visual change); on phone
         it becomes its own wrapping row so every action stays reachable. -->
    <div class="conn-actions">
    {#if isDbKind(c.kind)}
      <!-- DB engine: primary Open goes to the Database workbench; the terminal
           client is a secondary action. -->
      <button class="btn small primary open-main-solo" disabled={opening[c.id]} onclick={() => openDb(c)} title="Open in the Database workbench">
        <Icon name="play" size={11} />
        {opening[c.id] ? 'Opening…' : 'Open'}
      </button>
      <button
        class="btn small icon-only"
        title="Open terminal client — attach a DB CLI as a terminal session"
        aria-label="Open terminal client"
        disabled={opening[c.id]}
        onclick={() => open(c)}
      >
        <Icon name="terminal" size={12} />
      </button>
    {:else}
      <!-- Open as a session attached to a workspace; the caret picks which one. -->
      <div class="open-split">
        <button class="btn small primary open-main" disabled={opening[c.id]} onclick={() => open(c)}>
          <Icon name="play" size={11} />
          {opening[c.id] ? 'Opening…' : 'Open'}
        </button>
        <button
          class="btn small primary open-caret"
          title="Open in a specific workspace…"
          aria-label="Open in workspace"
          disabled={opening[c.id]}
          onclick={(e) => {
            e.stopPropagation();
            openMenuFor = openMenuFor === c.id ? null : c.id;
          }}
        >
          <Icon name="chevronDown" size={11} />
        </button>
        {#if openMenuFor === c.id}
          <div class="open-menu">
            <div class="open-menu-title">Attach session to…</div>
            {#each ws.workspaces as w (w.id)}
              <button class="open-menu-item" onclick={() => open(c, w.id)}>
                <span class="ellipsis">{w.name}</span>
                {#if w.id === ws.currentId}<span class="cur">current</span>{/if}
              </button>
            {/each}
          </div>
        {/if}
      </div>
    {/if}
    <button
      class="btn small icon-only"
      title="Open beside — adds this terminal as a split pane next to the current session"
      aria-label="Open beside"
      disabled={openingSplit[c.id]}
      onclick={() => openBeside(c)}
    >
      <Icon name="split" size={12} />
    </button>
    <button class="btn small" disabled={testing[c.id]} onclick={() => test(c)}>
      {testing[c.id] ? 'Testing…' : 'Test'}
    </button>
    {#if c.kind === 'ssh'}
      <button
        class="icon-btn"
        title="Browse files (SFTP)"
        aria-label="Browse files"
        onclick={() => (sftpFor = c)}
      >
        <Icon name="folder" size={13} />
      </button>
    {/if}
    <button
      class="icon-btn"
      title="Edit"
      onclick={() => {
        editing = c;
        formOpen = true;
      }}
    >
      <Icon name="edit" size={13} />
    </button>
    <button
      class="icon-btn"
      title={c.pinned ? 'Unpin' : 'Pin to top'}
      onclick={() => togglePin(c)}
    >
      <Icon name="pin" size={13} />
    </button>
    <button class="icon-btn" title="Delete" onclick={() => remove(c)}>
      <Icon name="trash" size={13} />
    </button>
    </div>
  </div>
{/snippet}

{#snippet clusterRow(cl: BrokerCluster, depth: number)}
  <!-- A message-broker cluster filed into the shared tree. Opening it jumps to
       Message Brokers; drag it to file it like any other row. -->
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div
    class="conn-row cluster-row"
    class:dragging={draggedClusterId === cl.id}
    style="padding-left: {depth * 16 + 8}px"
    data-testid="cluster-row"
    draggable="true"
    ondragstart={(e) => {
      draggedClusterId = cl.id;
      e.stopPropagation();
    }}
    ondragend={() => (draggedClusterId = null)}
    ondblclick={(e) => {
      if ((e.target as HTMLElement).closest('button')) return;
      openCluster(cl);
    }}
    title={`${cl.name} — double-click to open in Message Brokers`}
  >
    <div class="conn-main">
      <span
        class="conn-dot cluster-dot"
        style={cl.color ? `background:color-mix(in srgb, ${cl.color} 16%, transparent);color:${cl.color}` : ''}
      >
        <Icon name="box" size={13} />
      </span>
      <span class="conn-name ellipsis">{cl.name}</span>
      <span class="kind-badge" title="Kafka cluster">kafka</span>
      {#if cl.environment === 'prod'}
        <span class="env-chip prod" title="Production">PROD</span>
      {:else if cl.read_only}
        <span class="env-chip ro" title="Read-only">RO</span>
      {:else if cl.environment === 'staging'}
        <span class="env-chip stg" title="Staging">STG</span>
      {/if}
      <span class="conn-desc mono ellipsis">{cl.bootstrap_servers}</span>
      <span class="grow"></span>
    </div>
    <div class="conn-actions">
      <button class="btn small primary open-main-solo" onclick={() => openCluster(cl)} title="Open in Message Brokers">
        <Icon name="play" size={11} /> Open
      </button>
      <button class="icon-btn" title="Edit cluster" onclick={() => editCluster(cl)}>
        <Icon name="edit" size={13} />
      </button>
      <button class="icon-btn" title="Remove cluster" onclick={() => removeCluster(cl)}>
        <Icon name="trash" size={13} />
      </button>
    </div>
  </div>
{/snippet}

{#if formOpen}
  <ConnectionForm
    existing={editing}
    kinds={ALL_KINDS}
    onclose={() => (formOpen = false)}
    onsaved={onSaved}
  />
{/if}

{#if clusterFormOpen}
  <ClusterForm cluster={editingCluster} onclose={() => (clusterFormOpen = false)} />
{/if}

{#if sftpFor}
  <SftpBrowser conn={sftpFor} onclose={() => (sftpFor = null)} />
{/if}

{#if importOpen && ws.currentId}
  <ConnectionImportDialog
    wsId={ws.currentId}
    onclose={() => (importOpen = false)}
    onimported={() => {
      if (ws.currentId) void loadConns(ws.currentId);
    }}
  />
{/if}

<style>
  .conn-search {
    display: flex;
    align-items: center;
    gap: 7px;
    padding: 6px 10px;
    margin-bottom: 8px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--surface-raised, var(--surface));
    color: var(--text-dim);
  }
  .conn-search:focus-within {
    border-color: var(--accent);
  }
  .conn-search-input {
    flex: 1;
    min-width: 0;
    border: none;
    background: transparent;
    color: var(--text);
    font-size: 13px;
    outline: none;
  }
  .conn-search-input::placeholder {
    color: var(--text-dim);
  }
  .conn-search-clear {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    border: none;
    background: transparent;
    color: var(--text-dim);
    cursor: pointer;
    padding: 2px;
    border-radius: 3px;
  }
  .conn-search-clear:hover {
    color: var(--text);
  }
  .search-empty {
    padding: 16px 4px;
    font-size: 13px;
    color: var(--text-dim);
    font-style: italic;
  }
  /* Type filter chips — single-select row under the search box. */
  .filter-chips {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    margin-bottom: 10px;
  }
  .filter-chip {
    height: 26px;
    padding: 0 11px;
    border-radius: 999px;
    border: 1px solid var(--border);
    background: var(--surface-2);
    font-size: 12px;
    color: var(--text-dim);
    cursor: pointer;
    transition: all 130ms ease-out;
  }
  .filter-chip:hover {
    color: var(--text);
  }
  .filter-chip.on {
    background: color-mix(in srgb, var(--accent) 15%, transparent);
    border-color: color-mix(in srgb, var(--accent) 45%, transparent);
    color: var(--accent);
    font-weight: 500;
  }
  /* Small type badge on cluster rows (mirrors the row identity line). */
  .kind-badge {
    font-size: 9px;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    padding: 1px 5px;
    border-radius: 3px;
    flex-shrink: 0;
    background: color-mix(in srgb, var(--accent) 12%, transparent);
    color: var(--accent);
  }
  .cluster-dot {
    background: color-mix(in srgb, var(--accent) 14%, transparent);
    color: var(--accent);
  }
  .tree {
    display: flex;
    flex-direction: column;
    gap: 1px;
  }
  .section-head {
    display: flex;
    align-items: center;
    gap: 6px;
    height: 30px;
    padding: 0 8px 0 6px;
    border-radius: var(--radius-s);
    cursor: grab;
    user-select: none;
  }
  .section-head:hover {
    background: color-mix(in srgb, var(--text-dim) 10%, transparent);
  }
  .section-head.plain {
    cursor: default;
    margin-top: 6px;
  }
  .section-head.drop-target {
    outline: 1px dashed color-mix(in srgb, var(--accent) 55%, transparent);
    outline-offset: -1px;
    background: color-mix(in srgb, var(--accent) 8%, transparent);
  }
  .section-name {
    font-size: 11px;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--text-dim);
  }
  .caret {
    display: grid;
    place-items: center;
    width: 18px;
    height: 18px;
    border: none;
    background: transparent;
    color: var(--text-dim);
    border-radius: var(--radius-s);
    cursor: pointer;
    flex-shrink: 0;
  }
  .caret:hover {
    color: var(--text);
  }
  .caret-spacer {
    width: 18px;
    flex-shrink: 0;
  }
  .count {
    font-size: 10px;
    color: var(--text-dim);
    min-width: 16px;
    text-align: center;
  }
  .conn-row {
    display: flex;
    align-items: center;
    gap: 8px;
    height: 34px;
    padding: 0 8px;
    border-radius: var(--radius-s);
    transition: background 120ms ease-out;
  }
  /* On desktop the identity line + actions strip are layout-transparent, so the
     row renders as one flat flex line exactly as before. They only take effect
     in the phone media query below. */
  .conn-main,
  .conn-actions {
    display: contents;
  }
  .conn-row:hover {
    background: color-mix(in srgb, var(--text-dim) 8%, transparent);
  }
  .conn-row.dragging {
    opacity: 0.5;
  }
  .conn-dot {
    width: 22px;
    height: 22px;
    border-radius: var(--radius-s);
    background: color-mix(in srgb, var(--accent) 14%, transparent);
    color: var(--accent);
    display: grid;
    place-items: center;
    flex-shrink: 0;
  }
  .conn-name {
    font-size: 12.5px;
    font-weight: 600;
    display: flex;
    gap: 6px;
    align-items: center;
    flex-shrink: 0;
    max-width: 280px;
  }
  .conn-desc {
    font-size: 11px;
    color: var(--text-dim);
    min-width: 0;
    flex: 0 1 auto;
  }
  .ellipsis {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .chip.warn {
    color: #b8860b;
    background: color-mix(in srgb, #b8860b 14%, transparent);
  }
  /* Clickable amber chip + the fix popover it anchors. */
  .keyperms-wrap {
    position: relative;
    display: inline-flex;
  }
  .chip.warn.keyperms-chip {
    cursor: pointer;
    border-color: color-mix(in srgb, #b8860b 35%, transparent);
  }
  .chip.warn.keyperms-chip:hover {
    background: color-mix(in srgb, #b8860b 22%, transparent);
  }
  .keyperms-pop {
    position: absolute;
    top: calc(100% + 6px);
    right: 0;
    z-index: 30;
    width: min(360px, 78vw);
    padding: 10px;
    border: 1px solid var(--border);
    border-radius: 8px;
    background: var(--surface-1, var(--surface-2));
    box-shadow: 0 8px 24px rgba(0, 0, 0, 0.28);
    display: flex;
    flex-direction: column;
    gap: 8px;
    text-align: start;
    white-space: normal;
  }
  .keyperms-pop-head {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 12px;
    font-weight: 600;
    color: #b8860b;
  }
  .keyperms-pop-head span {
    flex: 1;
  }
  .keyperms-close {
    color: var(--text-dim);
  }
  .keyperms-msg {
    margin: 0;
    font-size: 11.5px;
    line-height: 1.45;
    color: var(--text-dim);
  }
  .keyperms-fix {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 6px 8px;
    border-radius: 6px;
    background: var(--surface-2);
    border: 1px solid var(--border);
  }
  .keyperms-fix code {
    flex: 1;
    min-width: 0;
    font-size: 11.5px;
    color: var(--text);
  }
  .header-actions {
    display: flex;
    gap: 8px;
    align-items: center;
  }
  /* Square-ish split-pane action sitting next to "Open". */
  .btn.small.icon-only {
    padding: 0 6px;
    color: var(--text-dim);
  }
  .btn.small.icon-only:hover {
    color: var(--text);
  }
  /* Split "Open ▾" — primary button + workspace-picker caret. */
  .open-split {
    position: relative;
    display: inline-flex;
    align-items: stretch;
  }
  .open-main {
    border-top-right-radius: 0;
    border-bottom-right-radius: 0;
  }
  .open-caret {
    border-top-left-radius: 0;
    border-bottom-left-radius: 0;
    padding: 0 5px;
    margin-inline-start: 1px;
  }
  .open-menu {
    position: absolute;
    top: calc(100% + 4px);
    inset-inline-end: 0;
    z-index: 50;
    min-width: 180px;
    max-height: 280px;
    overflow-y: auto;
    padding: 4px;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    box-shadow: 0 8px 24px rgba(0, 0, 0, 0.28);
  }
  .open-menu-title {
    font-size: 10px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--text-dim);
    padding: 4px 8px 6px;
  }
  .open-menu-item {
    display: flex;
    align-items: center;
    gap: 6px;
    width: 100%;
    height: 30px;
    padding: 0 8px;
    border: none;
    background: transparent;
    color: var(--text);
    cursor: pointer;
    border-radius: var(--radius-s);
    font-size: 12.5px;
    text-align: start;
  }
  .open-menu-item:hover {
    background: color-mix(in srgb, var(--accent) 14%, transparent);
  }
  .open-menu-item .cur {
    margin-inline-start: auto;
    font-size: 9.5px;
    text-transform: uppercase;
    letter-spacing: 0.03em;
    color: var(--accent);
  }
  .menu-backdrop {
    position: fixed;
    inset: 0;
    z-index: 40;
    border: none;
    background: transparent;
    padding: 0;
    cursor: default;
  }
  /* Recency / pin indicators */
  .pin-glyph {
    display: inline-flex;
    align-items: center;
    color: var(--accent);
    flex-shrink: 0;
  }
  /* Environment badges */
  .env-chip {
    font-size: 9px;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    padding: 1px 5px;
    border-radius: 3px;
    flex-shrink: 0;
    background: var(--surface-2);
    color: var(--text-dim);
  }
  .env-chip.prod {
    background: color-mix(in srgb, var(--status-exited) 20%, transparent);
    color: var(--status-exited);
  }
  .env-chip.ro {
    background: color-mix(in srgb, var(--accent) 15%, transparent);
    color: var(--accent);
  }
  .env-chip.stg {
    background: color-mix(in srgb, orange 15%, transparent);
    color: orange;
  }

  /* ───────────────── Phone (≤640px) ─────────────────
     The desktop row crams the name + host + status chips + SEVEN action controls
     (Open ▾, split, Test, SFTP, Edit, Pin, Delete) onto one fixed-height flex
     line. On a phone that overflows the viewport horizontally — the actions run
     off the right edge, unreachable, and the whole document scrolls sideways.
     On a phone we stack each connection into a card: an identity line (icon +
     name + badges + host) over a wrapping action strip, so every control is
     reachable and the page only scrolls vertically. The header buttons wrap to
     full width, and the section-head icon controls grow to tap size. Desktop
     (≥641px) is untouched — the wrappers stay `display:contents`. */
  @media (max-width: 640px) {
    /* Header: title block over full-width action buttons (no side overflow). */
    .page-header {
      flex-direction: column;
      align-items: stretch;
    }
    .header-actions {
      width: 100%;
      /* Four actions (New Section / Import / New Cluster / New Connection) can't
         shrink below their label on a narrow phone — wrap them into rows (two per
         line) instead of letting one jut past the viewport edge. */
      flex-wrap: wrap;
    }
    .header-actions :global(.btn) {
      flex: 1 1 45%;
      min-width: 0;
      min-height: 38px;
    }

    /* Bump the page sub-line & search to a comfortably readable size. */
    .conn-search-input {
      font-size: 14px;
    }

    /* Each connection becomes a two-tier card. */
    .conn-row {
      height: auto;
      flex-direction: column;
      align-items: stretch;
      gap: 8px;
      padding: 10px 10px;
      /* Override the inline depth padding-left so nested rows don't shove the
         card off the right edge on a narrow phone. */
      padding-left: 10px !important;
      border: 1px solid var(--border);
      margin: 2px 0;
    }
    .conn-main {
      display: flex;
      align-items: center;
      gap: 8px;
      min-width: 0;
    }
    /* The name can shrink/ellipsize so badges + host stay on the line. */
    .conn-name {
      font-size: 15px;
      max-width: none;
      flex: 0 1 auto;
      min-width: 0;
    }
    .conn-desc {
      font-size: 12px;
    }
    /* Actions: a horizontally-scrollable strip so all seven controls are always
       reachable without wrapping into an unpredictable number of rows. */
    .conn-actions {
      display: flex;
      align-items: center;
      gap: 8px;
      overflow-x: auto;
      -webkit-overflow-scrolling: touch;
      padding-bottom: 2px;
      scrollbar-width: none;
    }
    .conn-actions::-webkit-scrollbar {
      display: none;
    }
    /* Tap-sized controls. The split "Open" stays grouped; everything else gets
       a comfortable hit area and never shrinks below its label. */
    .conn-actions :global(.btn),
    .conn-actions :global(.icon-btn) {
      min-height: 38px;
      flex-shrink: 0;
    }
    .conn-actions :global(.btn.small) {
      font-size: 13px;
      padding: 0 12px;
    }
    .conn-actions :global(.icon-btn) {
      min-width: 38px;
      display: grid;
      place-items: center;
    }
    .open-main {
      padding-inline: 14px;
    }

    /* Section / group headers: larger caret + tap-sized inline icon buttons,
       and let them wrap rather than push off-screen. */
    .section-head {
      height: auto;
      min-height: 40px;
      flex-wrap: wrap;
      padding-right: 4px;
    }
    .section-name {
      font-size: 12px;
    }
    .section-head :global(.icon-btn) {
      min-width: 34px;
      min-height: 34px;
      display: grid;
      place-items: center;
    }
    .count {
      font-size: 12px;
    }
  }
</style>
