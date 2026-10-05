<script lang="ts">
  import { rowMenu } from '../../lib/rowMenu';
  import Badge from '../../lib/components/Badge.svelte';
  import { focusOnMount } from '../../lib/focusOnMount';
  import { NO_WORKSPACE } from '../../lib/labels';
  import { plural } from '../../lib/plural';
  import { toastError } from '../../lib/toastError';
  // DB Explorer page (mirrors ApiPage): left sidebar = connection picker +
  // SchemaTree + a Saved/History switch; main = a tab strip (Query / Builder /
  // Structure / Dashboards) over the active view.
  import { tick } from 'svelte';
  import Icon, { type IconName } from '../../lib/components/Icon.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import { paneResizer, loadPaneWidth, LIST_PANE } from '../../lib/paneResizer';
  import PaneDivider from '../../lib/components/PaneDivider.svelte';
  import PageBody from '../../lib/components/PageBody.svelte';
  import EnvBadge from '../../lib/components/EnvBadge.svelte';
  import { envTone } from '../../lib/status';
  import LoadState from '../../lib/components/LoadState.svelte';
  import PageHeader from '../../lib/components/PageHeader.svelte';
  import SchemaTree from './SchemaTree.svelte';
  import QueryEditor from './QueryEditor.svelte';
  import ConnectionComparison from './ConnectionComparison.svelte';
  import Modal from '../../lib/components/Modal.svelte';
  import DatabaseChanges from './DatabaseChanges.svelte';
  import ResourceAccess from '../../lib/components/ResourceAccess.svelte';
  import { resourceAccess } from '../../lib/stores/resource-access.svelte';
  import { auth } from '../../lib/stores/auth.svelte';
  import ConnectionForm from '../connections/ConnectionForm.svelte';
  import ConnectionImportDialog from '../connections/ConnectionImportDialog.svelte';
  import LazyMount from '../../lib/components/LazyMount.svelte';
  import { lazyComponent, whenIdle } from '../../lib/lazy-component.svelte';
  import { PRIMARY_SCROLLBACK } from '../../lib/components/termFlow';
  import ImportDialog from './ImportDialog.svelte';
  import ExportDialog from './ExportDialog.svelte';
  import { stmtPreview } from './sql-util';
  import { databaseAccessChild } from '../../lib/access-options';
  import { database, engineGlyph, type DbMainTab } from '../../lib/stores/database.svelte';
  import { brokers } from '../../lib/stores/brokers.svelte';
  import { ws, DB_PANE_ID } from '../../lib/stores/workspace.svelte';
  import { viewport } from '../../lib/stores/viewport.svelte';
  import { api } from '../../lib/api/client';
  import { confirmer } from '../../lib/confirm.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { ctxMenu } from '../../lib/contextmenu.svelte';
  import { onTabKey } from '../../lib/tabKeys';
  import { popoutItems } from '../../lib/popoutMenu';
  import { router } from '../../lib/router.svelte';
  import { registry } from '../../lib/commands.svelte';
  import type {
    BrokerCluster,
    Connection,
    ConnectionKind,
    ConnectionSection,
    DbSavedQuery,
    Session,
  } from '../../lib/api/types';

  // Secondary views and the SSH/Kafka/SFTP panes load on first open, not with
  // the page: xterm, the Kafka viewer (+ lag alerts) and ~4.7k lines of
  // builder/structure/diagram used to ride in the page chunk every DB open
  // paid for. The common views are warmed on idle after first paint; the
  // terminal/Kafka/SFTP chunks only when a pane of that kind opens.
  const QueryBuilderLazy = lazyComponent(() => import('./QueryBuilder.svelte'));
  const StructureLazy = lazyComponent(() => import('./StructureView.svelte'));
  const DiagramLazy = lazyComponent(() => import('./DiagramView.svelte'));
  const DashboardsLazy = lazyComponent(() => import('./Dashboards.svelte'));
  const TerminalLazy = lazyComponent(() => import('../../lib/components/Terminal.svelte'));
  const ClusterViewerLazy = lazyComponent(() => import('../brokers/ClusterViewer.svelte'));
  const ClusterFormLazy = lazyComponent(() => import('../brokers/ClusterForm.svelte'));
  const SftpLazy = lazyComponent(() => import('../connections/SftpBrowser.svelte'));
  // The assistant embeds a Terminal (xterm) — only loaded when it's opened.
  const AssistantLazy = lazyComponent(() => import('./DbAssistantPanel.svelte'));
  $effect(() =>
    whenIdle(() => {
      StructureLazy.prefetch();
      QueryBuilderLazy.prefetch();
      DashboardsLazy.prefetch();
      DiagramLazy.prefetch();
    }, 4000),
  );

  // The unified Connections hub: EVERY profile kind is created/managed in this
  // tree — DB engines open the workbench, ssh/custom open a terminal session,
  // Kafka clusters open Message Brokers.
  const ALL_KINDS: ConnectionKind[] = [
    'ssh',
    'mysql',
    'postgres',
    'redis',
    'mongodb',
    'clickhouse',
    'custom',
  ];
  let connFormOpen = $state(false);
  let editingConn = $state<Connection | null>(null);
  // Import connection profiles from other DB tools (MySQL Workbench / DBeaver /
  // DataGrip / NoSQLBooster) — the daemon reads each tool's config from disk.
  let connImportOpen = $state(false);
  // Kafka cluster form (New Cluster / edit a cluster row).
  let clusterFormOpen = $state(false);
  let editingCluster = $state<BrokerCluster | null>(null);
  // SFTP file browser over an ssh profile's row action.
  let sftpFor = $state<Connection | null>(null);
  // Spinner guard while a terminal session is being spawned for a row.
  let opening = $state<Record<string, boolean>>({});

  // ── Type-filter chips (single-select; 'kafka' matches broker clusters) ─────
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
  let filterKind = $state(
    typeof localStorage === 'undefined' ? 'all' : localStorage.getItem(FILTER_KEY) || 'all',
  );
  function setFilter(id: string): void {
    filterKind = id;
    if (typeof localStorage !== 'undefined') localStorage.setItem(FILTER_KEY, id);
  }
  // Only offer kinds that exist (plus the active one, so a remembered filter
  // can always be cleared). One kind or none → no chip row at all: a filter
  // that can't narrow anything is noise.
  const presentKinds = $derived.by(() => {
    const kinds = new Set<string>([...database.connections, ...database.otherConnections].map((c) => c.kind));
    if (brokers.clusters.length > 0) kinds.add('kafka');
    return kinds;
  });
  const visibleChips = $derived(
    FILTER_CHIPS.filter((chip) => chip.id === 'all' || chip.id === filterKind || presentKinds.has(chip.id)),
  );
  const filtering = $derived(filterKind !== 'all');
  const connMatchesKind = (c: Connection): boolean => !filtering || c.kind === filterKind;
  const clusterMatchesKind = (): boolean => !filtering || filterKind === 'kafka';

  // ── Saved / History sidebar (search + inline rename) ──────────────────────
  let savedSearch = $state('');
  let historySearch = $state('');
  let renamingId = $state<string | null>(null);
  let renameDraft = $state('');
  // Lower-cased search text per row, memoized on the row's text — not
  // re-lowercasing every full statement (which can be a whole pasted script)
  // on each search keystroke.
  const searchText = new WeakMap<object, { src: string; lower: string }>();
  function lowered(row: object, text: string): string {
    const hit = searchText.get(row);
    if (hit && hit.src === text) return hit.lower;
    const lower = text.toLowerCase();
    searchText.set(row, { src: text, lower });
    return lower;
  }
  const filteredSaved = $derived.by(() => {
    const s = savedSearch.trim().toLowerCase();
    if (!s) return database.savedQueries;
    return database.savedQueries.filter(
      (q) => q.name.toLowerCase().includes(s) || lowered(q, q.statement).includes(s),
    );
  });
  const filteredHistory = $derived.by(() => {
    const s = historySearch.trim().toLowerCase();
    if (!s) return database.history;
    return database.history.filter((h) =>
      lowered(h, h.error ? `${h.statement}\n${h.error}` : h.statement).includes(s),
    );
  });
  function startRename(q: DbSavedQuery): void {
    renamingId = q.id;
    renameDraft = q.name;
  }
  async function commitRename(): Promise<void> {
    const id = renamingId;
    if (id && renameDraft.trim()) await database.renameSavedQuery(id, renameDraft);
    renamingId = null;
  }
  function cancelRename(): void {
    renamingId = null;
  }

  // ── Phone accordion ────────────────────────────────────────────────────────
  // On a phone the whole page scrolls and each major section (Connections /
  // Schema) is a collapsible, independently-scrolling block — tap a header to
  // expand/minimise so the user keeps only what they need on screen. These
  // flags are inert on desktop/tablet (the headers only render when isPhone).
  let connOpen = $state(true);
  let schemaOpen = $state(true);

  // Dock this connection as a pane in the Agents split (beside an agent), with
  // the full DB Explorer. Right-clicked from a connection tab or sidebar row.
  function openConnInAgents(c: Connection): void {
    void database.openConnection(c.id);
    ws.openInSplit(DB_PANE_ID);
    router.go('agents');
  }
  // The folder path a connection sits under, e.g. "PLATFORM / STG" — so it's
  // clear which environment (stg/prod) a connection belongs to.
  function sectionPath(c: Connection): string {
    if (!c.section_id) return '';
    const byId = new Map(sections.map((s) => [s.id, s]));
    const parts: string[] = [];
    let cur = byId.get(c.section_id);
    let guard = 0;
    while (cur && guard++ < 20) {
      parts.unshift(cur.name);
      cur = cur.parent_id ? byId.get(cur.parent_id) : undefined;
    }
    return parts.join(' / ');
  }
  // The immediate folder (sub-section), e.g. "STG" — compact, for the tab badge.
  function sectionLeaf(c: Connection): string {
    if (!c.section_id) return '';
    return sections.find((s) => s.id === c.section_id)?.name ?? '';
  }
  $effect(()=>resourceAccess.subscribe(change=>{
    if(change.type==='reset' && change.identity){connFormOpen=false;connImportOpen=false;editingConn=null;accessFor=null;sftpFor=null;changesOpen=false;}
  }));
  let changesOpen = $state(false);
  let accessFor = $state<Connection | null>(null);
  const connectionAccess = (c: Connection, operation: string, capability: 'view'|'edit'|'admin'='edit') => resourceAccess.can('connection',c.id,operation,'connections',capability);
  $effect(() => { for (const c of [...database.connections,...database.otherConnections]) void resourceAccess.load('connection',c.id); });
  function connMenu(e: MouseEvent, c: Connection): void {
    const isDb = database.connections.some((x) => x.id === c.id);
    ctxMenu.show(e, [
      // Per-kind primary/secondary opens: DB kinds live in the workbench but
      // keep their CLI client; ssh adds the SFTP file browser.
      ...(isDb
        ? [
            { label: 'Open beside agents (split)', icon: 'split', action: () => openConnInAgents(c) },
            ...(connectionAccess(c,'shell') ? [{ label: 'Open terminal client', icon: 'terminal', action: () => void openTerminal(c) }] : []),
          ]
        : connectionAccess(c,'shell') ? [{ label: 'Open terminal session', icon: 'terminal', action: () => void openTerminal(c) }] : []),
      ...(c.kind === 'ssh' && connectionAccess(c,'sftp_read','view')
        ? [{ label: 'Browse files (SFTP)', icon: 'folder', action: () => (sftpFor = c) }]
        : []),
      ...(isDb && database.isWarming(c.id)
        ? [{ label: 'Stop connecting', icon: 'stop', action: () => database.stopConnecting(c.id) }]
        : []),
      ...(isDb && database.connStatus.get(c.id)?.phase === 'idle' && database.selectedConnId !== c.id
        ? [{ label: 'Connect in background', icon: 'refresh', action: () => void database.warm(c.id) }]
        : []),
      ...(isDb ? popoutItems(`database/${c.id}`, c.name) : []),
      { separator: true },
      ...(connectionAccess(c,'configure','admin') ? [{ label: 'Edit', icon: 'edit', action: () => editConnection(c) }, { label: 'Delete…', icon: 'trash', danger: true, action: () => void deleteConnection(c) }] : []),
      ...(auth.isRoot && connectionAccess(c,'configure','admin') ? [{ label: 'Duplicate without password', icon: 'copy', action: () => void duplicateConnection(c) }] : []),
      ...(auth.isRoot || connectionAccess(c,'manage_access','admin') ? [{ label: 'Access', icon: 'key', action: () => {accessFor=c;} }] : []),
    ]);
  }

  /** A Kafka cluster's menu — its sidebar row and its open tab (right-click or ⋯). */
  function clusterMenu(e: MouseEvent, cl: BrokerCluster): void {
    ctxMenu.show(e, [
      { label: 'Open', icon: 'split', action: () => openCluster(cl) },
      { label: 'Open in Message Brokers page', icon: 'split', action: () => openClusterStandalone(cl) },
      { separator: true },
      { label: 'Edit', icon: 'edit', action: () => editCluster(cl) },
      { label: 'Remove…', icon: 'trash', danger: true, action: () => void deleteCluster(cl) },
    ]);
  }

  async function duplicateConnection(c: Connection): Promise<void> {
    try {
      const copy = await api.post<Connection>(`/connections/${c.id}/duplicate`, {});
      await database.loadConnections();
      editConnection(copy);
      toasts.info('Configuration duplicated', 'Set a password for the new connection. Access grants were not copied.');
    } catch (error) { toastError('Couldn’t duplicate connection', error); }
  }

  // --- Section hierarchy (THE unified tree: every kind + broker clusters) ----
  interface TreeNode {
    sec: ConnectionSection;
    items: Connection[];
    clusters: BrokerCluster[];
    children: TreeNode[];
  }
  let sections = $state<ConnectionSection[]>([]);
  let collapsed = $state<Record<string, boolean>>({});
  let draggedConnId = $state<string | null>(null);
  let draggedClusterId = $state<string | null>(null);
  let draggedSectionId = $state<string | null>(null);

  const sortByName = (a: Connection, b: Connection): number => a.name.localeCompare(b.name);
  const sortClusters = (a: BrokerCluster, b: BrokerCluster): number =>
    a.name.localeCompare(b.name);

  // Every profile the tree shows (DB kinds + ssh/custom), kind-filtered.
  const allConns = $derived(
    [...database.connections, ...database.otherConnections].filter(connMatchesKind),
  );
  const treeClusters = $derived(clusterMatchesKind() ? brokers.clusters : []);

  // Build the section tree from the flat list; `parentId = null` is the root.
  function buildTree(parentId: string | null): TreeNode[] {
    return sections
      .filter((s) => (s.parent_id ?? null) === parentId)
      .sort((a, b) => a.position - b.position || a.name.localeCompare(b.name))
      .map((sec) => ({
        sec,
        items: allConns.filter((c) => c.section_id === sec.id).sort(sortByName),
        clusters: treeClusters.filter((cl) => cl.section_id === sec.id).sort(sortClusters),
        children: buildTree(sec.id),
      }));
  }
  const tree = $derived(buildTree(null));
  // Known folder ids in the (global, shared) tree. A connection whose folder is
  // not among them falls back to Ungrouped, so connections never vanish.
  const knownSectionIds = $derived(new Set(sections.map((s) => s.id)));
  const ungrouped = $derived(
    allConns.filter((c) => !c.section_id || !knownSectionIds.has(c.section_id)).sort(sortByName),
  );
  const ungroupedClusters = $derived(
    treeClusters
      .filter((cl) => !cl.section_id || !knownSectionIds.has(cl.section_id))
      .sort(sortClusters),
  );
  /** Matching descendants under a node (drives count chips + hide-when-empty
   *  while a type filter narrows the tree). */
  function nodeCount(n: TreeNode): number {
    return (
      n.items.length + n.clusters.length + n.children.reduce((sum, c) => sum + nodeCount(c), 0)
    );
  }
  // Sections holding anything at all (any kind, unfiltered), ancestors
  // included. A truly empty folder isn't "filtered out" — it has nothing to
  // filter — so it stays visible under a type filter as a drop target.
  const occupiedSectionIds = $derived.by(() => {
    const parentOf = new Map(sections.map((s) => [s.id, s.parent_id ?? null]));
    const occ = new Set<string>();
    const mark = (id: string | null | undefined): void => {
      let cur = id ?? null;
      while (cur && parentOf.has(cur) && !occ.has(cur)) {
        occ.add(cur);
        cur = parentOf.get(cur) ?? null;
      }
    };
    for (const c of database.connections) mark(c.section_id);
    for (const c of database.otherConnections) mark(c.section_id);
    for (const cl of brokers.clusters) mark(cl.section_id);
    return occ;
  });
  // While anything is being dragged, a type filter reveals every folder so
  // any section is a reachable drop target. Flipped on a macrotask: mutating
  // the DOM inside `dragstart` makes WebKit abort the drag.
  let dragReveal = $state(false);
  $effect(() => {
    const dragging = !!(draggedConnId || draggedClusterId || draggedSectionId);
    if (!dragging) {
      dragReveal = false;
      return;
    }
    const t = setTimeout(() => (dragReveal = true), 0);
    return () => clearTimeout(t);
  });
  /** Under a type filter a folder shows when it has matches, is empty, has a
   *  visible sub-folder, or a drag is in progress. */
  function nodeVisible(n: TreeNode): boolean {
    return (
      !filtering ||
      dragReveal ||
      nodeCount(n) > 0 ||
      !occupiedSectionIds.has(n.sec.id) ||
      n.children.some(nodeVisible)
    );
  }

  // --- Connection search / filter --------------------------------------------
  // A filter box over the connection list (mirrors SchemaTree's "Filter schema").
  // When it has text we show a flat, name-sorted result list instead of the tree
  // so a connection is findable instantly even with a hundred of them.
  let connFilter = $state('');
  // Compact host/uri descriptor used for matching (and shown on flat results).
  function connDesc(c: Connection): string {
    const p = (c.params ?? {}) as Record<string, unknown>;
    const host = String(p.host ?? p.uri ?? p.url ?? p.path ?? '');
    const port = p.port != null && p.port !== '' ? `:${String(p.port)}` : '';
    return `${host}${port}`;
  }
  const connMatches = $derived.by(() => {
    const q = connFilter.trim().toLowerCase();
    if (!q) return [];
    return allConns
      .filter(
        (c) =>
          c.name.toLowerCase().includes(q) ||
          c.kind.toLowerCase().includes(q) ||
          connDesc(c).toLowerCase().includes(q) ||
          sectionPath(c).toLowerCase().includes(q),
      )
      .sort(sortByName);
  });
  const clusterMatches = $derived.by(() => {
    const q = connFilter.trim().toLowerCase();
    if (!q) return [];
    return treeClusters
      .filter(
        (cl) =>
          cl.name.toLowerCase().includes(q) ||
          'kafka'.includes(q) ||
          cl.bootstrap_servers.toLowerCase().includes(q),
      )
      .sort(sortClusters);
  });

  async function loadSections(): Promise<void> {
    const wsId = ws.currentId;
    if (!wsId) return;
    try {
      // One global tree shared with the Connections page.
      sections = await api.get<ConnectionSection[]>(`/workspaces/${wsId}/connection-sections`);
    } catch {
      /* sections are optional — fall back to a flat list */
    }
  }

  function toggleCollapse(id: string): void {
    collapsed[id] = !collapsed[id];
  }

  async function createSection(parentId: string | null): Promise<void> {
    if (!ws.currentId) return;
    const name = await confirmer.promptText(parentId ? 'Sub-section name' : 'Section name', {
      title: parentId ? 'New sub-section' : 'New section',
      confirmLabel: 'Create',
      placeholder: 'e.g. AWS · STG',
    });
    if (!name) return;
    try {
      const sec = await api.post<ConnectionSection>(
        `/workspaces/${ws.currentId}/connection-sections`,
        { name, parent_id: parentId },
      );
      sections = [...sections, sec];
    } catch (e) {
      toastError('Couldn’t create the section', e);
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
      toastError('Couldn’t rename', e);
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
      const removed = new Set<string>();
      const collect = (id: string): void => {
        removed.add(id);
        for (const s of sections) if (s.parent_id === id) collect(s.id);
      };
      collect(sec.id);
      sections = sections.filter((s) => !removed.has(s.id));
      // Locally drop the section_id of anything that fell out (rows never vanish).
      database.connections = database.connections.map((c) =>
        c.section_id && removed.has(c.section_id) ? { ...c, section_id: null } : c,
      );
      database.otherConnections = database.otherConnections.map((c) =>
        c.section_id && removed.has(c.section_id) ? { ...c, section_id: null } : c,
      );
      brokers.clusters = brokers.clusters.map((cl) =>
        cl.section_id && removed.has(cl.section_id) ? { ...cl, section_id: null } : cl,
      );
    } catch (e) {
      toastError('Couldn’t delete', e);
    }
  }

  // Move a connection into a folder (or null = ungrouped). Connections are
  // global, so all are assignable. Reuses the PATCH endpoint, updates in place.
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
      database.connections = database.connections.map((x) => (x.id === c.id ? saved : x));
      database.otherConnections = database.otherConnections.map((x) =>
        x.id === c.id ? saved : x,
      );
    } catch (e) {
      toastError('Couldn’t move', e);
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
      toastError('Couldn’t move', e);
    }
  }

  // A drop onto a section: a dragged connection/cluster files into it; a
  // dragged section nests under it. A drop onto the root zone reverses all.
  function findAnyConn(id: string): Connection | undefined {
    return (
      database.connections.find((x) => x.id === id) ??
      database.otherConnections.find((x) => x.id === id)
    );
  }
  function onSectionDrop(sectionId: string): void {
    if (draggedConnId) {
      const c = findAnyConn(draggedConnId);
      draggedConnId = null;
      if (c) void moveConn(c, sectionId);
    } else if (draggedClusterId) {
      const id = draggedClusterId;
      draggedClusterId = null;
      void brokers.moveCluster(id, sectionId);
    } else if (draggedSectionId) {
      const src = draggedSectionId;
      draggedSectionId = null;
      void reparentSection(src, sectionId);
    }
  }
  function onRootDrop(): void {
    if (draggedConnId) {
      const c = findAnyConn(draggedConnId);
      draggedConnId = null;
      if (c) void moveConn(c, null);
    } else if (draggedClusterId) {
      const id = draggedClusterId;
      draggedClusterId = null;
      void brokers.moveCluster(id, null);
    } else if (draggedSectionId) {
      const src = draggedSectionId;
      draggedSectionId = null;
      void reparentSection(src, null);
    }
  }

  function newConnection(): void {
    if(!auth.isRoot)return;
    editingConn = null;
    connFormOpen = true;
  }
  async function showSchemaSidebar(): Promise<void> {
    const page = document.querySelector('.db-page');
    database.toggleSidebar();
    await tick();
    const selected = [...(page?.querySelectorAll<HTMLElement>('[data-node-id]') ?? [])]
      .find((node) => node.dataset.nodeId === database.selectedObjectPath);
    (selected?.querySelector<HTMLButtonElement>('.node-label') ?? selected
      ?? page?.querySelector<HTMLInputElement>('[aria-label="Find an object"]'))?.focus();
  }

  // Point the user at the connection list from anywhere: make sure the sidebar
  // rail is actually visible (it may be collapsed) and land on the picker tab.
  function showConnections(): void {
    if (database.sidebarCollapsed) database.toggleSidebar();
    database.setSideTab('connections');
  }
  // The persistent "+" in the side-tab strip: jump to the picker AND open the
  // new-connection form — the create affordance must not live only inside the
  // Connections tab body.
  function newConnectionFromStrip(): void {
    showConnections();
    newConnection();
  }
  function editConnection(c: Connection): void {
    editingConn = c;
    connFormOpen = true;
  }

  // ⌘K: the Connections hub's verbs.
  $effect(() =>
    registry.register('connections', [
      { id: 'conn.new', title: 'New connection…', group: 'Connections', keywords: 'database ssh mysql postgres mongo redis clickhouse kafka add', disabled: !auth.isRoot, run: newConnectionFromStrip },
      { id: 'conn.import', title: 'Import connections…', group: 'Connections', keywords: 'workbench dbeaver datagrip nosqlbooster', disabled: !auth.isRoot, run: () => (connImportOpen = true) },
      { id: 'conn.list', title: 'Show the connection list', group: 'Connections', keywords: 'picker sidebar profiles', run: showConnections },
      { id: 'conn.sidebar', title: database.sidebarCollapsed ? 'Show the schema sidebar' : 'Hide the schema sidebar', group: 'Connections', shortcut: '⌘B', keywords: 'toggle collapse tree', run: () => (database.sidebarCollapsed ? void showSchemaSidebar() : database.toggleSidebar()) },
    ]),
  );
  async function onConnSaved(c: Connection): Promise<void> {
    connFormOpen = false;
    await database.loadConnections();
    // DB kinds open straight into the workbench; ssh/custom just appear in the
    // tree (opening them spawns a terminal, which the user does deliberately).
    if (database.connections.some((x) => x.id === c.id)) void database.openConnection(c.id);
  }
  function newCluster(): void {
    editingCluster = null;
    clusterFormOpen = true;
  }
  function editCluster(cl: BrokerCluster): void {
    editingCluster = cl;
    clusterFormOpen = true;
  }
  // Open a Kafka cluster as a workbench tab (in place, next to the DB tabs) —
  // the brokers store owns the tab list + selection; `focusKafka` points the main
  // area at it. No navigation: the viewer renders right here.
  function openCluster(cl: BrokerCluster): void {
    brokers.select(cl.id);
    database.focusKafka(cl.id);
  }
  // Escape hatch: open the cluster in the standalone Message Brokers page.
  function openClusterStandalone(cl: BrokerCluster): void {
    brokers.select(cl.id);
    router.go('brokers');
  }
  async function deleteCluster(cl: BrokerCluster): Promise<void> {
    if (
      // Same verb + consequence as the Message Brokers page and ClusterViewer's
      // "Remove" button, which lands here.
      !(await confirmer.ask(
        `Remove cluster “${cl.name}”? Its saved settings and Keychain secrets are removed from Otto; topics on the broker are not touched.`,
        { title: 'Remove cluster', confirmLabel: 'Remove' },
      ))
    )
      return;
    try {
      await brokers.remove(cl.id);
    } catch (e) {
      toastError('Couldn’t remove', e);
    }
  }

  /** Open an ssh/custom profile (or a DB kind's CLI client) as a live terminal
   *  tab in the workbench — the terminal renders in place, seamlessly, like the
   *  DB kinds. The session is also registered in the Agents list. */
  async function openTerminal(c: Connection): Promise<void> {
    // Already open as a workbench tab → just focus it (don't spawn a 2nd session).
    if (database.sshTabs.some((t) => t.connId === c.id)) {
      database.focusSsh(c.id);
      return;
    }
    const wsId = ws.currentId;
    if (!wsId) {
      toasts.error(NO_WORKSPACE, 'The database session is attached to a workspace — create or pick one.');
      return;
    }
    opening[c.id] = true;
    try {
      const session = await api.long.post<Session>(`/connections/${c.id}/open`, {
        workspace_id: wsId,
      });
      // Open it in place as a workbench tab (like the DB kinds). Deliberately NOT
      // registered via `ws.addSession` — that would navigate to the Agents view
      // and surface the terminal there. The connection lives only in this hub; the
      // session is killed when the tab closes (see `closeSshTerminal`).
      database.addSshTab({ connId: c.id, sessionId: session.id, name: c.name, kind: c.kind });
    } catch (e) {
      toastError('Couldn’t open', e);
    } finally {
      opening[c.id] = false;
    }
  }

  /** Close an SSH/custom terminal tab and kill its underlying session so it
   *  doesn't linger on the daemon (it isn't tracked in the Agents list). */
  async function closeSshTerminal(connId: string): Promise<void> {
    const tab = database.sshTabs.find((t) => t.connId === connId);
    database.closeSshTab(connId);
    if (tab) {
      try {
        await api.del(`/sessions/${tab.sessionId}`);
      } catch {
        // best-effort: the tab is gone from the workbench regardless
      }
    }
  }
  async function deleteConnection(c: Connection): Promise<void> {
    if (
      !(await confirmer.ask(`Delete connection “${c.name}”? Its Keychain secret is removed too.`, {
        title: 'Delete connection',
      }))
    )
      return;
    try {
      await api.del(`/connections/${c.id}`);
      if (database.openConnIds.includes(c.id)) database.closeConnection(c.id);
      await database.loadConnections();
    } catch (e) {
      toastError('Couldn’t delete', e);
    }
  }

  // Load connections + workspace-scoped saved/dashboards when the workspace changes.
  // The workbench itself is GLOBAL — open connection tabs survive workspace
  // switches. Restore the persisted workbench (open tabs + focus) ONLY after
  // loadConnections resolves — the restore filters against the loaded list.
  $effect(() => {
    if (ws.currentId) {
      void (async () => {
        await database.loadConnections();
        connsSettled = true;
        await database.restoreWorkbench();
        // `#/database/<connId>` (a pop-out window, a link) opens that tab.
        const deep = router.module === 'database' ? router.parts[1] : undefined;
        if (deep && database.connections.some((c) => c.id === deep)) await database.openConnection(deep);
      })();
      void loadSections();
      void brokers.load(ws.currentId); // clusters render in the same tree
      void database.loadSavedQueries();
      void database.loadDashboards();
    }
  });

  /** The first connection load has finished (so "nothing listed" is real, not
   *  "not fetched yet") — gates the empty-hub layout below. */
  let connsSettled = $state(false);
  /** Nothing to list at all: the list pane is hidden and one page-level
   *  EmptyState owns the "New connection" CTA (no duplicate link in the list). */
  const hubEmpty = $derived(
    connsSettled &&
      !database.connectionsLoading &&
      !database.connectionsError &&
      database.connections.length === 0 &&
      database.otherConnections.length === 0 &&
      brokers.clusters.length === 0 &&
      sections.length === 0,
  );

  /** Tooltip for the connection health chip: "Connected · <version> · <ms> ms". */
  function healthTitle(st: { serverVersion?: string; latencyMs?: number }): string {
    const parts = ['Connected'];
    if (st.serverVersion) parts.push(st.serverVersion);
    if (st.latencyMs != null) parts.push(`${st.latencyMs} ms`);
    return parts.join(' · ');
  }

  const mainTabs: { id: DbMainTab; label: string; icon: IconName; show: () => boolean }[] = [
    { id: 'query', label: 'Query', icon: 'terminal', show: () => true },
    { id: 'builder', label: 'Builder', icon: 'layers', show: () => database.supportsBuilder },
    { id: 'structure', label: 'Structure', icon: 'columns', show: () => true },
    // ERD is table/collection-oriented; Redis (keys, no table model) is excluded.
    { id: 'diagram', label: 'Diagram', icon: 'shapes', show: () => database.capabilities?.engine !== 'redis' },
    { id: 'dashboards', label: 'Dashboards', icon: 'chart', show: () => true },
  ];
  const visibleTabs = $derived(mainTabs.filter((t) => t.show()));
  // ── DB Assistant split (resizable, persisted) ────────────────────────────────
  // When open, the DB Assistant panel sits BESIDE the editor/results, separated by
  // a draggable divider so the user can enlarge the agent's shell. Mirrors the
  // query-editor's own resizable-pane idiom (pointer drag + localStorage px).
  let assistW = $state(loadAssistW());
  function loadAssistW(): number {
    if (typeof localStorage === 'undefined') return 460;
    const v = Number(localStorage.getItem('db.assistW'));
    return Number.isFinite(v) && v > 280 ? v : 460;
  }
  function persistAssistW(): void {
    try {
      localStorage.setItem('db.assistW', String(Math.round(assistW)));
    } catch {
      /* storage unavailable — non-fatal */
    }
  }
  const assistMaxW = (): number => Math.max(320, (typeof window !== 'undefined' ? window.innerWidth : 1280) - 360);
  const pxText = (v: number): string => `${Math.round(v)} pixels wide`;
  function setAssistW(w: number): void {
    assistW = w;
    persistAssistW();
  }
  function startAssistResize(e: PointerEvent): void {
    e.preventDefault();
    const startX = e.clientX;
    const startW = assistW;
    const maxW = assistMaxW();
    const onMove = (ev: PointerEvent): void => {
      // The panel is pinned to the right edge, so dragging LEFT widens it.
      assistW = Math.max(300, Math.min(maxW, startW + (startX - ev.clientX)));
    };
    const onUp = (): void => {
      persistAssistW();
      window.removeEventListener('pointermove', onMove);
      window.removeEventListener('pointerup', onUp);
    };
    window.addEventListener('pointermove', onMove);
    window.addEventListener('pointerup', onUp);
  }

  // ── Connection sidebar width (resizable, persisted) ───────────────────────────
  // The tablet/desktop sidebar is drag-resizable so long, deeply-nested connection
  // names ("DB MySQL - Platform Aggregates Prod") get the horizontal room they need;
  // the chosen width survives reloads. Mirrors the assist-pane idiom above. (On
  // phones the sidebar is a full-width band — the width binding is skipped there.)
  // Documented exception to LIST_PANE: the default is 300 (not 280) and the max
  // tracks the window (up to 640), because this one pane also hosts the schema
  // tree, Saved and History — deeper, wider content than a plain list.
  const SIDE_DEFAULT = 300;
  let sideW = $state(loadPaneWidth('db.sideW', SIDE_DEFAULT, LIST_PANE.min, 640));
  // Leave room for the editor/results area; cap so the sidebar can't eat the page.
  const sideMaxW = (): number => Math.min(640, Math.max(360, (typeof window !== 'undefined' ? window.innerWidth : 1280) - 420));
  // Open connections as top-level tabs (Workbench-style), resolved to their
  // Connection records for name + engine glyph.
  const openConns = $derived(
    database.openConnIds
      .map((id) => database.connections.find((c) => c.id === id))
      .filter((c): c is NonNullable<typeof c> => c != null),
  );

  // The unified workbench renders three kinds of tab: DB connections (openConns),
  // Kafka clusters (brokers store), and ssh/custom terminals (database.sshTabs).
  const hasAnyTab = $derived(
    openConns.length > 0 || brokers.openClusters.length > 0 || database.sshTabs.length > 0,
  );
  // No auto-open on arrival: opening a connection connects to the server (it
  // may be prod), so the empty main pane shows the collection summary instead.
  // "6 connections · 2 prod · 1 Kafka cluster" — what the empty main pane says.
  const hubSummary = $derived.by(() => {
    const dbs = database.connections.length + database.otherConnections.length;
    const prod = [...database.connections, ...database.otherConnections].filter((c) => c.environment === 'prod').length;
    const k = brokers.clusters.length;
    const parts = [`${plural(dbs, 'connection')}`];
    if (prod > 0) parts.push(`${prod} prod`);
    if (k > 0) parts.push(plural(k, 'Kafka cluster'));
    return `${parts.join(' · ')}.`;
  });
  // An empty workbench (fresh start, after a restore that found nothing, or the
  // last tab closed) always surfaces the picker — a schema/saved/history side
  // tab with no connection behind it is a dead end.
  $effect(() => {
    if (!hasAnyTab && database.sideTab !== 'connections') database.setSideTab('connections');
  });
  // The cluster / ssh session backing the active non-DB pane (null when a DB tab
  // is focused, or when the backing tab was closed out from under the pane).
  const activeCluster = $derived(
    database.activePane?.kind === 'kafka'
      ? (brokers.openClusters.find((c) => c.id === database.activePane!.id) ?? null)
      : null,
  );
  const activeSsh = $derived(
    database.activePane?.kind === 'ssh'
      ? (database.sshTabs.find((t) => t.connId === database.activePane!.id) ?? null)
      : null,
  );
  // Roving tabindex over the connection strip: the selected tab is the one Tab
  // stop; with none selected, the first tab is, so the strip stays reachable.
  const connTabActive = $derived(
    activeCluster != null ||
      activeSsh != null ||
      (database.activePane === null && openConns.some((c) => c.id === database.selectedConnId)),
  );
  const firstConnTab = $derived(
    openConns[0] ? `db:${openConns[0].id}`
      : brokers.openClusters[0] ? `kafka:${brokers.openClusters[0].id}`
      : database.sshTabs[0] ? `ssh:${database.sshTabs[0].connId}`
      : '',
  );
  const connTabStop = (on: boolean, key: string): number => (on || (!connTabActive && firstConnTab === key) ? 0 : -1);

  // The open-connections strip scrolls sideways with its scrollbar hidden, so a
  // tab opened (or focused) past the right edge would be selected but invisible —
  // with nothing hinting the strip scrolls. Bring the active tab into view
  // whenever the selection or the set of open tabs changes.
  let connTabsEl = $state<HTMLElement | null>(null);
  $effect(() => {
    void database.selectedConnId;
    void database.activePane;
    void openConns.length;
    void brokers.openClusters.length;
    void database.sshTabs.length;
    const strip = connTabsEl;
    if (!strip) return;
    requestAnimationFrame(() =>
      strip.querySelector<HTMLElement>('.conn-tab.active')?.scrollIntoView({ block: 'nearest', inline: 'nearest' }),
    );
  });

  // Close a Kafka cluster tab; follow the brokers store's neighbour reselection
  // (or drop back to the DB workbench when no clusters remain open).
  function closeKafkaTab(id: string): void {
    const wasActive = database.activePane?.kind === 'kafka' && database.activePane.id === id;
    brokers.close(id);
    if (wasActive) {
      if (brokers.selectedId) database.focusKafka(brokers.selectedId);
      else database.activePane = null;
    }
  }

  function fmtAgo(iso: string): string {
    const ms = Date.now() - new Date(iso).getTime();
    const s = Math.floor(ms / 1000);
    if (s < 60) return `${s}s`;
    if (s < 3600) return `${Math.floor(s / 60)}m`;
    if (s < 86400) return `${Math.floor(s / 3600)}h`;
    return `${Math.floor(s / 86400)}d`;
  }

  // ── Environment guardrail (danger styling) ─────────────────────────────────
  // Prod connections are dangerous; read-only are locked. Both get a badge, and
  // the selected one draws a red rail down the main area as a constant reminder.
  // Structural pick so broker clusters (same guard fields) share the badges.
  type EnvGuarded = Pick<Connection, 'environment' | 'read_only'>;
  const isProdConn = (c: EnvGuarded): boolean => c.environment === 'prod';
  const isGuardedConn = (c: EnvGuarded): boolean => c.environment === 'prod' || c.read_only;
  // Whether a row gets the shared <EnvBadge> — the same rule as Brokers /
  // Kubernetes / AWS: prod and staging are badged, and so is read-only (RO);
  // dev (the default) stays unbadged. Uses `envTone` so env aliases agree with
  // the badge itself.
  function envBadge(c: EnvGuarded): boolean {
    return envTone(c.environment).key !== 'dev' || c.read_only;
  }
