<script lang="ts">
  import { announce } from '../../lib/announce.svelte';
  import { rowMenu } from '../../lib/rowMenu';
  import { NO_WORKSPACE } from '../../lib/labels';
  import { toastError } from '../../lib/toastError';
  // DB workbench (`#/database/<connection>`): open connections as tabs over a
  // schema / saved / history sidebar and the active view (Query / Builder /
  // Structure / Diagram / Dashboards). The connection list itself is its own
  // page (modules/connections/ConnectionsPage.svelte, `#/connections`).
  import { tick, untrack } from 'svelte';
  import Icon from '../../lib/components/Icon.svelte';
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
  import { resourceAccess } from '../../lib/stores/resource-access.svelte';
  import { auth } from '../../lib/stores/auth.svelte';
  import { connectionList } from '../../lib/stores/connectionList.svelte';
  import type { ConnectionPrefill } from '../connections/ConnectionForm.svelte';
  import LazyMount from '../../lib/components/LazyMount.svelte';
  import { lazyComponent, whenIdle } from '../../lib/lazy-component.svelte';
  import SshPane from './SshPane.svelte';
  import DbConnErrorBanner from './DbConnErrorBanner.svelte';
  import DiagnoseAside from './DiagnoseAside.svelte';
  import { connDiagnose } from './diagnose.svelte';
  import { sshTabs } from './ssh-tabs.svelte';
  import DbSavedHistorySidebar from './DbSavedHistorySidebar.svelte';
  import { editorBar } from './workbenchBar.svelte';
  import { comparePin, toggleCompare } from './comparePin.svelte';
  import { KIND_LABEL, type KindWithLabel } from '../../lib/database/kindLabel';
  import type { MenuItem } from '../../lib/contextmenu.svelte';
  import { buildDbCliSnippet, cliLabelForKind } from '../../lib/database/cliSnippet';
  import { processEngine } from './process-list';
  import { copyWithToast } from '../../lib/clipboard';
  import { databaseAccessChild } from '../../lib/access-options';
  import { database, engineGlyph, type DbMainTab, type DbSideTab } from '../../lib/stores/database.svelte';
  import Tabs, { type TabItem } from '../../lib/components/Tabs.svelte';
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
  import { lsGet, lsSet } from '../../lib/storage';
  import type {
    BrokerCluster,
    Connection,
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
  const ClusterViewerLazy = lazyComponent(() => import('../brokers/ClusterViewer.svelte'));
  const ClusterFormLazy = lazyComponent(() => import('../brokers/ClusterForm.svelte'));
  const SftpLazy = lazyComponent(() => import('../connections/SftpBrowser.svelte'));
  // The assistant embeds a Terminal (xterm) — only loaded when it's opened.
  const AssistantLazy = lazyComponent(() => import('./DbAssistantPanel.svelte'));
  // Dialogs only ever shown as modals: off the first paint.
  const ImportDialogLazy = lazyComponent(() => import('./ImportDialog.svelte'));
  const ExportDialogLazy = lazyComponent(() => import('./ExportDialog.svelte'));
  const ProcessListLazy = lazyComponent(() => import('./ProcessList.svelte'));
  const DatabaseChangesLazy = lazyComponent(() => import('./DatabaseChanges.svelte'));
  const ResourceAccessLazy = lazyComponent(() => import('../../lib/components/ResourceAccess.svelte'));
  $effect(() =>
    whenIdle(() => {
      StructureLazy.prefetch();
      QueryBuilderLazy.prefetch();
      DashboardsLazy.prefetch();
      DiagramLazy.prefetch();
    }, 4000),
  );
  // Edit form, opened from a tab's menu, the connection-error banner or the
  // SSH pane (lazy: creating connections lives on the Connections page).
  const ConnectionFormLazy = lazyComponent(() => import('../connections/ConnectionForm.svelte'));
  const ALL_KINDS: ConnectionPrefill['kind'][] = ['ssh', 'mysql', 'postgres', 'redis', 'mongodb', 'clickhouse', 'custom'];
  let connFormOpen = $state(false);
  let editingConn = $state<Connection | null>(null);
  // Kafka cluster form (New Cluster / edit a cluster row).
  let clusterFormOpen = $state(false);
  let editingCluster = $state<BrokerCluster | null>(null);
  // SFTP file browser over an ssh profile's row action.
  // Spinner guard while a terminal session is being spawned for a row.
  let opening = $state<Record<string, boolean>>({});

  // ── Phone accordion ────────────────────────────────────────────────────────
  // On a phone the whole page scrolls and each major section (Connections /

  // Dock this connection as a pane in the Agents split (beside an agent), with
  // the full DB Explorer. Right-clicked from a connection tab or sidebar row.
  function openConnInAgents(c: Connection): void {
    void database.openConnection(c.id);
    ws.openInSplit(DB_PANE_ID);
    router.go('agents');
  }
  $effect(()=>resourceAccess.subscribe(change=>{
    if(change.type==='reset' && change.identity){connFormOpen=false;editingConn=null;accessFor=null;processesFor=null;changesOpen=false;}
  }));
  let changesOpen = $state(false);
  let accessFor = $state<Connection | null>(null); let processesFor = $state<Connection | null>(null);
  const connectionAccess = (c: Connection, operation: string, capability: 'view'|'edit'|'admin'='edit') => resourceAccess.can('connection',c.id,operation,'connections',capability);
  $effect(() => { for (const id of database.openConnIds) void resourceAccess.load('connection', id); });
  /** A database connection's actions: its tab's ⋯ / right-click and the
   *  Connection group of the header's ⋯. */
  function connItems(c: Connection): MenuItem[] {
    const cliCmd = buildDbCliSnippet(c, { activeDb: database.selectedConnId === c.id ? database.activeDb : null });
    const cliName = cliLabelForKind(c.kind);
    return [
      { label: 'Open beside agents (split)', icon: 'split', action: () => openConnInAgents(c) },
      ...(connectionAccess(c,'shell') ? [{ label: 'Open terminal client', icon: 'terminal', action: () => void openTerminal(c) }] : []),
      ...(cliCmd && cliName ? [{ label: `Copy as ${cliName}`, icon: 'copy', action: () => void copyWithToast(cliCmd, `${cliName} command`) }] : []),
      ...(database.isWarming(c.id) ? [{ label: 'Stop connecting', icon: 'stop', action: () => database.stopConnecting(c.id) }] : []),
      ...(database.connStatus.get(c.id)?.phase === 'idle' && database.selectedConnId !== c.id ? [{ label: 'Connect in background', icon: 'refresh', action: () => void database.warm(c.id) }] : []),
      ...popoutItems(`database/${c.id}`, c.name),
      ...(processEngine(c.kind) ? [{ label: 'Processes…', icon: 'gauge', action: () => (processesFor = c) }] : []),
      { separator: true },
      ...(connectionAccess(c,'configure','admin') ? [{ label: 'Edit…', icon: 'edit', action: () => editConnection(c) }] : []),
      ...(auth.isRoot || connectionAccess(c,'manage_access','admin') ? [{ label: 'Access', icon: 'key', action: () => {accessFor=c;} }] : []),
    ];
  }
  function connMenu(e: MouseEvent, c: Connection): void {
    ctxMenu.show(e, connItems(c));
  }

  // ── The header's one ⋯ (grouped): view, query, connection, sidebar ───────
  const VIEWS: { id: DbMainTab; label: string; show: () => boolean }[] = [
    { id: 'query', label: 'Query', show: () => true },
    { id: 'builder', label: 'Builder', show: () => database.supportsBuilder },
    { id: 'structure', label: 'Structure', show: () => true },
    { id: 'diagram', label: 'Diagram', show: () => database.capabilities?.engine !== 'redis' },
    { id: 'dashboards', label: 'Dashboards', show: () => true },
  ];
  const viewLabel = $derived(VIEWS.find((v) => v.id === database.mainTab)?.label ?? 'Query');
  function headerMenu(): MenuItem[] {
    const c = database.selectedConn;
    if (!c || database.activePane !== null) return [];
    const query = database.mainTab === 'query' ? (editorBar.cur?.menu(null) ?? []) : [];
    return [
      ...VIEWS.filter((v) => v.show()).map((v) => ({ label: v.label, checked: database.mainTab === v.id, action: () => database.setMainTab(v.id) })),
      { separator: true },
      ...query,
      ...(!viewport.isPhone && !database.sidebarCollapsed ? [{ label: 'Hide tables', icon: 'panel' as const, hint: '⌘B', action: () => database.toggleSidebar() }] : []),
      { label: comparePin.value ? 'Clear comparison' : 'Compare results', icon: 'columns', title: 'Keep loaded results beside the current query', action: toggleCompare },
      { separator: true },
      { label: database.testing ? 'Testing connection…' : 'Test connection', icon: 'plug', disabled: database.testing, action: () => void database.testConnection() },
      ...(['mysql', 'postgres'].includes(c.kind) ? [{ label: 'Database changes', icon: 'branch', title: 'Reviewed database changes for this connection', action: () => (changesOpen = true) }] : []),
      ...connItems(c),
      { separator: true },
      { label: 'Close connection', icon: 'x', title: 'Close this connection (its query tabs come back when you open it again)', action: () => database.closeConnection(c.id) },
    ];
  }

  // Tables / History: open the hidden side pane on that view.
  // Phone: Tables / History are a sheet over the results (Back closes it).
  let phoneSide = $state<DbSideTab | null>(null);
  let sheetKey = '';
  $effect(() => {
    // Opening a table (a new / switched tab) or a view closes the sheet.
    const k = `${database.activeTab}:${database.tabs.length}:${database.mainTab}:${database.selectedConnId}`;
    untrack(() => {
      if (phoneSide && sheetKey && k !== sheetKey) phoneSide = null;
      sheetKey = k;
    });
  });
  function showSide(tab: DbSideTab): void {
    database.setSideTab(tab);
    if (viewport.isPhone) phoneSide = tab;
    // Reopening the tables puts focus back on the selected object.
    else if (database.sidebarCollapsed) void showSchemaSidebar();
  }

  // The state, in words: Connected · MySQL 8.0.36 · 14 ms / Connecting… /
  // Can't connect — reason. Never a bare dot, never a spinner beside a Run
  // that cannot work.
  const connState = $derived.by(() => {
    const st = database.activeConnStatus;
    const c = database.selectedConn;
    const engine = c ? (KIND_LABEL[c.kind as KindWithLabel] ?? c.kind) : '';
    if (st?.phase === 'error') {
      const reason = (st.error ?? '').replace(/\s+/g, ' ').trim();
      return { word: 'Can’t connect', tone: 'danger', detail: reason.length > 70 ? `${reason.slice(0, 67)}…` : reason, tip: st.error ?? '' };
    }
    if (st?.phase === 'ready') {
      const detail = [st.serverVersion ? `${engine} ${st.serverVersion}` : engine, st.latencyMs != null ? `${Math.round(st.latencyMs)} ms` : ''].filter(Boolean).join(' · ');
      return { word: 'Connected', tone: 'success', detail, tip: detail };
    }
    if (st?.phase === 'idle') return { word: 'Not connected yet', tone: '', detail: engine, tip: '' };
    return { word: 'Connecting…', tone: '', detail: engine, tip: '' };
  });
  let retrying = $state(false);
  let errDetails = $state(false);
  async function retryConn(): Promise<void> {
    retrying = true;
    try {
      await database.retryConnection();
    } finally {
      retrying = false;
    }
  }
  function dbMenu(e: MouseEvent): void {
    const items = database.isRedis
      ? database.keyspaces.map((k) => ({ label: k.label, checked: (database.activeDb ?? database.keyspaces[0]?.id) === k.id, action: () => database.setActiveDb(k.id) }))
      : [{ label: 'Default database', checked: !database.activeDb, action: () => database.setActiveDb(null) }, ...database.databaseNames.map((n) => ({ label: n, checked: database.activeDb === n, action: () => database.setActiveDb(n) }))];
    ctxMenu.show(e, items, { filter: items.length > 12, filterPlaceholder: 'Find a database' });
  }
  const dbChoice = $derived(database.isRedis ? database.keyspaces.length > 0 : !!database.capabilities?.sql && database.databaseNames.length > 0);
  const activeDbLabel = $derived(database.isRedis ? (database.keyspaces.find((k) => k.id === database.activeDb)?.label ?? database.keyspaces[0]?.label ?? '') : (database.activeDb ?? 'default database'));

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

  async function showSchemaSidebar(): Promise<void> {
    const page = document.querySelector('.db-page');
    database.toggleSidebar();
    await tick();
    const selected = [...(page?.querySelectorAll<HTMLElement>('[data-node-id]') ?? [])]
      .find((node) => node.dataset.nodeId === database.selectedObjectPath);
    (selected?.querySelector<HTMLButtonElement>('.node-label') ?? selected
      ?? page?.querySelector<HTMLInputElement>('[aria-label="Find an object"]'))?.focus();
  }

  // The connection list is its own page now.
  function showConnections(): void {
    router.go('connections');
  }
  function editConnection(c: Connection): void {
    editingConn = c; connFormOpen = true;
  }

  $effect(() =>
    registry.register('connections', [
      { id: 'conn.list', title: 'Show the connection list', group: 'Connections', keywords: 'picker sidebar profiles', run: showConnections },
      { id: 'conn.sidebar', title: database.sidebarCollapsed ? 'Show the schema sidebar' : 'Hide the schema sidebar', group: 'Connections', shortcut: '⌘B', keywords: 'toggle collapse tree', run: () => (database.sidebarCollapsed ? void showSchemaSidebar() : database.toggleSidebar()) },
    ]),
  );
  async function onConnSaved(c: Connection): Promise<void> {
    connFormOpen = false;
    connectionList.upsert(c, true);
    await database.loadConnections();
    // The edit may have fixed (or broken) the open tab's connection: connect again.
    if (database.selectedConnId === c.id) void database.retryConnection(c.id);
  }
  function editCluster(cl: BrokerCluster): void { editingCluster = cl; clusterFormOpen = true; }
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
        { title: 'Remove cluster', confirmLabel: 'Remove', danger: true },
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
   *  DB kinds. `reconnect` (the pane's Reconnect / Restart) always spawns a
   *  fresh session into the existing tab. */
  async function openTerminal(c: Connection, reconnect = false): Promise<void> {
    // Already open and still running → just focus it (don't spawn a 2nd
    // session). A tab whose session ENDED reconnects instead of focusing it.
    if (!reconnect && sshTabs.action(c.id) === 'focus') {
      database.focusSsh(c.id);
      return;
    }
    if (opening[c.id]) return; // a double-click on Reconnect must not spawn two
    const wsId = ws.currentId;
    if (!wsId) {
      toasts.error(NO_WORKSPACE, 'The database session is attached to a workspace — create or pick one.');
      return;
    }
    opening[c.id] = true;
    announce(`Opening ${c.name}…`);
    try {
      // In place as a workbench tab (like the DB kinds); ssh-tabs.svelte.ts.
      await sshTabs.open(c, wsId);
    } catch (e) {
      toastError('Couldn’t open', e);
    } finally {
      opening[c.id] = false;
    }
  }

  // Load connections when the workspace changes. The workbench itself is
  // GLOBAL — open connection tabs survive workspace switches. Restore the
  // persisted workbench (open tabs + focus) ONLY after loadConnections
  // resolves — the restore filters against the loaded list.
  $effect(() => {
    if (ws.currentId) {
      void (async () => {
        await database.loadConnections();
        await database.restoreWorkbench();
        connsSettled = true;
      })();
      void brokers.ensure(ws.currentId); // Kafka tabs (the list page has usually read them)
    }
  });
  // Saved queries load when they are shown (the Saved tab) or needed (a tab
  // opened from one: its Save updates it in place) — not with every open.
  let savedFor: string | null = null;
  $effect(() => {
    const wid = ws.currentId;
    if (!wid || savedFor === wid) return;
    if (database.sideTab !== 'saved' && !database.tabs[database.activeTab]?.savedQueryId) return;
    savedFor = wid;
    void database.loadSavedQueries();
  });
  // Dashboards (and their widgets) load when the Dashboards view opens, not
  // with every workbench visit.
  let dashboardsFor: string | null = null;
  $effect(() => {
    const wid = ws.currentId;
    if (database.mainTab !== 'dashboards' || !wid || dashboardsFor === wid) return;
    dashboardsFor = wid;
    void database.loadDashboards();
  });

  /** The first connection load + workbench restore has finished, so "no tab
   *  open" is real, not "not restored yet". */
  let connsSettled = $state(false);

  // `#/database/<id>` (a row on the Connections page, a pop-out window, a
  // link) opens that connection: a database's workbench tab, an SSH/custom
  // terminal tab, or a Kafka cluster tab.
  const deepId = $derived(router.module === 'database' ? (router.parts[1] ?? '').split('?')[0] : '');
  // The deep id already opened (or found gone): once its tab closes, the
  // id left in the URL must not hold the page on "restoring" for good.
  let deepTried = $state('');
  $effect(() => {
    const id = deepId;
    if (!connsSettled || !id) return;
    untrack(() => {
      deepTried = '';
      // Only the link still in the URL: an older, slower open settling last must not claim it.
      void openDeep(id).finally(() => { if (id === deepId) deepTried = id; });
    });
  });
  // …and the URL follows the open connection (a tab switch), so a reload or
  // a copied link comes back to it, not to the one the page was opened on.
  $effect(() => {
    const id = database.activePane === null ? database.selectedConnId : null;
    if (!id || !isPage || !connsSettled || id === deepId || (deepId && deepTried !== deepId)) return;
    untrack(() => router.replace(`database/${id}`));
  });
  async function openDeep(id: string): Promise<void> {
    if (database.connections.some((c) => c.id === id)) {
      connectionList.stampOpened(id);
      await database.openConnection(id);
      return;
    }
    const other = database.otherConnections.find((c) => c.id === id);
    if (other) return openTerminal(other);
    const cl = brokers.clusters.find((c) => c.id === id) ?? (ws.currentId ? (await brokers.load(ws.currentId, { withSections: false }), brokers.clusters.find((c) => c.id === id)) : undefined);
    if (cl) openCluster(cl);
  }

  const SIDE_TABS: TabItem<DbSideTab>[] = [
    { id: 'schema', label: 'Tables' },
    { id: 'saved', label: 'Saved' },
    { id: 'history', label: 'History' },
  ];
  // ── DB Assistant split (resizable, persisted) ────────────────────────────────
  // When open, the DB Assistant panel sits BESIDE the editor/results, separated by
  // a draggable divider so the user can enlarge the agent's shell. Mirrors the
  // query-editor's own resizable-pane idiom (pointer drag + localStorage px).
  let assistW = $state(loadAssistW());
  function loadAssistW(): number {
    const v = Number(lsGet('db.assistW'));
    return Number.isFinite(v) && v > 280 ? v : 460;
  }
  function persistAssistW(): void {
    lsSet('db.assistW', String(Math.round(assistW)));
  }
  const assistMaxW = (): number => Math.max(320, (typeof window !== 'undefined' ? window.innerWidth : 1280) - 360);
  const pxText = (v: number): string => `${Math.round(v)} pixels wide`;
  function setAssistW(w: number): void {
    assistW = w;
    persistAssistW();
  }
  let stopAssistResize: (() => void) | null = null;
  $effect(() => () => stopAssistResize?.());
  function startAssistResize(e: PointerEvent): void {
    e.preventDefault(); stopAssistResize?.();
    const startX = e.clientX, startW = assistW, maxW = assistMaxW();
    const onMove = (ev: PointerEvent): void => { assistW = Math.max(300, Math.min(maxW, startW + (startX - ev.clientX))); };
    const onUp = (): void => {
      persistAssistW();
      window.removeEventListener('pointermove', onMove); window.removeEventListener('pointerup', onUp); window.removeEventListener('pointercancel', onUp);
      if (stopAssistResize === onUp) stopAssistResize = null;
    };
    stopAssistResize = onUp;
    window.addEventListener('pointermove', onMove); window.addEventListener('pointerup', onUp); window.addEventListener('pointercancel', onUp);
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
  // Nothing open (fresh start, a restore that found nothing, the last tab
  // closed, a link to a connection that is gone): the workbench has nothing
  // to show — go to the connection list. Only as the page itself: the same
  // workbench docked beside Agents shows an empty state instead of taking
  // the window away from Agents.
  // (While the next page's chunk loads the route still reads "database": a
  // pending move elsewhere — "Open beside agents" — is not this page.)
  const isPage = $derived(router.module === 'database' && (router.pendingTarget?.[0] ?? 'database') === 'database');
  $effect(() => {
    if (connsSettled && isPage && !hasAnyTab && (!deepId || deepTried === deepId)) untrack(() => router.replace('connections'));
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
  /** The header title: the open database, cluster or terminal. */
  const workbenchTitle = $derived(
    activeCluster?.name ?? activeSsh?.name ?? openConns.find((c) => c.id === database.selectedConnId)?.name ?? 'Connections',
  );
  /** The profile behind an SSH/custom tab (null once it was deleted). */
  const sshConn = (id: string): Connection | null => database.otherConnections.find((c) => c.id === id) ?? database.connections.find((c) => c.id === id) ?? null;
  const editableConn = (c: Connection | null) => (c && connectionAccess(c, 'configure', 'admin') ? editConnection : undefined);
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
<!-- Three rows of chrome: this header (what is open · its state in words ·
     Tables · History · ⋯ · Run), the query tabs, the editor; then the
     results. The back button goes to the connection list. -->
<PageHeader title={workbenchTitle} more={headerMenu}>
  {#snippet leading()}
    <button class="icon-btn" onclick={showConnections} aria-label="All connections" title="All connections">
      <Icon name="chevronLeft" size={16} />
    </button>
  {/snippet}
  {#snippet badge()}
    {#if database.selectedConn && database.activePane === null}
      <span class="db-conn-state" data-testid="db-conn-state" title={connState.tip}>
        <span class="db-word {connState.tone}">{connState.word}</span>
        {#if connState.detail}<span class="db-sub ellipsis">{connState.tone === 'danger' ? '—' : '·'} {connState.detail}</span>{/if}
        {#if connState.tone === 'danger' && !viewport.isPhone}
          <!-- Can't connect: this one line says it; the raw error and the ways
               to fix it (Diagnose, Edit connection) are behind Details. (A
               phone's header has no room: the details card shows under it.) -->
          <button class="chip" disabled={retrying} onclick={() => void retryConn()}>{retrying ? 'Retrying…' : 'Retry'}</button><!-- ui-guards: allow — the header's one "Can't connect" line (no LoadState there) -->
          <button class="chip" aria-expanded={errDetails} onclick={() => (errDetails = !errDetails)}>Details</button>
        {/if}
        {#if database.isGuarded}
          <span class="db-sub" class:db-prod={database.isProd} data-testid="db-guard-chip">· {database.isProd ? 'Production' : 'Read-only'}</span>
        {/if}
        {#if dbChoice}
          <button class="db-link" onclick={dbMenu} aria-haspopup="menu" title="Active database — queries run scoped to it">· {activeDbLabel}</button>
        {/if}
        {#if database.mainTab !== 'query'}
          <button class="chip" onclick={() => database.setMainTab('query')} aria-label="Back to the query editor (showing {viewLabel})">{viewLabel} <Icon name="x" size={12} /></button>
        {/if}
        {#if editorBar.cur?.masked}
          <button class="chip" onclick={() => editorBar.cur?.unmask()} aria-label="Masked results — turn masking off">Masked <Icon name="x" size={12} /></button>
        {/if}
      </span>
    {/if}
  {/snippet}
  {#snippet actions()}
    {#if database.selectedConn && database.activePane === null}
      <!-- One way to the side pane: while it is open its own tabs are Tables ·
           Saved · History; while it is hidden (and on a phone, where it is a
           sheet) these two open it on that tab. -->
      {#if viewport.isPhone || database.sidebarCollapsed}
        <button class="btn small ghost" data-keep onclick={() => showSide('schema')} title={viewport.isPhone ? 'Tables' : 'Tables (⌘B)'}>Tables</button>
        <button class="btn small ghost" data-overflow="1" onclick={() => showSide('history')} title="Query history and saved queries">History</button>
      {/if}
      {#if database.mainTab === 'query' && editorBar.cur}
        {#if editorBar.cur.running}
          <button class="btn small qe-stop" data-keep onclick={() => editorBar.cur?.stop()} title="Stop the running query (Esc)"><Icon name="stop" size={12} /> Stop</button>
        {:else}
          <button
            class="btn small primary"
            disabled={!editorBar.cur.canRun || database.activeConnStatus?.phase === 'error'}
            title={database.activeConnStatus?.phase === 'error' ? `Can’t connect — ${connState.detail || 'see the error below'}` : editorBar.cur.runTitle}
            onclick={() => editorBar.cur?.run()}
          ><Icon name="play" size={12} /> Run <span class="kbd">⌘↵</span></button>
        {/if}
      {/if}
    {/if}
  {/snippet}
</PageHeader>
<PageBody fill padded={false}>
<div class="db-page" class:terminal-active={database.activePane?.kind === 'ssh'}>
  {#if !viewport.isPhone}
  <aside
    id="db-side"
    class="db-side"
    class:collapsed={!viewport.isPhone && database.sidebarCollapsed}
    style={viewport.isPhone || database.sidebarCollapsed ? '' : `width:${sideW}px`}
  >
      <!-- TABLET / DESKTOP: one tab strip over the schema tree, Saved and History. -->
      <div class="side-switch">
        <Tabs label="Sidebar view" tabs={SIDE_TABS} value={database.sideTab === 'connections' ? 'schema' : database.sideTab} onchange={(id) => database.setSideTab(id)}>
          {#snippet trailing()}
            {#if database.sideTab === 'schema' && database.selectedConnId}
              <button class="icon-btn" onclick={() => database.refreshSchema()} title="Refresh schema" aria-label="Refresh schema"><Icon name="refresh" size={12} /></button>
            {/if}
          {/snippet}
        </Tabs>
      </div>
      <div class="side-body">
        {#if database.selectedConnId}
          {@render schemaSideBody()}
        {:else}
          <div class="side-empty">
            <EmptyState icon="file" title="No connection open" body="Open a database connection to browse its tables." />
          </div>
        {/if}
      </div>
  </aside>
  {/if}

  {#if !viewport.isPhone && !database.sidebarCollapsed}
    <PaneDivider bind:width={sideW} storageKey="db.sideW" label="Resize connections sidebar" max={sideMaxW()} defaultWidth={SIDE_DEFAULT} />
  {/if}

  <div class="db-main" class:danger-rail={database.isProd} class:guard-rail={database.isGuarded && !database.isProd}>
    {#if !hasAnyTab && connsSettled && !isPage}
      <EmptyState icon="db" title="No connection open" body="Open one from the connection list." actionLabel="Open connections" onaction={showConnections} />
    {:else if !hasAnyTab}
      <!-- Restoring the open tabs, or about to go to the connection list. -->
      <LoadState what="the workbench" loading empty variant="page" />
    {:else}
      <!-- Unified tab strip: DB connections, Kafka clusters, and SSH/custom terminals -->
      <!-- Each tab is the main <button role="tab">; the status glyph, the ⋯ menu
           (the right-click menu, reachable without a mouse) and the close button
           are its siblings inside a presentational wrapper. -->
      <!-- One open database: no strip (the header names it; ⋯ closes it). A
           terminal or Kafka tab keeps its strip (its ended mark, its close). -->
      {#if openConns.length + brokers.openClusters.length + database.sshTabs.length > 1 || openConns.length === 0}
      <div class="conn-tabs" role="tablist" aria-label="Open connections" tabindex="-1" onkeydown={onTabKey} bind:this={connTabsEl}><!-- ui-guards: allow — closable document tabs (status glyph, ⋯ menu, close per tab): <Tabs> has no slot for per-tab controls -->
        {#each openConns as c (c.id)}
          {@const st = database.connStatus.get(c.id)}
          {@const on = database.activePane === null && database.selectedConnId === c.id}
          {@const stText = st?.phase === 'connecting' ? 'Connecting…' : st?.phase === 'error' ? (st.error ? `Couldn’t connect: ${st.error}` : 'Couldn’t connect — the server gave no reason') : st?.phase === 'idle' ? 'Not connected yet' : ''}
          <div use:rowMenu class="conn-tab" class:active={on} class:prod={isProdConn(c)} class:guarded={isGuardedConn(c) && !isProdConn(c)} role="presentation" oncontextmenu={(e) => { e.preventDefault(); connMenu(e, c); }}>
            <!-- The status glyphs below are decorative; the tab carries their state as its description. -->
            <button class="conn-tab-main" role="tab" aria-selected={on} aria-describedby={stText ? `conn-st-${c.id}` : undefined} tabindex={connTabStop(on, `db:${c.id}`)} onclick={() => database.openConnection(c.id)} title="{c.name} — right-click to open beside agents">
              <span class="conn-tab-glyph {c.kind}"><Icon name={engineGlyph(c.kind)} size={12} /></span>
              <span class="conn-tab-name ellipsis">{c.name}</span>
              {#if envBadge(c)}<EnvBadge env={c.environment} readOnly={c.read_only} />{/if}
            </button>
            {#if stText}<span id="conn-st-{c.id}" class="sr-only">{stText}</span>{/if}
            {#if st?.phase === 'connecting'}
              <span class="conn-tab-spin spinner" style="--spinner-size: 10px" aria-hidden="true" title={database.isWarming(c.id) ? 'Connecting in the background… (right-click to stop)' : 'Connecting…'} data-testid="conn-tab-connecting"></span>
            {:else if st?.phase === 'error' && !on}
              <!-- In words, never a bare dot; the open tab's header says it already. -->
              <span class="conn-tab-word" aria-hidden="true" title={st.error}>Can’t connect</span>
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
          {@const sc = sshConn(s.connId)}
          {@const ended = sshTabs.liveness(s.sessionId) === 'exited'}
          <div class="conn-tab" class:active={on} class:prod={!!sc && isProdConn(sc)} role="presentation" data-testid="ssh-tab">
            <button class="conn-tab-main" role="tab" aria-selected={on} aria-describedby={ended ? `ssh-st-${s.connId}` : undefined} tabindex={connTabStop(on, `ssh:${s.connId}`)} onclick={() => database.focusSsh(s.connId)} title={s.name}>
              <span class="conn-tab-glyph {s.kind}"><Icon name={engineGlyph(s.kind)} size={12} /></span>
              <span class="conn-tab-name ellipsis">{s.name}</span>
              {#if sc && envBadge(sc)}<EnvBadge env={sc.environment} readOnly={sc.read_only} />{/if}
            </button>
            {#if ended}<span id="ssh-st-{s.connId}" class="sr-only">Session ended</span><span class="conn-tab-word" aria-hidden="true" title="Session ended — click the connection or Reconnect to start a new one" data-testid="ssh-tab-ended">Ended</span>{/if}
            <button class="conn-tab-close" onclick={(e) => { e.stopPropagation(); void sshTabs.close(s.connId); }} aria-label="Close terminal tab" title="Close terminal tab">
              <Icon name="x" size={12} />
            </button>
          </div>
        {/each}
      </div>
      {/if}

      {#if database.activePane?.kind === 'kafka'}
        {#if activeCluster}
          <LazyMount lazy={ClusterViewerLazy} what="the Kafka cluster viewer" props={{ cluster: activeCluster, onEdit: editCluster, onRemove: (c: BrokerCluster) => void deleteCluster(c) }} />
        {:else}
          <EmptyState icon="box" title="Cluster closed" body="This Kafka cluster tab is no longer open." />
        {/if}
      {:else if database.activePane?.kind === 'ssh'}
        {#if activeSsh}
          {@const sc = sshConn(activeSsh.connId)}
          <SshPane tab={activeSsh} conn={sc} onreconnect={() => sc && void openTerminal(sc, true)} onedit={editableConn(sc)} />
        {:else}
          <EmptyState icon="terminal" title="Session closed" body="This terminal tab is no longer open." />
        {/if}
      {:else if database.selectedConnId}

      <!-- The active view (editor/results/…) and, when open, the DB Assistant
           agent panel side-by-side, separated by a draggable, persisted divider. -->
      <div class="main-split" class:assist-open={database.assistOpen}>
        <div class="main-body">
          {#if errDetails || viewport.isPhone}<DbConnErrorBanner onedit={editableConn(database.selectedConn)} onretry={viewport.isPhone ? retryConn : undefined} {retrying} />{/if}
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
        <EmptyState icon="db" title="Pick a tab" body="Choose an open connection above." />
      {/if}
    {/if}
  </div>
  <DiagnoseAside editFor={editableConn} />
</div>
</PageBody>
</div>

{#snippet schemaSideBody()}
  {#if database.sideTab === 'saved' || database.sideTab === 'history'}
    <DbSavedHistorySidebar mode={database.sideTab} />
  {:else}
    <!-- The header names the server; the tree starts right away. -->
    <SchemaTree />
  {/if}
{/snippet}

{#if viewport.isPhone && phoneSide && database.selectedConnId}
  <Modal title={phoneSide === 'history' ? 'History' : phoneSide === 'saved' ? 'Saved' : 'Tables'} onclose={() => (phoneSide = null)}>
    <div class="side-switch">
      <Tabs label="Sidebar view" tabs={SIDE_TABS} value={database.sideTab === 'connections' ? 'schema' : database.sideTab} onchange={(id) => { database.setSideTab(id); phoneSide = id; }} />
    </div>
    <div class="phone-side">{@render schemaSideBody()}</div>
  </Modal>
{/if}

{#if changesOpen && database.selectedConnId}<Modal title="Database changes" width={1040} onclose={() => changesOpen=false}><LazyMount lazy={DatabaseChangesLazy} what="database changes" variant="panel" props={{ connectionId: database.selectedConnId, node: database.activeDb }} /></Modal>{/if}

{#if accessFor}<Modal title={`Access · ${accessFor.name}`} width={820} onclose={() => accessFor=null}><LazyMount lazy={ResourceAccessLazy} what="access settings" variant="panel" props={{ kind: 'connection', resourceId: accessFor.id }} /></Modal>{/if}
{#if processesFor}<LazyMount lazy={ProcessListLazy} what="the process list" quiet onerror={() => (processesFor = null)} props={{ conn: processesFor, onclose: () => (processesFor = null) }} />{/if}

{#if connFormOpen && editingConn}
  {#key editingConn.id}
    <LazyMount lazy={ConnectionFormLazy} what="the connection form" quiet onerror={() => (connFormOpen = false)} props={{
      existing: editingConn, kinds: ALL_KINDS, onclose: () => { connFormOpen = false; }, onsaved: onConnSaved,
      ondiagnose: (p: { message: string; hint?: string | null; detail?: string | null }) => { const c = editingConn; connFormOpen = false; if (c) connDiagnose.request(c, p); },
    }} />
  {/key}
{/if}

{#if clusterFormOpen}
  <LazyMount lazy={ClusterFormLazy} what="the cluster form" quiet props={{ cluster: editingCluster, onclose: () => (clusterFormOpen = false) }} />
{/if}

<!-- File → table import dialog (launched from the schema-tree "Import into…"
     action or the results-grid toolbar). Keyed so it remounts fresh each open. -->
{#if database.importDialogOpen && database.selectedConnId}
  {#key database.importTable}
    <LazyMount lazy={ImportDialogLazy} what="the import dialog" quiet onerror={() => (database.importDialogOpen = false)} />
  {/key}
{/if}

<!-- "Export all rows…" an agent asked for (otto.ui_db_export): prefilled, but
     the person picks the folder + file and confirms. Closing without exporting
     reports `exported:false` back to the agent. -->
{#if database.exportRequest}
  {@const req = database.exportRequest}
  <LazyMount lazy={ExportDialogLazy} what="the export dialog" quiet props={{
    statement: req.statement,
    connectionId: req.connId,
    node: req.node,
    canExport: resourceAccess.can('connection', req.connId, 'db_export', 'database', 'view', databaseAccessChild(req.node ?? undefined)),
    initialFormat: req.format,
    initialMaxRows: req.maxRows,
    requestedBy: req.agentLabel,
    ondone: (r: { local_path: string; rows: number; bytes: number }) => {
      req.done({ exported: true, path: r.local_path, rows: r.rows, bytes: r.bytes });
    },
    onclose: () => {
      if (database.exportRequest === req) database.exportRequest = null;
      req.done({ exported: false });
    },
  }} />
{/if}

<style>
  .db-conn-state {
    display: inline-flex;
    align-items: center;
    gap: var(--sp-2);
    min-width: 0;
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .db-word {
    font-weight: 600;
  }
  .db-word.success {
    color: var(--success);
  }
  .db-word.danger {
    color: var(--danger);
  }
  .db-sub {
    min-width: 0;
  }
  .db-prod {
    color: var(--danger);
    font-weight: 600;
  }
  .kbd {
    font-size: var(--fs-xs);
    opacity: 0.7;
    font-variant-numeric: tabular-nums;
  }
  .qe-stop {
    font-weight: 600;
  }
  .db-link {
    border: 0;
    padding: 0;
    background: none;
    color: var(--accent-text);
    font: inherit;
    cursor: pointer;
  }
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
  /* On tablet/desktop the list lives inside the scrollable .side-body tab, so it
     fills the full sidebar height (no cap) and the side-body owns the scroll. */
  .side-body .conn-list {
    padding: 0;
    border-bottom: none;
    overflow-y: visible;
  }
  .conn-empty,
  /* Schema-tab empty state with its way-forward buttons. */
  .side-empty {
    display: flex;
    flex-direction: column;
    gap: 4px;
    padding: 4px 2px;
  }
  .link {
    border: none;
    background: none;
    color: var(--accent-text);
    cursor: pointer;
    font-size: var(--fs-s);
    padding: 0;
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
  .side-switch {
    flex-shrink: 0;
  }
  .side-body {
    flex: 1;
    overflow-y: auto;
    overflow-x: hidden;
    padding: 8px;
    min-height: 0;
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
  .conn-tab-spin {
    color: var(--text-dim);
    margin-inline-start: 4px;
    flex-shrink: 0;
  }
  .conn-tab-word {
    color: var(--danger);
    font-size: var(--fs-xs);
    margin-inline-start: 4px;
    white-space: nowrap;
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

  /* The DB Assistant pane — fixed (resizable) width, pinned to the right edge. */
  .assist-pane {
    flex: none;
    min-width: 0;
    min-height: 0;
    display: flex;
    border-inline-start: 1px solid var(--border);
  }
  .ellipsis {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  /* ───────────────── Phone (≤640px) ─────────────────
     The results are the screen: the view fills the height under the header
     (the query tabs, the editor folded to one line after a run, then the rows
     down to the bottom bar); a view taller than that (Structure, Builder)
     scrolls inside it. Tables and History are a sheet from the header. */
  @media (max-width: 640px) {
    .db-page {
      flex-direction: column;
      overflow: hidden;
    }
    .db-main {
      flex: 1;
      min-height: 0;
    }
    .main-body {
      padding: 0;
      overflow-x: hidden;
      overflow-y: auto;
      -webkit-overflow-scrolling: touch;
    }
    /* The assistant drops below the view as a full-width block. */
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
    .conn-tabs { height: 44px; }
  }
  .phone-side {
    max-height: 60vh;
    overflow-y: auto;
  }
  /* Finger-sized header controls on touch. */
  @media (hover: none), (pointer: coarse) {
    .db-link,
    .db-conn-state :global(.chip),
    .db-root :global(.page-header-bar .btn),
    .db-root :global(.page-header-bar .icon-btn) {
      min-block-size: 44px;
      min-inline-size: 44px;
    }
  }
  /* No keyboard hints on touch. */
  @media (hover: none), (pointer: coarse) {
    .kbd {
      display: none;
    }
  }
  /* Phone: the title keeps its room; the state is its word (the details are
     in the error banner / the ⋯). */
  @media (max-width: 640px) {
    .db-conn-state .db-sub,
    .db-conn-state .db-link {
      display: none;
    }
  }
  /* Tablet keeps two columns; cap restored sidebar width within viewport. */
  @media (min-width: 641px) and (max-width: 1024px) {
    .db-side { max-width: 45vw; }
    .main-tabs { height: auto; min-height: 44px; padding: 6px 12px; flex-wrap: wrap; row-gap: 4px; }
    .main-tabs .grow { flex-basis: 100%; height: 0; }
    .conn-status { flex-wrap: wrap; }
  }
  @media (max-height: 500px) and (pointer: coarse) {
    .db-side { max-width: min(32vw, 220px); }
    .main-tabs { min-height: 36px; padding: 4px 10px; }
  }
</style>