</script>

<div class="db-root">
<!-- One header for the Connections hub. "New connection" is the page's primary
     (it used to be a bare "+" in the side-tab strip); while there is nothing
     to open, the empty state owns that CTA instead. Phone keeps its accordion
     head buttons, so the header stays action-free there. -->
<PageHeader title="Connections">
  <!-- The one list-collapse control (same as Canvas / Skills Lab): the header
       `sidebar` icon. ⌘B does the same; nothing in the pane duplicates it. -->
  {#snippet leading()}
    {#if !viewport.isPhone && !hubEmpty}
      <button
        class="icon-btn"
        onclick={() => (database.sidebarCollapsed ? void showSchemaSidebar() : database.toggleSidebar())}
        aria-label={database.sidebarCollapsed ? 'Show schema sidebar' : 'Hide sidebar'}
        title={database.sidebarCollapsed ? 'Show sidebar (⌘B)' : 'Hide sidebar (⌘B)'}
        aria-expanded={!database.sidebarCollapsed}
        aria-controls="db-side"
      >
        <Icon name="sidebar" size={16} />
      </button>
    {/if}
  {/snippet}
  {#snippet actions()}
    {#if !viewport.isPhone}
      {#if !hubEmpty}
      <button class="btn small ghost" disabled={!auth.isRoot} onclick={() => (connImportOpen = true)} title={auth.isRoot ? 'Import connections from MySQL Workbench, DBeaver, DataGrip or NoSQLBooster' : 'Only the owner can import connections'}>
        <Icon name="arrowDown" size={12} /> Import
      </button>
      {/if}
      {#if database.connections.length > 0 || database.otherConnections.length > 0 || brokers.clusters.length > 0}
        <button class="btn small primary" disabled={!auth.isRoot} onclick={newConnectionFromStrip} title={auth.isRoot ? 'New connection (SSH, database or custom CLI)' : 'Only the owner can create connections'}>
          <Icon name="plus" size={12} /> New connection
        </button>
      {/if}
    {/if}
  {/snippet}
</PageHeader>
<PageBody fill padded={false}>
<div class="db-page">
  <aside
    id="db-side"
    class="db-side"
    class:collapsed={!viewport.isPhone && (database.sidebarCollapsed || hubEmpty)}
    style={viewport.isPhone || database.sidebarCollapsed ? '' : `width:${sideW}px`}
  >
    {#if viewport.isPhone}
      <!-- PHONE: collapsible accordions (one section at a time), unchanged layout
           except the connection list now carries a filter box. -->
      <div class="conn-head acc-head">
        <button class="acc-toggle" onclick={() => (connOpen = !connOpen)} aria-expanded={connOpen}>
          <Icon name={connOpen ? 'chevronDown' : 'chevronRight'} size={14} />
          <span class="conn-head-title">Connections</span>
          {#if database.connections.length > 0}<span class="acc-count">{database.connections.length}</span>{/if}
        </button>
        <div class="head-btns">
          <button class="icon-btn" onclick={() => createSection(null)} aria-label="New section" title="New section"><Icon name="folder" size={13} /></button>
          <button class="icon-btn" disabled={!auth.isRoot} onclick={newConnection} aria-label="New connection" title={auth.isRoot ? 'New connection' : 'Only the owner can create connections'}><Icon name="plus" size={13} /></button>
          <button class="icon-btn" disabled={!auth.isRoot} onclick={() => (connImportOpen = true)} aria-label="Import connections" title={auth.isRoot ? 'Import connections from MySQL Workbench, DBeaver, DataGrip or NoSQLBooster' : 'Only the owner can import connections'}><Icon name="arrowDown" size={13} /></button>
        </div>
      </div>
      <div class="conn-list" class:acc-collapsed={!connOpen}>
        {@render connListBody()}
      </div>

      {#if database.selectedConnId}
        <!-- Phone: a tappable accordion header gates the whole schema panel. -->
        <div class="conn-head acc-head">
          <button class="acc-toggle" onclick={() => (schemaOpen = !schemaOpen)} aria-expanded={schemaOpen}>
            <Icon name={schemaOpen ? 'chevronDown' : 'chevronRight'} size={14} />
            <span class="conn-head-title">Schema &amp; saved</span>
          </button>
          {#if schemaOpen && database.sideTab === 'schema'}
            <div class="head-btns">
              <button class="icon-btn" onclick={() => database.refreshSchema()} title="Refresh schema" aria-label="Refresh schema"><Icon name="refresh" size={13} /></button>
            </div>
          {/if}
        </div>
        <div class="side-switch" class:acc-collapsed={!schemaOpen} role="tablist" aria-label="Sidebar view" tabindex="-1" onkeydown={onTabKey}>
          <button class="ss" class:active={database.sideTab === 'schema' || database.sideTab === 'connections'} role="tab" aria-selected={database.sideTab === 'schema' || database.sideTab === 'connections'} tabindex={database.sideTab === 'schema' || database.sideTab === 'connections' ? 0 : -1} onclick={() => database.setSideTab('schema')}>Schema</button>
          <button class="ss" class:active={database.sideTab === 'saved'} role="tab" aria-selected={database.sideTab === 'saved'} tabindex={database.sideTab === 'saved' ? 0 : -1} onclick={() => database.setSideTab('saved')}>Saved</button>
          <button class="ss" class:active={database.sideTab === 'history'} role="tab" aria-selected={database.sideTab === 'history'} tabindex={database.sideTab === 'history' ? 0 : -1} onclick={() => database.setSideTab('history')}>History</button>
        </div>
        <div class="side-body" class:acc-collapsed={!schemaOpen}>
          {@render schemaSideBody()}
        </div>
      {/if}
    {:else}
      <!-- TABLET / DESKTOP: one tab strip. "Connections" is the picker tab, so
           the list takes the full sidebar height instead of a capped section. -->
      <div class="side-switch">
        <!-- The tabs get their own tablist: the strip also carries plain
             buttons (Refresh), which a tablist may not own. -->
        <div class="ss-tabs" role="tablist" aria-label="Sidebar view" tabindex="-1" onkeydown={onTabKey}>
          <button class="ss" class:active={database.sideTab === 'connections'} role="tab" aria-selected={database.sideTab === 'connections'} tabindex={database.sideTab === 'connections' ? 0 : -1} onclick={() => database.setSideTab('connections')}>Connections</button>
          <button class="ss" class:active={database.sideTab === 'schema'} role="tab" aria-selected={database.sideTab === 'schema'} tabindex={database.sideTab === 'schema' ? 0 : -1} onclick={() => database.setSideTab('schema')}>Schema</button>
          <button class="ss" class:active={database.sideTab === 'saved'} role="tab" aria-selected={database.sideTab === 'saved'} tabindex={database.sideTab === 'saved' ? 0 : -1} onclick={() => database.setSideTab('saved')}>Saved</button>
          <button class="ss" class:active={database.sideTab === 'history'} role="tab" aria-selected={database.sideTab === 'history'} tabindex={database.sideTab === 'history' ? 0 : -1} onclick={() => database.setSideTab('history')}>History</button>
        </div>
        <span class="grow"></span>
        {#if database.sideTab === 'schema' && database.selectedConnId}
          <button class="icon-btn" onclick={() => database.refreshSchema()} title="Refresh schema" aria-label="Refresh schema"><Icon name="refresh" size={12} /></button>
        {/if}
      </div>
      <div class="side-body">
        {#if database.sideTab === 'connections'}
          <div class="conn-list">{@render connListBody()}</div>
        {:else if database.selectedConnId}
          {@render schemaSideBody()}
        {:else}
          <!-- Not a dead end: hand the user the two ways forward. -->
          <div class="side-empty">
            <EmptyState icon="file" title="No connection open" body="Open a connection to browse its schema." />
            <div class="side-empty-actions">
              <button class="btn small" onclick={() => database.setSideTab('connections')}>Browse connections</button>
              <button class="btn small ghost" disabled={!auth.isRoot} onclick={newConnection} title={auth.isRoot ? undefined : 'Only the owner can create connections'}>New connection</button>
            </div>
          </div>
        {/if}
      </div>
    {/if}
  </aside>

  {#if !viewport.isPhone && !database.sidebarCollapsed && !hubEmpty}
    <PaneDivider bind:width={sideW} storageKey="db.sideW" label="Resize connections sidebar" max={sideMaxW()} defaultWidth={SIDE_DEFAULT} />
  {/if}

  <div class="db-main" class:danger-rail={database.isProd} class:guard-rail={database.isGuarded && !database.isProd}>
    {#if !hasAnyTab}
      <!-- Always actionable: create the first connection, or surface the picker
           (which may be hidden behind a collapsed rail / another side tab). -->
      <!-- One CTA, and only when it does something: create the first
           connection, or reveal a picker that is hidden (collapsed rail /
           another side tab). With the list already on screen there is no
           button — "Show connections" next to the visible list was noise. -->
      {#if database.connections.length === 0 && database.otherConnections.length === 0 && brokers.clusters.length === 0}
        <EmptyState
          variant={viewport.isPhone ? 'panel' : 'page'}
          icon="plug"
          title="No connections yet"
          body="Add a database (MySQL, PostgreSQL, Redis, MongoDB, ClickHouse), an SSH host or a custom CLI, then open it here to query, browse its schema or get a terminal."
          actionLabel={auth.isRoot ? 'New connection' : undefined}
          actionIcon="plus"
          onaction={auth.isRoot ? newConnection : undefined}
        >
          {#if auth.isRoot}
            <div class="empty-alt">
              <button class="btn ghost small" onclick={newCluster}><Icon name={engineGlyph('kafka')} size={12} /> Add a Kafka cluster</button>
              <button class="btn ghost small" onclick={() => (connImportOpen = true)}><Icon name="arrowDown" size={12} /> Import from another tool…</button>
            </div>
          {:else}
            <p class="empty-note">Only the owner can add connections.</p>
          {/if}
        </EmptyState>
      {:else}
        <!-- One CTA, and only when it does something: reveal a picker that is
             hidden (collapsed rail / another side tab). With the list already
             on screen there is no button. -->
        <EmptyState
          variant={viewport.isPhone ? 'panel' : 'page'}
          icon="db"
          title="Open a connection"
          body={`${hubSummary} Choose one ${viewport.isPhone ? 'above' : 'on the left'} to open it here.`}
          actionLabel={database.sidebarCollapsed || database.sideTab !== 'connections' ? 'Show connections' : undefined}
          onaction={showConnections}
        />
      {/if}
    {:else}
      <!-- Unified tab strip: DB connections, Kafka clusters, and SSH/custom terminals -->
      <!-- Each tab is the main <button role="tab">; the status glyph, the ⋯ menu
           (the right-click menu, reachable without a mouse) and the close button
           are its siblings inside a presentational wrapper. -->
      <div class="conn-tabs" role="tablist" aria-label="Open connections" tabindex="-1" onkeydown={onTabKey} bind:this={connTabsEl}>
        {#each openConns as c (c.id)}
          {@const st = database.connStatus.get(c.id)}
          {@const on = database.activePane === null && database.selectedConnId === c.id}
          <div use:rowMenu class="conn-tab" class:active={on} class:prod={isProdConn(c)} class:guarded={isGuardedConn(c) && !isProdConn(c)} role="presentation" oncontextmenu={(e) => { e.preventDefault(); connMenu(e, c); }}>
            <button class="conn-tab-main" role="tab" aria-selected={on} tabindex={connTabStop(on, `db:${c.id}`)} onclick={() => database.openConnection(c.id)} title="{c.name} — right-click to open beside agents">
              <span class="conn-tab-glyph {c.kind}"><Icon name={engineGlyph(c.kind)} size={12} /></span>
              {#if sectionLeaf(c)}<span class="conn-tab-path mono" title="Folder: {sectionPath(c)}">{sectionLeaf(c)}</span>{/if}
              <span class="conn-tab-name ellipsis">{c.name}</span>
              {#if envBadge(c)}<EnvBadge env={c.environment} readOnly={c.read_only} />{/if}
            </button>
            {#if st?.phase === 'connecting'}
              <span class="conn-tab-spin spinner" style="--spinner-size: 10px" title={database.isWarming(c.id) ? 'Connecting in the background… (right-click to stop)' : 'Connecting…'} data-testid="conn-tab-connecting"></span>
            {:else if st?.phase === 'error'}
              <span class="conn-tab-dot" title={st.error}></span>
            {:else if st?.phase === 'idle'}
              <span class="conn-tab-idle" title="Not connected yet — opens on click" aria-label="Not connected yet" data-testid="conn-tab-idle"></span>
            {/if}
            <button class="conn-tab-more" onclick={(e) => connMenu(e, c)} aria-haspopup="menu" aria-label="More actions for {c.name}" title="More actions for {c.name}">
              <Icon name="more" size={12} />
            </button>
            <button
              class="conn-tab-close"
              onclick={(e) => {
                e.stopPropagation();
                database.closeConnection(c.id);
              }}
              aria-label="Close connection tab"
              title="Close connection tab"
            >
              <Icon name="x" size={12} />
            </button>
          </div>
        {/each}
        {#each brokers.openClusters as cl (cl.id)}
          {@const on = database.activePane?.kind === 'kafka' && database.activePane.id === cl.id}
          <div use:rowMenu class="conn-tab" class:active={on} class:prod={isProdConn(cl)} role="presentation" oncontextmenu={(e) => { e.preventDefault(); clusterMenu(e, cl); }}>
            <button class="conn-tab-main" role="tab" aria-selected={on} tabindex={connTabStop(on, `kafka:${cl.id}`)} onclick={() => openCluster(cl)} title={cl.name}>
              <span class="conn-tab-glyph kafka"><Icon name={engineGlyph('kafka')} size={12} /></span>
              <span class="conn-tab-name ellipsis">{cl.name}</span>
              {#if envBadge(cl)}<EnvBadge env={cl.environment} readOnly={cl.read_only} />{/if}
            </button>
            <button class="conn-tab-more" onclick={(e) => clusterMenu(e, cl)} aria-haspopup="menu" aria-label="More actions for {cl.name}" title="More actions for {cl.name}">
              <Icon name="more" size={12} />
            </button>
            <button class="conn-tab-close" onclick={(e) => { e.stopPropagation(); closeKafkaTab(cl.id); }} aria-label="Close cluster tab" title="Close cluster tab">
              <Icon name="x" size={12} />
            </button>
          </div>
        {/each}
        {#each database.sshTabs as s (s.connId)}
          {@const on = database.activePane?.kind === 'ssh' && database.activePane.id === s.connId}
          <div class="conn-tab" class:active={on} role="presentation">
            <button class="conn-tab-main" role="tab" aria-selected={on} tabindex={connTabStop(on, `ssh:${s.connId}`)} onclick={() => database.focusSsh(s.connId)} title={s.name}>
              <span class="conn-tab-glyph {s.kind}"><Icon name={engineGlyph(s.kind)} size={12} /></span>
              <span class="conn-tab-name ellipsis">{s.name}</span>
            </button>
            <button class="conn-tab-close" onclick={(e) => { e.stopPropagation(); void closeSshTerminal(s.connId); }} aria-label="Close terminal tab" title="Close terminal tab">
              <Icon name="x" size={12} />
            </button>
          </div>
        {/each}
      </div>

      {#if database.activePane?.kind === 'kafka'}
        {#if activeCluster}
          <LazyMount lazy={ClusterViewerLazy} what="the Kafka cluster viewer" props={{ cluster: activeCluster, onEdit: editCluster, onRemove: (c: BrokerCluster) => void deleteCluster(c) }} />
        {:else}
          <EmptyState icon="box" title="Cluster closed" body="This Kafka cluster tab is no longer open." />
        {/if}
      {:else if database.activePane?.kind === 'ssh'}
        {#if activeSsh}
          <div class="term-pane">
            {#key activeSsh.sessionId}
              <LazyMount lazy={TerminalLazy} what="the terminal" props={{ sessionId: activeSsh.sessionId, scrollback: PRIMARY_SCROLLBACK }} />
            {/key}
          </div>
        {:else}
          <EmptyState icon="terminal" title="Session closed" body="This terminal tab is no longer open." />
        {/if}
      {:else if database.selectedConnId}
      {#if database.isGuarded}
        <div class="guard-banner" class:prod={database.isProd}>
          <Icon name={database.isProd ? 'zap' : 'key'} size={13} />
          <span>
            {#if database.isProd}
              Production connection — schema changes require an approved change. Other writes require permission and confirmation.
            {:else}
              Read-only connection — native privileges and access rules limit execution.
            {/if}
          </span>
        </div>
      {/if}

      <div class="main-tabs" class:many={visibleTabs.length > 3}>
        <!-- The workbench views: a segmented control (selection = surface
             lift), ←/→ move between them like any tablist. -->
        <div class="segmented view-switch" role="tablist" aria-label="Workbench view" tabindex="-1" onkeydown={onTabKey}>
          {#each visibleTabs as t (t.id)}
            <button
              class="mt"
              class:active={database.mainTab === t.id}
              role="tab"
              aria-selected={database.mainTab === t.id}
              tabindex={database.mainTab === t.id ? 0 : -1}
              title={t.label}
              onclick={() => database.setMainTab(t.id)}
            >
              <Icon name={t.icon} size={12} />{t.label}
            </button>
          {/each}
        </div>
        <span class="grow"></span>
        <div class="conn-status">
          {#if database.capabilities}
            <span class="cap-chip mono" title="Engine">{database.capabilities.engine}</span>
          {/if}
          {#if database.activeConnStatus?.phase === 'connecting'}
            <span class="conn-state" title="Connecting…"><span class="conn-tab-spin spinner" style="--spinner-size: 12px" aria-hidden="true"></span><span class="lbl">Connecting…</span></span>
          {:else if database.activeConnStatus?.phase === 'error'}
            <span class="conn-state err" title={database.activeConnStatus.error}>Disconnected</span>
          {:else if database.activeConnStatus?.phase === 'ready'}
            <span class="conn-state ok" title={healthTitle(database.activeConnStatus)}>
              <span class="health-dot"></span>
              {#if database.activeConnStatus.serverVersion}
                <span class="health-ver mono ellipsis">{database.activeConnStatus.serverVersion}</span>
              {/if}
              {#if database.activeConnStatus.latencyMs != null}
                <span class="health-lat">{database.activeConnStatus.latencyMs} ms</span>
              {/if}
            </span>
          {/if}
          {#if ['mysql','postgres'].includes(database.connections.find(c=>c.id===database.selectedConnId)?.kind ?? '')}
            <button class="btn small ghost" onclick={()=>changesOpen=true} title="Reviewed schema changes for this connection" aria-label="Schema changes">
              <Icon name="branch" size={12} /><span class="lbl">Changes</span>
            </button>
          {/if}
          <button class="btn small ghost" onclick={() => database.testConnection()} disabled={database.testing} title="Test this connection" aria-label={database.testing ? 'Testing connection' : 'Test connection'}>
            <Icon name="plug" size={12} /><span class="lbl">{database.testing ? 'Testing…' : 'Test'}</span>
          </button>
          {#if database.testResult}
            <span class="test-dot" class:ok={database.testResult.ok} title={database.testResult.message}></span>
          {/if}
        </div>
      </div>

      <!-- The active view (editor/results/…) and, when open, the DB Assistant
           agent panel side-by-side, separated by a draggable, persisted divider. -->
      <div class="main-split" class:assist-open={database.assistOpen}>
        <div class="main-body">
          {#key database.accessRevision}
          <ConnectionComparison enabled={database.mainTab === 'query'}>
          {#if database.mainTab === 'query'}
            <QueryEditor />
          {:else if database.mainTab === 'builder'}
            <LazyMount lazy={QueryBuilderLazy} what="the query builder" />
          {:else if database.mainTab === 'structure'}
            <LazyMount lazy={StructureLazy} what="the structure view" />
          {:else if database.mainTab === 'diagram'}
            <LazyMount lazy={DiagramLazy} what="the diagram" />
          {:else}
            <LazyMount lazy={DashboardsLazy} what="dashboards" />
          {/if}
          </ConnectionComparison>
          {/key}
        </div>
        {#if database.assistOpen}
          <!-- A focusable separator is the ARIA window-splitter widget (paneResizer sets its tabIndex and value, and adds ←/→, Home/End). -->
          <div
            class="assist-divider"
            role="separator"
            aria-orientation="vertical"
            aria-label="Resize assistant"
            title="Drag or use ←/→ to resize the assistant"
            onpointerdown={startAssistResize}
            use:paneResizer={{ value: assistW, min: 300, max: assistMaxW(), invert: true, onChange: setAssistW, text: pxText }}
          ></div>
          <aside class="assist-pane" style="width:{assistW}px">
            <LazyMount lazy={AssistantLazy} what="the assistant" variant="panel" />
          </aside>
        {/if}
      </div>
      {:else}
        <EmptyState
          icon="db"
          title="Pick a connection"
          body={`${hubSummary} Choose one on the left to open it here.`}
          actionLabel="Show connections"
          onaction={showConnections}
        />
      {/if}
    {/if}
  </div>
</div>
</PageBody>
</div>

{#snippet sectionNode(node: TreeNode, depth: number)}
  {@const isOpen = !collapsed[node.sec.id]}
  {#if nodeVisible(node)}
    <!-- The whole header toggles (the caret button stays the keyboard/AT
         control); clicks on its own buttons — caret, row actions — don't. -->
    <!-- The caret button is the keyboard/AT toggle; a click anywhere on the head is a mouse shortcut, so the head itself is presentational. -->
    <div
      role="presentation"
      class="sec-head"
      onclick={(e) => {
        if (!(e.target as Element).closest('button')) toggleCollapse(node.sec.id);
      }}
      class:drop-target={(draggedSectionId && draggedSectionId !== node.sec.id) ||
        draggedConnId ||
        draggedClusterId}
      style="padding-inline-start: {depth * 14 + 2}px"
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
      <button
        class="caret"
        onclick={() => toggleCollapse(node.sec.id)}
        aria-label={isOpen ? `Collapse ${node.sec.name}` : `Expand ${node.sec.name}`} title={isOpen ? `Collapse ${node.sec.name}` : `Expand ${node.sec.name}`}
        aria-expanded={isOpen}
      >
        <Icon name={isOpen ? 'chevronDown' : 'chevronRight'} size={12} />
      </button>
      <Icon name="folder" size={12} />
      <span class="sec-name grow ellipsis">{node.sec.name}</span>
      {#if nodeCount(node) > 0}<span class="count">{nodeCount(node)}</span>{/if}
      <div class="sec-actions">
        <button class="icon-btn" title="Add sub-section" aria-label="Add sub-section" onclick={() => createSection(node.sec.id)}>
          <Icon name="plus" size={12} />
        </button>
        <button class="icon-btn" title="Rename section" aria-label="Rename section" onclick={() => renameSection(node.sec)}>
          <Icon name="edit" size={12} />
        </button>
        <button class="icon-btn" title="Delete section" aria-label="Delete section" onclick={() => deleteSection(node.sec)}>
          <Icon name="trash" size={12} />
        </button>
      </div>
    </div>
    {#if isOpen}
      {#each node.items as c (c.id)}
        {@render connRow(c, depth + 1)}
      {/each}
      {#each node.clusters as cl (cl.id)}
        {@render clusterRow(cl, depth + 1)}
      {/each}
      {#each node.children as child (child.sec.id)}
        {@render sectionNode(child, depth + 1)}
      {/each}
    {/if}
  {/if}
{/snippet}

{#snippet connRow(c: Connection, depth: number)}
  {@const isDb = database.connections.some((x) => x.id === c.id)}
  <div use:rowMenu
    role="group"
    aria-label={c.name}
    class="conn-row"
    class:active={database.selectedConnId === c.id}
    class:open={database.openConnIds.includes(c.id)}
    class:dragging={draggedConnId === c.id}
    style="padding-inline-start: {depth * 14}px"
    draggable="true"
    ondragstart={(e) => {
      draggedConnId = c.id;
      e.stopPropagation();
    }}
    ondragend={() => (draggedConnId = null)}
    oncontextmenu={(e) => { e.preventDefault(); connMenu(e, c); }}
  >
    <button
      class="conn-item"
      onclick={() => (isDb ? database.openConnection(c.id) : void openTerminal(c))}
      title="{c.name} · {c.kind}{isProdConn(c) ? ' · PRODUCTION' : c.read_only ? ' · read-only' : ''} — {isDb ? 'opens the workbench; right-click for more' : 'opens a terminal session; right-click for more'}"
    >
      <span class="conn-glyph {c.kind}"><Icon name={engineGlyph(c.kind)} size={12} /></span>
      <span class="conn-name">{c.name}</span>
      <Badge variant="outline" label={c.kind} />
      {#if opening[c.id]}<span class="spinner" style="--spinner-size: 10px" role="status" aria-label="Opening…" title="Opening…"></span>{/if}
      {#if envBadge(c)}<EnvBadge env={c.environment} readOnly={c.read_only} />{/if}
    </button>
    <div class="conn-actions">
      <!-- One ⋯ per row (same menu as right-click), like every other list in
           Infrastructure — not a row of per-action icons. -->
      <button class="icon-btn" aria-label={`Actions for ${c.name}`} title="Actions" onclick={(e) => connMenu(e, c)}>
        <Icon name="more" size={14} />
      </button>
    </div>
  </div>
{/snippet}

{#snippet clusterRow(cl: BrokerCluster, depth: number)}
  <div use:rowMenu
    role="group"
    aria-label={cl.name}
    class="conn-row"
    class:dragging={draggedClusterId === cl.id}
    style="padding-inline-start: {depth * 14}px"
    draggable="true"
    ondragstart={(e) => {
      draggedClusterId = cl.id;
      e.stopPropagation();
    }}
    ondragend={() => (draggedClusterId = null)}
    oncontextmenu={(e) => { e.preventDefault(); clusterMenu(e, cl); }}
  >
    <button
      class="conn-item"
      onclick={() => openCluster(cl)}
      title="{cl.name} · kafka{isProdConn(cl) ? ' · PRODUCTION' : cl.read_only ? ' · read-only' : ''} — opens here"
    >
      <span class="conn-glyph kafka"><Icon name={engineGlyph('kafka')} size={12} /></span>
      <span class="conn-name">{cl.name}</span>
      <Badge variant="outline" label="kafka" />
      {#if envBadge(cl)}<EnvBadge env={cl.environment} readOnly={cl.read_only} />{/if}
    </button>
    <div class="conn-actions">
      <button class="icon-btn" aria-label={`Actions for ${cl.name}`} title="Actions" onclick={(e) => clusterMenu(e, cl)}>
        <Icon name="more" size={14} />
      </button>
    </div>
  </div>
{/snippet}

{#snippet connSearchBox()}
  <div class="tree-search">
    <Icon name="search" size={12} />
    <input dir="ltr"
      class="tree-search-input"
      type="text"
      bind:value={connFilter}
      placeholder="Filter connections…"
      spellcheck="false"
      aria-label="Filter connections"
    />
    {#if connFilter}
      <button class="tree-search-clear" onclick={() => (connFilter = '')} aria-label="Clear filter" title="Clear filter"><Icon name="x" size={10} /></button>
    {/if}
    {#if !viewport.isPhone}
      <!-- New section / connection live here on tablet/desktop (the phone keeps
           them in the accordion header), so the tab strip never overflows. -->
      <button class="icon-btn" onclick={() => createSection(null)} aria-label="New section" title="New section"><Icon name="folder" size={12} /></button>
      <button class="icon-btn" disabled={!auth.isRoot} onclick={newConnection} aria-label="New connection" title={auth.isRoot ? 'New connection (SSH, database or custom CLI)' : 'Only the owner can create connections'}><Icon name="plus" size={12} /></button>
      <button class="icon-btn" onclick={newCluster} aria-label="New Kafka cluster" title="New Kafka cluster"><Icon name="split" size={12} /></button>
      <button class="icon-btn" disabled={!auth.isRoot} onclick={() => (connImportOpen = true)} aria-label="Import connections" title={auth.isRoot ? 'Import connections from MySQL Workbench, DBeaver, DataGrip or NoSQLBooster' : 'Only the owner can import connections'}><Icon name="arrowDown" size={12} /></button>
    {/if}
  </div>
  <!-- Type-filter chips: one tree, narrowed by connection type. -->
  {#if visibleChips.length > 2 || filtering}
  <div class="type-chips" role="group" aria-label="Filter by connection type">
    {#each visibleChips as chip (chip.id)}
      <button
        class="type-chip"
        class:on={filterKind === chip.id}
        aria-pressed={filterKind === chip.id}
        data-testid="connhub-filter-{chip.id}"
        onclick={() => setFilter(chip.id)}
      >{chip.label}</button>
    {/each}
  </div>
  {/if}
{/snippet}

{#snippet connListBody()}
  {@render connSearchBox()}
  {#if database.connectionsError}
    <!-- A failed load is not "No connections yet" (brokers may still list below). -->
    <LoadState
      what="connections"
      variant="compact"
      loading={database.connectionsLoading}
      error={database.connectionsError}
      empty
      onretry={() => void database.loadConnections()}
    />
  {/if}
  {#if database.connections.length === 0 && database.otherConnections.length === 0 && brokers.clusters.length === 0 && sections.length === 0 && (database.connectionsError || database.connectionsLoading)}
    {#if !database.connectionsError}<LoadState what="connections" variant="compact" loading empty />{/if}
  {:else if database.connections.length === 0 && database.otherConnections.length === 0 && brokers.clusters.length === 0 && sections.length === 0}
    <div class="conn-empty">
      No connections yet.
      <button class="link" disabled={!auth.isRoot} onclick={newConnection}>New connection →</button>
    </div>
  {:else if connFilter.trim()}
    {#if connMatches.length === 0 && clusterMatches.length === 0}
      <EmptyState icon="search" title="No matches" body={`No connections match “${connFilter.trim()}”.`} actionLabel="Clear filter" actionKind="secondary" onaction={() => (connFilter = '')} />
    {:else}
      {#each connMatches as c (c.id)}
        {@render connRow(c, 0)}
      {/each}
      {#each clusterMatches as cl (cl.id)}
        {@render clusterRow(cl, 0)}
      {/each}
    {/if}
  {:else}
    {#each tree as node (node.sec.id)}
      {@render sectionNode(node, 0)}
    {/each}

    {#if sections.length > 0}
      {#if !(filtering && !dragReveal && ungrouped.length + ungroupedClusters.length === 0)}
        <!-- Ungrouped doubles as the root / no-section drop target. -->
        <div
          role="group"
          aria-label="Ungrouped"
          class="sec-head plain"
          class:drop-target={draggedConnId || draggedClusterId || draggedSectionId}
          ondragover={(e) => {
            if (draggedConnId || draggedClusterId || draggedSectionId) e.preventDefault();
          }}
          ondrop={(e) => {
            e.preventDefault();
            onRootDrop();
          }}
          title="Drop here to remove from a section / make a section top-level"
        >
          <span class="caret-spacer"></span>
          <span class="sec-name grow">Ungrouped</span>
          {#if ungrouped.length + ungroupedClusters.length > 0}<span class="count">{ungrouped.length + ungroupedClusters.length}</span>{/if}
        </div>
        {#each ungrouped as c (c.id)}
          {@render connRow(c, 1)}
        {/each}
        {#each ungroupedClusters as cl (cl.id)}
          {@render clusterRow(cl, 1)}
        {/each}
      {/if}
    {:else}
      {#each ungrouped as c (c.id)}
        {@render connRow(c, 0)}
      {/each}
      {#each ungroupedClusters as cl (cl.id)}
        {@render clusterRow(cl, 0)}
      {/each}
    {/if}
  {/if}
{/snippet}

{#snippet schemaSideBody()}
  {#if database.sideTab === 'saved'}
    <div class="list-search">
      <Icon name="search" size={12} />
      <input dir="ltr"
        class="list-search-input"
        placeholder="Filter saved queries…"
        bind:value={savedSearch}
        aria-label="Filter saved queries"
      />
      {#if savedSearch}
        <button class="icon-btn" onclick={() => (savedSearch = '')} aria-label="Clear filter" title="Clear filter"><Icon name="x" size={12} /></button>
      {/if}
    </div>
    <LoadState what="saved queries" variant="compact" loading={database.savedQueriesLoading} error={database.savedQueriesError} empty={database.savedQueries.length === 0} onretry={() => database.loadSavedQueries()} />
    {#if database.savedQueries.length === 0 && (database.savedQueriesLoading || database.savedQueriesError)}
      <!-- Loading and failed loads are rendered above, never as an empty list. -->
    {:else if database.savedQueries.length === 0}
      <EmptyState icon="file" title="No saved queries" body="Save one from the Query tab." />
    {:else if filteredSaved.length === 0}
      <EmptyState icon="search" title="No matches" body={`No saved queries match “${savedSearch}”.`} actionLabel="Clear filter" actionKind="secondary" onaction={() => (savedSearch = '')} />
    {:else}
      {#each filteredSaved as q (q.id)}
        <div class="saved-row">
          {#if renamingId === q.id}
            <input dir="auto"
              class="rename-input"
              bind:value={renameDraft}
              use:focusOnMount
              onkeydown={(e) => {
                if (e.key === 'Enter') void commitRename();
                else if (e.key === 'Escape') cancelRename();
              }}
              onblur={() => void commitRename()}
              aria-label="Rename saved query"
            />
          {:else}
            <button class="saved-open" onclick={() => database.openSavedQuery(q)} title={stmtPreview(q.statement, 1000)}>
              <Icon name="file" size={12} />
              <span class="ellipsis">{q.name}</span>
            </button>
            <button class="icon-btn row-del" onclick={() => startRename(q)} aria-label="Rename saved query" title="Rename"><Icon name="edit" size={12} /></button>
            <button
              class="icon-btn row-del"
              onclick={async () => {
                const ok = await confirmer.ask(`Delete saved query “${q.name}”? Open tabs keep their text but are no longer linked to it.`, {
                  title: 'Delete saved query',
                  confirmLabel: 'Delete query',
                });
                if (ok) void database.deleteSavedQuery(q.id);
              }}
              aria-label="Delete saved query “{q.name}”…"
              title="Delete…"><Icon name="trash" size={12} /></button>
          {/if}
        </div>
      {/each}
    {/if}
  {:else if database.sideTab === 'history'}
    <div class="list-search">
      <Icon name="search" size={12} />
      <input dir="ltr"
        class="list-search-input"
        placeholder="Filter history…"
        bind:value={historySearch}
        aria-label="Filter query history"
      />
      {#if historySearch}
        <button class="icon-btn" onclick={() => (historySearch = '')} aria-label="Clear filter" title="Clear filter"><Icon name="x" size={12} /></button>
      {/if}
    </div>
    <LoadState what="query history" variant="compact" loading={database.historyLoading} error={database.historyError} empty={database.history.length === 0} onretry={() => database.loadHistory()} />
    {#if database.history.length === 0 && (database.historyLoading || database.historyError)}
      <!-- Loading and failed loads are rendered above, never as an empty list. -->
    {:else if database.history.length === 0}
      <EmptyState icon="clock" title="No query history yet" body="Queries you run show up here." />
    {:else if filteredHistory.length === 0}
      <EmptyState icon="search" title="No matches" body={`No history matches “${historySearch}”.`} actionLabel="Clear filter" actionKind="secondary" onaction={() => (historySearch = '')} />
    {:else}
      {#each filteredHistory as h (h.id)}
        <!-- Bounded previews: a history row can hold a whole pasted script. -->
        <button class="hist-row" class:bad={!h.ok} onclick={() => void database.openHistory(h)} title={h.error ? stmtPreview(h.error, 1000) : stmtPreview(h.statement, 1000)}>
          <span class="hist-dot" class:ok={h.ok}></span>
          <span class="hist-stmt ellipsis mono">{stmtPreview(h.statement)}</span>
          <span class="hist-meta">{h.ok ? `${h.row_count}r` : 'err'} · {fmtAgo(h.created_at)}</span>
        </button>
      {/each}
      {#if database.canLoadMoreHistory}
        <button class="load-more" onclick={() => database.loadMoreHistory()} disabled={database.historyLoadingMore}>
          {database.historyLoadingMore ? 'Loading more history…' : 'Load more'}
        </button>
      {/if}
    {/if}
  {:else}
    {@const sc = database.connections.find((c) => c.id === database.selectedConnId)}
    {#if sc}
      <!-- Which server this tree is: engine mark, name, environment. -->
      <div class="schema-conn" title="{sc.name} · {sc.kind}">
        <span class="conn-glyph {sc.kind}"><Icon name={engineGlyph(sc.kind)} size={12} /></span>
        <span class="schema-conn-name ellipsis">{sc.name}</span>
        {#if envBadge(sc)}<EnvBadge env={sc.environment} readOnly={sc.read_only} />{/if}
        <Badge variant="outline" label={sc.kind} />
      </div>
    {/if}
    <SchemaTree />
  {/if}
{/snippet}

{#if changesOpen && database.selectedConnId}<Modal title="Database changes" width={1040} onclose={() => changesOpen=false}><DatabaseChanges connectionId={database.selectedConnId} node={database.activeDb} /></Modal>{/if}

{#if accessFor}<Modal title={`Access · ${accessFor.name}`} width={820} onclose={() => accessFor=null}><ResourceAccess kind="connection" resourceId={accessFor.id} /></Modal>{/if}

{#if connFormOpen}
  <ConnectionForm
    existing={editingConn}
    kinds={ALL_KINDS}
    onclose={() => (connFormOpen = false)}
    onsaved={onConnSaved}
  />
{/if}

{#if clusterFormOpen}
  <LazyMount lazy={ClusterFormLazy} what="the cluster form" quiet props={{ cluster: editingCluster, onclose: () => (clusterFormOpen = false) }} />
{/if}

{#if sftpFor}
  <LazyMount lazy={SftpLazy} what="the SFTP browser" quiet props={{ conn: sftpFor, onclose: () => (sftpFor = null) }} />
{/if}

<!-- Import connection profiles from another DB tool (MySQL Workbench / DBeaver /
     DataGrip / NoSQLBooster). The daemon reads each tool's config from disk. -->
{#if connImportOpen && ws.currentId}
  <ConnectionImportDialog
    wsId={ws.currentId}
    onclose={() => (connImportOpen = false)}
    onimported={() => {
      void database.loadConnections();
    }}
  />
{/if}

<!-- File → table import dialog (launched from the schema-tree "Import into…"
     action or the results-grid toolbar). Keyed so it remounts fresh each open. -->
{#if database.importDialogOpen && database.selectedConnId}
  {#key database.importTable}
    <ImportDialog />
  {/key}
{/if}

<!-- "Export all rows…" an agent asked for (otto.ui_db_export): prefilled, but
     the person picks the folder + file and confirms. Closing without exporting
     reports `exported:false` back to the agent. -->
{#if database.exportRequest}
  {@const req = database.exportRequest}
  <ExportDialog
    statement={req.statement}
    connectionId={req.connId}
    node={req.node}
    canExport={resourceAccess.can('connection', req.connId, 'db_export', 'database', 'view', databaseAccessChild(req.node ?? undefined))}
    initialFormat={req.format}
    initialMaxRows={req.maxRows}
    requestedBy={req.agentLabel}
    ondone={(r) => {
      req.done({ exported: true, path: r.local_path, rows: r.rows, bytes: r.bytes });
    }}
    onclose={() => {
      if (database.exportRequest === req) database.exportRequest = null;
      req.done({ exported: false });
    }}
  />
{/if}

<style>
  .db-root {
    height: 100%;
    display: flex;
    flex-direction: column;
    min-height: 0;
    min-width: 0;
  }
  .db-page {
    flex: 1;
    display: flex;
    min-height: 0;
    /* Let the page shrink inside its (flex) content pane instead of forcing its
       intrinsic width — belt-and-suspenders with .db-main's min-width:0 below. */
    min-width: 0;
  }
  .db-side {
    /* Default width; on tablet/desktop an inline `width:{sideW}px` (drag-resizable,
       persisted) overrides this, and the phone media query forces full width. */
    width: 300px;
    flex-shrink: 0;
    border-inline-end: 1px solid var(--border);
    display: flex;
    flex-direction: column;
    min-height: 0;
  }
  .conn-list {
    display: flex;
    flex-direction: column;
    gap: 1px;
    padding: 10px 8px;
    border-bottom: 1px solid var(--border);
    overflow-y: auto;
  }
  /* On tablet/desktop the list lives inside the scrollable .side-body tab, so it
     fills the full sidebar height (no cap) and the side-body owns the scroll. */
  .side-body .conn-list {
    padding: 0;
    border-bottom: none;
    overflow-y: visible;
  }
  /* Connection filter box — mirrors SchemaTree's "Filter schema" input. */
  .tree-search {
    display: flex;
    align-items: center;
    gap: 4px;
    padding: 4px 6px 6px;
    margin-bottom: 2px;
    border-bottom: 1px solid color-mix(in srgb, var(--border) 60%, transparent);
    color: var(--text-dim);
    flex-shrink: 0;
  }
  .tree-search-input {
    flex: 1;
    border: none;
    background: transparent;
    color: var(--text);
    font-size: var(--fs-s);
    outline: none;
    min-width: 0;
  }
  /* The bare input drops its outline; the search row shows focus instead. */
  .tree-search:focus-within {
    box-shadow: inset 0 -2px 0 var(--accent-text);
  }
  .tree-search-input::placeholder {
    color: var(--text-dim);
  }
  .tree-search-clear {
    display: grid;
    place-items: center;
    border: none;
    background: transparent;
    color: var(--text-dim);
    cursor: pointer;
    padding: 0;
    flex-shrink: 0;
  }
  .tree-search-clear:hover {
    color: var(--text);
  }
  .conn-empty,
  /* Schema-tab empty state with its way-forward buttons. */
  .side-empty {
    display: flex;
    flex-direction: column;
    gap: 4px;
    padding: 4px 2px;
  }
  .side-empty-actions {
    display: flex;
    gap: 6px;
    padding: 0 6px;
    flex-wrap: wrap;
  }
  .list-search {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 4px 6px 6px;
    color: var(--text-dim);
  }
  .list-search-input {
    flex: 1;
    min-width: 0;
    border: 1px solid var(--border);
    background: var(--surface-2);
    color: var(--text);
    border-radius: var(--radius-s);
    padding: 4px 6px;
    font-size: var(--fs-s);
  }
  .list-search-input:focus {
    outline: none;
    border-color: var(--accent-text); box-shadow: 0 0 0 3px var(--accent-soft-strong)
  }
  .rename-input {
    flex: 1;
    min-width: 0;
    border: 1px solid var(--accent);
    background: var(--surface-2);
    color: var(--text);
    border-radius: var(--radius-s);
    padding: 4px 6px;
    margin: 0 2px;
    font-size: var(--fs-s);
  }
  .rename-input:focus {
    outline: none;
  }
  .load-more {
    width: 100%;
    border: none;
    background: transparent;
    color: var(--accent-text);
    cursor: pointer;
    font-size: var(--fs-s);
    padding: 8px 6px;
    text-align: center;
  }
  .load-more:hover:not(:disabled) {
    text-decoration: underline;
  }
  .load-more:disabled {
    color: var(--text-dim);
    cursor: default;
  }
  .link {
    border: none;
    background: none;
    color: var(--accent-text);
    cursor: pointer;
    font-size: var(--fs-s);
    padding: 0;
  }
  .conn-item {
    display: flex;
    align-items: center;
    gap: 6px;
    /* min-height (not a fixed height) so the row grows when a long name wraps. */
    min-height: 26px;
    padding: 2px 6px;
    border: none;
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--text);
    cursor: pointer;
    text-align: start;
  }
  .conn-item:hover {
    background: var(--hover);
  }
  .conn-row.open:not(.active) .conn-item {
    background: var(--hover);
  }
  .conn-row.active .conn-item {
    background: var(--accent-soft);
  }
  .conn-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 6px 8px 2px;
  }
  .conn-head-title {
    font-size: var(--fs-xs);
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: .06em;
    color: var(--text-dim);
  }
  .head-btns {
    display: flex;
    align-items: center;
    gap: 1px;
  }
  /* --- Section hierarchy rows --- */
  .sec-head {
    position: relative;
    display: flex;
    align-items: center;
    gap: 4px;
    height: 24px;
    padding: 0 6px;
    border-radius: var(--radius-s);
    cursor: pointer;
    user-select: none;
    color: var(--text-dim);
  }
  .sec-head:hover {
    background: var(--hover);
  }
  .sec-head.plain {
    cursor: default;
    margin-top: 4px;
  }
  .sec-head.drop-target {
    outline: 1px dashed color-mix(in srgb, var(--accent-text) 55%, transparent);
    outline-offset: -1px;
    background: var(--accent-faint);
  }
  .sec-name {
    font-size: var(--fs-xs);
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: .06em;
    color: var(--text-dim);
  }
  .caret {
    display: grid;
    place-items: center;
    width: 16px;
    height: 16px;
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
    width: 16px;
    flex-shrink: 0;
  }
  /* The folder count sits in the same end column as the connection rows'
     type/env badges: the section's hover actions float over the row end (like
     .conn-actions) instead of reserving their width, and the count is sized
     like a badge so the numbers line up with the pills below them. */
  .count {
    flex-shrink: 0;
    font-size: var(--fs-xs);
    color: var(--text-dim);
    min-width: 16px;
    text-align: end;
    font-variant-numeric: tabular-nums;
  }
  .sec-actions {
    position: absolute;
    inset-inline-end: 2px;
    top: 50%;
    transform: translateY(-50%);
    display: flex;
    gap: 0;
    padding: 1px 2px;
    border-radius: var(--radius-s);
    background: var(--surface);
    box-shadow: 0 0 0 1px var(--border);
    opacity: 0;
  }
  .sec-head:hover .sec-actions,
  .sec-head:focus-within .sec-actions {
    opacity: 1;
  }
  .conn-row.dragging {
    opacity: 0.5;
  }
  .conn-row {
    position: relative;
    display: flex;
    align-items: center;
    gap: 2px;
  }
  .conn-row .conn-item {
    flex: 1;
    min-width: 0;
  }
  /* Hover actions float over the row's end instead of reserving their width in
     every row — that dead space squeezed the name + type/env badges until names
     broke mid-word. */
  .conn-actions {
    position: absolute;
    inset-inline-end: 2px;
    top: 50%;
    transform: translateY(-50%);
    display: flex;
    gap: 1px;
    padding: 1px 2px;
    border-radius: var(--radius-s);
    background: var(--surface);
    box-shadow: 0 0 0 1px var(--border);
    opacity: 0;
  }
  .conn-row:hover .conn-actions,
  .conn-row:focus-within .conn-actions {
    opacity: 1;
  }
  /* Touch has no hover: keep the ⋯ in the row's flow, always visible. */
  @media (hover: none) {
    .conn-actions {
      position: static;
      transform: none;
      background: none;
      box-shadow: none;
      opacity: 1;
    }
  }
  .conn-glyph {
    display: grid;
    place-items: center;
    flex-shrink: 0;
    color: var(--text-dim);
  }
  /* Engine glyphs stay neutral (no per-source hues); the kind tag names it. */
  .conn-row.active .conn-item .conn-glyph {
    color: var(--accent-text);
  }
  .conn-name {
    flex: 1;
    min-width: 0;
    font-size: var(--fs-s);
    font-weight: 500;
    line-height: 1.35;
    /* Show the FULL connection name instead of clipping it: wrap onto extra lines
       (breaking long unbroken tokens like host URLs), up to 3 lines (~50–75 chars)
       before ellipsizing — so deeply-nested names stay readable at any width. */
    overflow-wrap: anywhere;
    display: -webkit-box;
    -webkit-line-clamp: 3;
    line-clamp: 3;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }
  .ss-tabs { display: contents; }
  .side-switch {
    display: flex;
    align-items: center;
    gap: 2px;
    padding: 8px 8px 6px;
    border-bottom: 1px solid var(--border);
    overflow-x: auto;
    scrollbar-width: none;
  }
  .side-switch::-webkit-scrollbar {
    display: none;
  }
  .side-switch .ss {
    flex-shrink: 0;
  }
  .ss {
    height: 24px;
    padding: 0 6px;
    border: none;
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--text-dim);
    font-size: var(--fs-s);
    font-weight: 500;
    cursor: pointer;
  }
  .ss:hover {
    background: var(--hover);
  }
  .ss.active {
    background: var(--accent-soft);
    color: var(--accent-text);
  }
  .side-body {
    flex: 1;
    overflow-y: auto;
    overflow-x: hidden;
    padding: 8px;
    min-height: 0;
  }
  .saved-row,
  .hist-row {
    display: flex;
    align-items: center;
    gap: 6px;
    width: 100%;
    border: none;
    background: transparent;
    border-radius: var(--radius-s);
    cursor: pointer;
    color: var(--text);
    text-align: start;
    padding: 0 6px;
  }
  .saved-row {
    padding: 0;
  }
  .saved-open {
    flex: 1;
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
    border: none;
    background: transparent;
    color: var(--text);
    cursor: pointer;
    height: 28px;
    padding: 0 6px;
    border-radius: var(--radius-s);
    font-size: var(--fs-s);
  }
  .saved-row:hover,
  .hist-row:hover {
    background: var(--hover);
  }
  .row-del {
    opacity: 0;
  }
  .saved-row:hover .row-del {
    opacity: 1;
  }
  .hist-row {
    height: 32px;
  }
  .hist-dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--status-exited);
    flex-shrink: 0;
  }
  .hist-dot.ok {
    background: var(--status-working);
  }
  .hist-stmt {
    flex: 1;
    min-width: 0;
    font-size: var(--fs-xs);
  }
  .hist-meta {
    font-size: var(--fs-xs);
    color: var(--text-dim);
    flex-shrink: 0;
    font-variant-numeric: tabular-nums;
  }
  .db-main {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    min-height: 0;
    container: dbmain / inline-size;
  }
  /* Production connection → a persistent red rail down the main area. */
  .db-main.danger-rail {
    border-inline-start: 3px solid var(--status-exited);
  }
  /* Read-only (non-prod) connection → a softer amber rail. */
  .db-main.guard-rail {
    border-inline-start: 3px solid var(--status-working);
  }
  /* Guardrail banner above the main tabs. */
  .guard-banner {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 14px;
    font-size: var(--fs-s);
    line-height: 1.4;
    color: var(--success);
    background: color-mix(in srgb, var(--status-working) 12%, transparent);
    border-bottom: 1px solid color-mix(in srgb, var(--status-working) 35%, transparent);
  }
  .guard-banner.prod {
    color: var(--danger);
    background: color-mix(in srgb, var(--status-exited) 13%, transparent);
    border-bottom-color: color-mix(in srgb, var(--status-exited) 40%, transparent);
    font-weight: 600;
  }
  /* Per-row connection-type tag (mysql / ssh / kafka / …) — neutral, so the
     env badge keeps the color signal. */
  .schema-conn {
    display: flex;
    align-items: center;
    gap: 6px;
    height: 30px;
    padding: 0 6px;
    margin-bottom: 4px;
    border-bottom: 1px solid color-mix(in srgb, var(--border) 60%, transparent);
    min-width: 0;
  }
  .schema-conn-name {
    flex: 1;
    min-width: 0;
    font-size: var(--fs-s);
    font-weight: 600;
  }
  /* Type-filter chips under the tree search. */
  .type-chips {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
    padding: 4px 8px 6px;
    border-bottom: 1px solid var(--border);
  }
  .type-chip {
    font-size: var(--fs-xs);
    padding: 2px 8px;
    border-radius: 999px;
    border: 1px solid var(--border);
    background: transparent;
    color: var(--text-dim);
    cursor: pointer;
  }
  .type-chip:hover {
    background: var(--hover);
  }
  .type-chip.on {
    color: var(--accent-text);
    border-color: var(--accent-line);
    background: var(--accent-soft);
  }
  /* Prod / guarded connection tabs get a tinted edge. */
  .conn-tab.prod {
    border-color: color-mix(in srgb, var(--status-exited) 45%, transparent);
  }
  .conn-tab.prod.active {
    border-color: color-mix(in srgb, var(--status-exited) 65%, transparent);
  }
  .conn-tab.guarded {
    border-color: color-mix(in srgb, var(--status-working) 40%, transparent);
  }
  /* Top-level connection tabs (Workbench-style), above the main tab row. */
  .conn-tabs {
    display: flex;
    align-items: center;
    gap: 2px;
    height: 36px;
    padding: 0 10px;
    border-bottom: 1px solid var(--border);
    background: var(--bg);
    overflow-x: auto;
    scrollbar-width: none;
    flex-shrink: 0;
  }
  .conn-tabs::-webkit-scrollbar {
    display: none;
  }
  .conn-tab-path {
    font-size: var(--fs-xs);
    text-transform: uppercase;
    letter-spacing: .06em;
    color: var(--accent-text);
    background: var(--accent-soft);
    border-radius: 999px;
    padding: 1px 6px;
    flex-shrink: 0;
    white-space: nowrap;
  }
  .conn-tab {
    display: flex;
    align-items: center;
    height: 26px;
    padding-block: 0; padding-inline: 8px 2px;
    border-radius: var(--radius-s);
    border: 1px solid transparent;
    color: var(--text-dim);
    cursor: pointer;
    white-space: nowrap;
    max-width: 320px;
    flex-shrink: 0;
    transition: background var(--dur-fast) ease-out, color var(--dur-fast) ease-out;
  }
  .conn-tab:hover {
    background: var(--hover);
  }
  .conn-tab.active {
    background: var(--surface);
    border-color: var(--border);
    color: var(--text);
  }
  .conn-tab-main {
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
    border: none;
    background: transparent;
    color: inherit;
    cursor: pointer;
    font-size: var(--fs-m);
    font-weight: 500;
    padding: 0;
    height: 100%;
  }
  .conn-tab-glyph {
    display: grid;
    place-items: center;
    flex-shrink: 0;
    color: var(--text-dim);
  }
  /* Engine glyphs stay neutral (no per-source hues); the kind tag names it. */
  .conn-tab.active .conn-tab-glyph {
    color: var(--accent-text);
  }
  .conn-tab-name {
    min-width: 0;
    max-width: 220px;
  }
  .conn-tab-more,
  .conn-tab-close {
    display: grid;
    place-items: center;
    width: 17px;
    height: 17px;
    margin-inline-start: 4px;
    padding: 0;
    border: none;
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--text-dim);
    cursor: pointer;
    opacity: 0;
    flex-shrink: 0;
    transition: opacity var(--dur-fast) ease-out, background var(--dur-fast) ease-out, color var(--dur-fast) ease-out;
  }
  .conn-tab-more {
    margin-inline-start: 6px;
  }
  .conn-tab-more + .conn-tab-close {
    margin-inline-start: 0;
  }
  /* Revealed on hover, on the active tab and whenever focus is inside the tab;
     always visible on touch (no hover to reveal them). */
  .conn-tab:hover :is(.conn-tab-more, .conn-tab-close),
  .conn-tab:focus-within :is(.conn-tab-more, .conn-tab-close),
  .conn-tab.active :is(.conn-tab-more, .conn-tab-close),
  .conn-tab-more:focus-visible,
  .conn-tab-close:focus-visible {
    opacity: 1;
  }
  @media (hover: none) {
    .conn-tab {
      height: 32px;
    }
    .conn-tab-more,
    .conn-tab-close {
      opacity: 1;
      width: 28px;
      height: 28px;
    }
  }
  .conn-tab-more:hover,
  .conn-tab-close:hover {
    background: var(--hover);
    color: var(--text);
  }
  /* Workbench toolbar: the view switch (segmented) + connection status/actions. */
  .main-tabs {
    display: flex;
    align-items: center;
    gap: 8px;
    height: 44px;
    box-sizing: border-box;
    padding: 0 16px;
    border-bottom: 1px solid var(--border);
    flex-shrink: 0;
  }
  .view-switch {
    flex-shrink: 0;
  }
  .view-switch .mt {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    height: 24px;
    padding: 0 10px;
  }
  .view-switch .mt :global(svg) {
    opacity: 0.75;
  }
  .view-switch .mt.active :global(svg) {
    opacity: 1;
  }
  .view-switch .mt:focus-visible {
    outline: 1.5px solid var(--accent-text);
    outline-offset: 1px;
  }
  .conn-status {
    display: flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
  }
  /* A narrower workbench first sheds the redundant status (the engine is on
     the schema header too, the version is in the dot's tooltip); only a
     phone-narrow one drops the view words and keeps the icons. */
  @container dbmain (max-width: 900px) {
    .cap-chip,
    .health-ver,
    .health-lat {
      display: none;
    }
  }
  /* Five views (SQL engines) + status + Changes + Test don't fit a ~740px
     workbench: the status/utility buttons go icon-only first (they carry a
     title + aria-label), so nothing is ever clipped off the trailing edge. */
  @container dbmain (max-width: 900px) {
    .main-tabs.many .conn-status .lbl {
      display: none;
    }
  }
  @container dbmain (max-width: 700px) {
    .conn-status .lbl {
      display: none;
    }
  }
  @container dbmain (max-width: 640px) {
    .view-switch .mt {
      font-size: 0;
      gap: 0;
      padding: 0 8px;
    }
  }
  .cap-chip {
    font-size: var(--fs-xs);
    text-transform: uppercase;
    letter-spacing: .06em;
    color: var(--text-dim);
    background: var(--surface-2);
    padding: 1px 6px;
    border-radius: 999px;
  }
  .test-dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--status-exited);
  }
  .test-dot.ok {
    background: var(--status-working);
  }
  .conn-tab-spin {
    color: var(--text-dim);
    margin-inline-start: 4px;
    flex-shrink: 0;
  }
  /* Restored but not connected yet: a hollow muted ring (no colour = no claim). */
  .conn-tab-idle {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    border: 1.5px solid var(--text-dim);
    margin-inline-start: 4px;
    flex-shrink: 0;
  }
  .conn-tab-dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--status-exited);
    margin-inline-start: 4px;
    flex-shrink: 0;
  }
  .conn-state {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .conn-state.err {
    color: var(--danger);
    font-weight: 500;
  }
  .conn-state.ok {
    color: var(--text-dim);
    max-width: 220px;
  }
  .health-dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--status-working);
    flex-shrink: 0;
  }
  .health-ver {
    max-width: 130px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .health-lat {
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
    flex-shrink: 0;
  }
  
  /* Horizontal split holding the active view + (optionally) the DB Assistant. */
  .main-split {
    flex: 1;
    min-height: 0;
    min-width: 0;
    display: flex;
    flex-direction: row;
  }
  .main-body {
    flex: 1;
    min-height: 0;
    min-width: 0;
    padding: 12px 16px 16px;
    display: flex;
    flex-direction: column;
  }
  /* SSH/custom terminal pane — fills the workbench body below the tab strip. */
  .term-pane {
    flex: 1;
    min-height: 0;
    min-width: 0;
    display: flex;
    overflow: hidden;
  }
  .term-pane :global(> *) {
    flex: 1;
    min-height: 0;
    min-width: 0;
  }
  /* Draggable divider between the view and the assistant pane. */
  .assist-divider {
    flex: none;
    width: 6px;
    cursor: col-resize;
    background: var(--border);
    position: relative;
    touch-action: none;
  }
  .assist-divider:focus-visible {
    outline: none;
  }
  .assist-divider:hover,
  .assist-divider:focus-visible {
    background: var(--accent);
  }
  /* Draggable divider between the connections sidebar and the main area. Sits flush
     against the sidebar's inline-end border; a hit-area wider than its visible line
     makes it easy to grab. */
  .db-side.collapsed {
    display: none;
  }
  .empty-alt {
    display: flex;
    flex-wrap: wrap;
    justify-content: center;
    gap: 6px;
  }
  .empty-note {
    margin: 0;
    font-size: var(--fs-s);
    color: var(--text-dim);
  }

  /* The DB Assistant pane — fixed (resizable) width, pinned to the right edge. */
  .assist-pane {
    flex: none;
    min-width: 0;
    min-height: 0;
    display: flex;
    border-inline-start: 1px solid var(--border);
  }
  .grow {
    flex: 1;
  }
  .ellipsis {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  /* ───────────────── Phone (≤640px) ─────────────────
     The desktop layout packs the connection tree + schema + query tabs +
     toolbar + editor + results into ONE fixed viewport height — on a phone
     that crushes everything to unreadable, unscrollable slivers and the
     RESULTS fall off the bottom unreachable. On a phone we instead let the
     whole page scroll as a normal vertical document: stack the sidebar over
     the main area, give each section an intrinsic (readable) height, and turn
     the results grid into its own bounded, internally-scrolling block so a
     query's rows are always reachable. */
  @media (max-width: 640px) {
    /* The page itself becomes the scroll container (its parent .content is
       overflow:hidden and fixed-height — we can't change that from here). */
    .db-page {
      flex-direction: column;
      overflow-y: auto;
      -webkit-overflow-scrolling: touch;
    }
    /* Sidebar: full-width band on top, no longer fighting for height — the
       connection list and schema each get their own bounded scroll. */
    .db-side {
      width: 100%;
      flex: 0 0 auto;
      border-inline-end: none;
      border-bottom: 1px solid var(--border);
    }
    /* Each section's body scrolls INDEPENDENTLY when expanded (own max-height +
       overflow) so the user scrolls within the panel they care about. */
    .conn-list {
      max-height: 34vh;
      overflow-y: auto;
      -webkit-overflow-scrolling: touch;
    }
    .side-body {
      flex: 0 0 auto;
      max-height: 34vh;
      overflow-y: auto;
      -webkit-overflow-scrolling: touch;
    }
    /* ── Collapsible accordion headers (phone-only) ── */
    .acc-head {
      padding: 4px 8px;
      min-height: 44px;
      border-top: 1px solid var(--border);
    }
    .acc-toggle {
      display: flex;
      align-items: center;
      gap: 8px;
      flex: 1;
      min-width: 0;
      border: none;
      background: transparent;
      color: var(--text-dim);
      cursor: pointer;
      padding: 8px 2px;
      text-align: start;
    }
    .acc-toggle .conn-head-title {
      font-size: var(--fs-m);
    }
    .acc-count {
      font-size: var(--fs-xs);
      color: var(--text-dim);
      background: var(--surface-2);
      border-radius: 999px;
      padding: 1px 8px;
      font-variant-numeric: tabular-nums;
    }
    /* A collapsed section's body is removed from flow entirely. */
    .acc-collapsed {
      display: none !important;
    }
    /* Larger, legible text for the connection rows + tiny meta on phones. */
    .conn-name {
      font-size: var(--fs-l);
    }
    .conn-item {
      min-height: 40px;
    }
    .conn-head-title {
      font-size: var(--fs-s);
    }
    .conn-empty,
    .list-empty {
      font-size: var(--fs-m);
    }
    .hist-stmt {
      font-size: var(--fs-m);
    }
    .hist-meta,
    .count {
      font-size: var(--fs-s);
    }
    .saved-open {
      font-size: var(--fs-l);
      height: 36px;
    }
    .hist-row {
      height: 40px;
    }
    /* Main area: let it grow to its natural height so it stacks under the
       sidebar and the page scrolls — instead of being a clipped flex:1 box. */
    .db-main {
      flex: 0 0 auto;
      min-height: 0;
    }
    .main-body {
      flex: 0 0 auto;
      padding: 10px 12px 16px;
    }
    /* On a phone the split stacks: the assistant drops BELOW the view as a
       full-width, fixed-height block; the vertical divider is hidden (there's
       no side-by-side to drag). */
    .main-split {
      flex-direction: column;
    }
    .assist-divider {
      display: none;
    }
    .assist-pane {
      width: 100% !important;
      height: 60vh;
      border-inline-start: none;
      border-top: 1px solid var(--border);
    }
    /* Bigger tap targets + readable text for the tab strips. */
    .conn-tabs {
      height: 40px;
    }
    /* Let the tab row wrap so the engine chip + Test button drop to their own
       line on the narrowest phones instead of jutting past the edge. */
    .main-tabs {
      height: auto;
      min-height: 44px;
      padding: 6px 12px;
      flex-wrap: wrap;
      row-gap: 4px;
    }
    .view-switch {
      max-width: 100%;
      overflow-x: auto;
    }
    /* The flexible spacer would push conn-status onto an overflowing line —
       make it a full-width break so the status wraps cleanly below the tabs. */
    .main-tabs .grow {
      flex-basis: 100%;
      height: 0;
    }
    .view-switch .mt {
      height: 30px;
      font-size: var(--fs-m);
      padding: 0 10px;
    }
    .conn-name {
      font-size: var(--fs-l);
    }
    .ss {
      height: 30px;
      font-size: var(--fs-m);
    }
    /* The status row (engine chip + Test) can wrap rather than overflow. */
    .conn-status {
      flex-wrap: wrap;
    }
  }

  /* ───────────────── Tablet (641–1024px) ─────────────────
     The tablet keeps the desktop side-by-side layout (sidebar + main), but the
     shell ALSO shows a persistent ~220px Navigator column here, so the main
     column is doubly squeezed. Two guards keep content on-screen:
       1. The dense tab/status row wraps (engine chip + Test button drop to their
          own line) instead of being pushed past the (.content overflow:hidden)
          edge and becoming unreachable — the same treatment the phone uses.
       2. The connection sidebar is capped at 45vw. It's flex-shrink:0 at a
          persisted px width (db.sideW, up to ~640px carried from a desktop
          session); without the cap that fixed width + the Navigator could force
          the main pane off-screen. The cap never bites the default 300px sidebar
          at these widths, so the layout still reads as side-by-side. */
  @media (min-width: 641px) and (max-width: 1024px) {
    .db-side {
      max-width: 45vw;
    }
    .main-tabs {
      height: auto;
      min-height: 44px;
      padding: 6px 12px;
      flex-wrap: wrap;
      row-gap: 4px;
    }
    .main-tabs .grow {
      flex-basis: 100%;
      height: 0;
    }
    .conn-status {
      flex-wrap: wrap;
    }
  }
</style>
