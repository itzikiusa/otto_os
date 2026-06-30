<script lang="ts">
  // DB Explorer page (mirrors ApiPage): left sidebar = connection picker +
  // SchemaTree + a Saved/History switch; main = a tab strip (Query / Builder /
  // Structure / Dashboards) over the active view.
  import Icon from '../../lib/components/Icon.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import SchemaTree from './SchemaTree.svelte';
  import QueryEditor from './QueryEditor.svelte';
  import QueryBuilder from './QueryBuilder.svelte';
  import StructureView from './StructureView.svelte';
  import DiagramView from './DiagramView.svelte';
  import Dashboards from './Dashboards.svelte';
  import DbAssistantPanel from './DbAssistantPanel.svelte';
  import ConnectionForm from '../connections/ConnectionForm.svelte';
  import ConnectionImportDialog from '../connections/ConnectionImportDialog.svelte';
  import ImportDialog from './ImportDialog.svelte';
  import { database, engineGlyph, type DbMainTab } from '../../lib/stores/database.svelte';
  import { ws, DB_PANE_ID } from '../../lib/stores/workspace.svelte';
  import { viewport } from '../../lib/stores/viewport.svelte';
  import { api } from '../../lib/api/client';
  import { confirmer } from '../../lib/confirm.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { ctxMenu } from '../../lib/contextmenu.svelte';
  import { router } from '../../lib/router.svelte';
  import type { Connection, ConnectionKind, ConnectionSection } from '../../lib/api/types';

  // DB connections are created/managed here (hidden from the Connections page).
  const DB_KINDS: ConnectionKind[] = ['mysql', 'redis', 'mongodb', 'clickhouse'];
  let connFormOpen = $state(false);
  let editingConn = $state<Connection | null>(null);
  // Import connection profiles from other DB tools (MySQL Workbench / DBeaver /
  // DataGrip / NoSQLBooster) — the daemon reads each tool's config from disk.
  let connImportOpen = $state(false);

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
  function connMenu(e: MouseEvent, c: Connection): void {
    ctxMenu.show(e, [
      { label: 'Open beside agents (split)', icon: 'split', action: () => openConnInAgents(c) },
      { separator: true },
      { label: 'Edit', icon: 'edit', action: () => editConnection(c) },
      { label: 'Delete', icon: 'trash', danger: true, action: () => void deleteConnection(c) },
    ]);
  }

  // --- Section hierarchy (mirrors the Connections page tree) -----------------
  interface TreeNode {
    sec: ConnectionSection;
    items: Connection[];
    children: TreeNode[];
  }
  let sections = $state<ConnectionSection[]>([]);
  let collapsed = $state<Record<string, boolean>>({});
  let draggedConnId = $state<string | null>(null);
  let draggedSectionId = $state<string | null>(null);

  const sortByName = (a: Connection, b: Connection): number => a.name.localeCompare(b.name);

  // Build the section tree from the flat list; `parentId = null` is the root.
  function buildTree(parentId: string | null): TreeNode[] {
    return sections
      .filter((s) => (s.parent_id ?? null) === parentId)
      .sort((a, b) => a.position - b.position || a.name.localeCompare(b.name))
      .map((sec) => ({
        sec,
        items: database.connections.filter((c) => c.section_id === sec.id).sort(sortByName),
        children: buildTree(sec.id),
      }));
  }
  const tree = $derived(buildTree(null));
  // Known folder ids in the (global, shared) tree. A connection whose folder is
  // not among them falls back to Ungrouped, so connections never vanish.
  const knownSectionIds = $derived(new Set(sections.map((s) => s.id)));
  const ungrouped = $derived(
    database.connections
      .filter((c) => !c.section_id || !knownSectionIds.has(c.section_id))
      .sort(sortByName),
  );

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
    return database.connections
      .filter(
        (c) =>
          c.name.toLowerCase().includes(q) ||
          c.kind.toLowerCase().includes(q) ||
          connDesc(c).toLowerCase().includes(q) ||
          sectionPath(c).toLowerCase().includes(q),
      )
      .sort(sortByName);
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
      const removed = new Set<string>();
      const collect = (id: string): void => {
        removed.add(id);
        for (const s of sections) if (s.parent_id === id) collect(s.id);
      };
      collect(sec.id);
      sections = sections.filter((s) => !removed.has(s.id));
      // Locally drop the section_id of any connection that fell out.
      database.connections = database.connections.map((c) =>
        c.section_id && removed.has(c.section_id) ? { ...c, section_id: null } : c,
      );
    } catch (e) {
      toasts.error('Delete failed', e instanceof Error ? e.message : String(e));
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

  // A drop onto a section: a dragged connection files into it; a dragged
  // section nests under it. A drop onto the root zone reverses both.
  function onSectionDrop(sectionId: string): void {
    if (draggedConnId) {
      const c = database.connections.find((x) => x.id === draggedConnId);
      draggedConnId = null;
      if (c) void moveConn(c, sectionId);
    } else if (draggedSectionId) {
      const src = draggedSectionId;
      draggedSectionId = null;
      void reparentSection(src, sectionId);
    }
  }
  function onRootDrop(): void {
    if (draggedConnId) {
      const c = database.connections.find((x) => x.id === draggedConnId);
      draggedConnId = null;
      if (c) void moveConn(c, null);
    } else if (draggedSectionId) {
      const src = draggedSectionId;
      draggedSectionId = null;
      void reparentSection(src, null);
    }
  }

  function newConnection(): void {
    editingConn = null;
    connFormOpen = true;
  }
  function editConnection(c: Connection): void {
    editingConn = c;
    connFormOpen = true;
  }
  async function onConnSaved(c: Connection): Promise<void> {
    connFormOpen = false;
    await database.loadConnections();
    void database.openConnection(c.id);
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
      toasts.error('Delete failed', e instanceof Error ? e.message : String(e));
    }
  }

  // Load connections + workspace-scoped saved/dashboards when the workspace changes.
  $effect(() => {
    if (ws.currentId) {
      void database.loadConnections();
      void loadSections();
      void database.loadSavedQueries();
      void database.loadDashboards();
    }
  });

  const mainTabs: { id: DbMainTab; label: string; show: () => boolean }[] = [
    { id: 'query', label: 'Query', show: () => true },
    { id: 'builder', label: 'Builder', show: () => database.supportsBuilder },
    { id: 'structure', label: 'Structure', show: () => true },
    // ERD is table/collection-oriented; Redis (keys, no table model) is excluded.
    { id: 'diagram', label: 'Diagram', show: () => database.capabilities?.engine !== 'redis' },
    { id: 'dashboards', label: 'Dashboards', show: () => true },
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
  function startAssistResize(e: PointerEvent): void {
    e.preventDefault();
    const startX = e.clientX;
    const startW = assistW;
    const maxW = Math.max(320, (typeof window !== 'undefined' ? window.innerWidth : 1280) - 360);
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
  const SIDE_W_DEFAULT = 300;
  let sideW = $state(loadSideW());
  function loadSideW(): number {
    if (typeof localStorage === 'undefined') return SIDE_W_DEFAULT;
    const v = Number(localStorage.getItem('db.sideW'));
    return Number.isFinite(v) && v >= 220 ? v : SIDE_W_DEFAULT;
  }
  function persistSideW(): void {
    try {
      localStorage.setItem('db.sideW', String(Math.round(sideW)));
    } catch {
      /* storage unavailable — non-fatal */
    }
  }
  function startSideResize(e: PointerEvent): void {
    e.preventDefault();
    const startX = e.clientX;
    const startW = sideW;
    // Leave room for the editor/results area; cap so the sidebar can't eat the page.
    const maxW = Math.min(640, Math.max(360, (typeof window !== 'undefined' ? window.innerWidth : 1280) - 420));
    const onMove = (ev: PointerEvent): void => {
      // The sidebar is pinned to the LEFT edge, so dragging RIGHT widens it.
      sideW = Math.max(220, Math.min(maxW, startW + (ev.clientX - startX)));
    };
    const onUp = (): void => {
      persistSideW();
      window.removeEventListener('pointermove', onMove);
      window.removeEventListener('pointerup', onUp);
    };
    window.addEventListener('pointermove', onMove);
    window.addEventListener('pointerup', onUp);
  }
  function resetSideW(): void {
    sideW = SIDE_W_DEFAULT;
    persistSideW();
  }

  // Open connections as top-level tabs (Workbench-style), resolved to their
  // Connection records for name + engine glyph.
  const openConns = $derived(
    database.openConnIds
      .map((id) => database.connections.find((c) => c.id === id))
      .filter((c): c is NonNullable<typeof c> => c != null),
  );

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
  const isProdConn = (c: Connection): boolean => c.environment === 'prod';
  const isGuardedConn = (c: Connection): boolean => c.environment === 'prod' || c.read_only;
  // Short badge label, or '' when neither (dev/staging, not read-only).
  function envBadge(c: Connection): string {
    if (c.environment === 'prod') return 'PROD';
    if (c.read_only) return 'RO';
    if (c.environment === 'staging') return 'STG';
    return '';
  }
</script>

<div class="db-page">
  <aside class="db-side" style={viewport.isPhone ? '' : `width:${sideW}px`}>
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
          <button class="icon-btn" onclick={newConnection} aria-label="New connection" title="New connection"><Icon name="plus" size={13} /></button>
          <button class="icon-btn" onclick={() => (connImportOpen = true)} aria-label="Import connections" title="Import connections from MySQL Workbench, DBeaver, DataGrip or NoSQLBooster"><Icon name="arrowDown" size={13} /></button>
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
        <div class="side-switch" class:acc-collapsed={!schemaOpen} role="tablist">
          <button class="ss" class:active={database.sideTab === 'schema' || database.sideTab === 'connections'} role="tab" aria-selected={database.sideTab === 'schema'} onclick={() => (database.sideTab = 'schema')}>Schema</button>
          <button class="ss" class:active={database.sideTab === 'saved'} role="tab" aria-selected={database.sideTab === 'saved'} onclick={() => (database.sideTab = 'saved')}>Saved</button>
          <button class="ss" class:active={database.sideTab === 'history'} role="tab" aria-selected={database.sideTab === 'history'} onclick={() => (database.sideTab = 'history')}>History</button>
        </div>
        <div class="side-body" class:acc-collapsed={!schemaOpen}>
          {@render schemaSideBody()}
        </div>
      {/if}
    {:else}
      <!-- TABLET / DESKTOP: one tab strip. "Connections" is the picker tab, so
           the list takes the full sidebar height instead of a capped section. -->
      <div class="side-switch" role="tablist">
        <button class="ss" class:active={database.sideTab === 'connections'} role="tab" aria-selected={database.sideTab === 'connections'} onclick={() => (database.sideTab = 'connections')}>Connections</button>
        <button class="ss" class:active={database.sideTab === 'schema'} role="tab" aria-selected={database.sideTab === 'schema'} onclick={() => (database.sideTab = 'schema')}>Schema</button>
        <button class="ss" class:active={database.sideTab === 'saved'} role="tab" aria-selected={database.sideTab === 'saved'} onclick={() => (database.sideTab = 'saved')}>Saved</button>
        <button class="ss" class:active={database.sideTab === 'history'} role="tab" aria-selected={database.sideTab === 'history'} onclick={() => (database.sideTab = 'history')}>History</button>
        {#if database.sideTab === 'schema' && database.selectedConnId}
          <span class="grow"></span>
          <button class="icon-btn" onclick={() => database.refreshSchema()} title="Refresh schema" aria-label="Refresh schema"><Icon name="refresh" size={12} /></button>
        {/if}
      </div>
      <div class="side-body">
        {#if database.sideTab === 'connections'}
          <div class="conn-list">{@render connListBody()}</div>
        {:else if database.selectedConnId}
          {@render schemaSideBody()}
        {:else}
          <div class="list-empty">Open a connection to browse its schema.</div>
        {/if}
      </div>
    {/if}
  </aside>

  {#if !viewport.isPhone}
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div
      class="side-resizer"
      role="separator"
      aria-orientation="vertical"
      aria-label="Drag to resize the connections sidebar (double-click to reset)"
      title="Drag to resize · double-click to reset"
      ondblclick={resetSideW}
      onpointerdown={startSideResize}
    ></div>
  {/if}

  <div class="db-main" class:danger-rail={database.isProd} class:guard-rail={database.isGuarded && !database.isProd}>
    {#if !database.selectedConnId}
      <EmptyState
        icon="db"
        title="Pick a database connection"
        body={database.connections.length === 0
          ? 'No MySQL, Redis, MongoDB or ClickHouse connections in this workspace yet.'
          : 'Choose a connection on the left to browse its schema and run queries.'}
        actionLabel={database.connections.length === 0 ? 'New connection' : undefined}
        onaction={database.connections.length === 0 ? newConnection : undefined}
      />
    {:else}
      <!-- Top-level connection tabs (one per open connection) -->
      <div class="conn-tabs" role="tablist" aria-label="Open connections">
        {#each openConns as c (c.id)}
          {@const st = database.connStatus.get(c.id)}
          <div class="conn-tab" class:active={database.selectedConnId === c.id} class:prod={isProdConn(c)} class:guarded={isGuardedConn(c) && !isProdConn(c)} role="tab" tabindex="-1" aria-selected={database.selectedConnId === c.id} oncontextmenu={(e) => { e.preventDefault(); connMenu(e, c); }}>
            <button class="conn-tab-main" onclick={() => database.openConnection(c.id)} title="{c.name} — right-click to open beside agents">
              <span class="conn-tab-glyph {c.kind}"><Icon name={engineGlyph(c.kind)} size={12} /></span>
              {#if sectionLeaf(c)}<span class="conn-tab-path mono" title="Folder: {sectionPath(c)}">{sectionLeaf(c)}</span>{/if}
              <span class="conn-tab-name ellipsis">{c.name}</span>
              {#if envBadge(c)}<span class="env-badge mono" class:prod={isProdConn(c)}>{envBadge(c)}</span>{/if}
            </button>
            {#if st?.phase === 'connecting'}
              <span class="conn-tab-spin spin" title="Connecting…"><Icon name="refresh" size={10} /></span>
            {:else if st?.phase === 'error'}
              <span class="conn-tab-dot" title={st.error}></span>
            {/if}
            <button
              class="conn-tab-close"
              onclick={(e) => {
                e.stopPropagation();
                database.closeConnection(c.id);
              }}
              aria-label="Close connection tab"
              title="Close"
            >
              <Icon name="x" size={11} />
            </button>
          </div>
        {/each}
      </div>

      {#if database.isGuarded}
        <div class="guard-banner" class:prod={database.isProd}>
          <Icon name={database.isProd ? 'zap' : 'key'} size={13} />
          <span>
            {#if database.isProd}
              Production connection — writes &amp; schema changes require a typed confirmation.
            {:else}
              Read-only connection — writes &amp; schema changes require a typed confirmation.
            {/if}
          </span>
        </div>
      {/if}

      <div class="main-tabs">
        {#each visibleTabs as t (t.id)}
          <button class="mt" class:active={database.mainTab === t.id} role="tab" aria-selected={database.mainTab === t.id} onclick={() => (database.mainTab = t.id)}>
            {t.label}
          </button>
        {/each}
        <span class="grow"></span>
        <div class="conn-status">
          {#if database.capabilities}
            <span class="cap-chip mono" title="Engine">{database.capabilities.engine}</span>
          {/if}
          {#if database.activeConnStatus?.phase === 'connecting'}
            <span class="conn-state"><span class="conn-tab-spin spin"><Icon name="refresh" size={11} /></span>Connecting…</span>
          {:else if database.activeConnStatus?.phase === 'error'}
            <span class="conn-state err" title={database.activeConnStatus.error}>Disconnected</span>
          {/if}
          <button class="btn small ghost" onclick={() => database.testConnection()} disabled={database.testing}>
            <Icon name="plug" size={11} />{database.testing ? 'Testing…' : 'Test'}
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
          {#if database.mainTab === 'query'}
            <QueryEditor />
          {:else if database.mainTab === 'builder'}
            <QueryBuilder />
          {:else if database.mainTab === 'structure'}
            <StructureView />
          {:else if database.mainTab === 'diagram'}
            <DiagramView />
          {:else}
            <Dashboards />
          {/if}
        </div>
        {#if database.assistOpen}
          <!-- svelte-ignore a11y_no_static_element_interactions -->
          <div
            class="assist-divider"
            role="separator"
            aria-orientation="vertical"
            aria-label="Drag to resize the assistant"
            title="Drag to resize the assistant"
            onpointerdown={startAssistResize}
          ></div>
          <aside class="assist-pane" style="width:{assistW}px">
            <DbAssistantPanel />
          </aside>
        {/if}
      </div>
    {/if}
  </div>
</div>

{#snippet sectionNode(node: TreeNode, depth: number)}
  {@const isOpen = !collapsed[node.sec.id]}
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div
    class="sec-head"
    class:drop-target={(draggedSectionId && draggedSectionId !== node.sec.id) || draggedConnId}
    style="padding-inline-start: {depth * 14 + 2}px"
    draggable="true"
    ondragstart={(e) => {
      draggedSectionId = node.sec.id;
      e.stopPropagation();
    }}
    ondragend={() => (draggedSectionId = null)}
    ondragover={(e) => {
      if (draggedConnId || (draggedSectionId && draggedSectionId !== node.sec.id)) e.preventDefault();
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
    <Icon name="folder" size={12} />
    <span class="sec-name grow ellipsis">{node.sec.name}</span>
    <span class="count">{node.items.length}</span>
    <div class="sec-actions">
      <button class="icon-btn" title="Add sub-section" aria-label="Add sub-section" onclick={() => createSection(node.sec.id)}>
        <Icon name="plus" size={11} />
      </button>
      <button class="icon-btn" title="Rename section" aria-label="Rename section" onclick={() => renameSection(node.sec)}>
        <Icon name="edit" size={11} />
      </button>
      <button class="icon-btn" title="Delete section" aria-label="Delete section" onclick={() => deleteSection(node.sec)}>
        <Icon name="trash" size={11} />
      </button>
    </div>
  </div>
  {#if isOpen}
    {#each node.items as c (c.id)}
      {@render connRow(c, depth + 1)}
    {/each}
    {#each node.children as child (child.sec.id)}
      {@render sectionNode(child, depth + 1)}
    {/each}
  {/if}
{/snippet}

{#snippet connRow(c: Connection, depth: number)}
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div
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
    <button class="conn-item" onclick={() => database.openConnection(c.id)} title="{c.name} · {c.kind}{isProdConn(c) ? ' · PRODUCTION' : c.read_only ? ' · read-only' : ''} — right-click to open beside agents">
      <span class="conn-glyph {c.kind}"><Icon name={engineGlyph(c.kind)} size={12} /></span>
      <span class="conn-name">{c.name}</span>
      {#if envBadge(c)}<span class="env-badge mono" class:prod={isProdConn(c)}>{envBadge(c)}</span>{/if}
    </button>
    <div class="conn-actions">
      <button class="icon-btn" aria-label="Edit connection" title="Edit" onclick={() => editConnection(c)}>
        <Icon name="edit" size={11} />
      </button>
      <button class="icon-btn" aria-label="Delete connection" title="Delete" onclick={() => deleteConnection(c)}>
        <Icon name="trash" size={11} />
      </button>
    </div>
  </div>
{/snippet}

{#snippet connSearchBox()}
  <div class="tree-search">
    <Icon name="search" size={11} />
    <input
      class="tree-search-input"
      type="text"
      bind:value={connFilter}
      placeholder="Filter connections…"
      spellcheck="false"
      aria-label="Filter connections"
    />
    {#if connFilter}
      <button class="tree-search-clear" onclick={() => (connFilter = '')} aria-label="Clear filter"><Icon name="x" size={10} /></button>
    {/if}
    {#if !viewport.isPhone}
      <!-- New section / connection live here on tablet/desktop (the phone keeps
           them in the accordion header), so the tab strip never overflows. -->
      <button class="icon-btn" onclick={() => createSection(null)} aria-label="New section" title="New section"><Icon name="folder" size={12} /></button>
      <button class="icon-btn" onclick={newConnection} aria-label="New connection" title="New connection"><Icon name="plus" size={12} /></button>
      <button class="icon-btn" onclick={() => (connImportOpen = true)} aria-label="Import connections" title="Import connections from MySQL Workbench, DBeaver, DataGrip or NoSQLBooster"><Icon name="arrowDown" size={12} /></button>
    {/if}
  </div>
{/snippet}

{#snippet connListBody()}
  {@render connSearchBox()}
  {#if database.connections.length === 0 && sections.length === 0}
    <div class="conn-empty">
      No database connections.
      <button class="link" onclick={newConnection}>New connection →</button>
    </div>
  {:else if connFilter.trim()}
    {#if connMatches.length === 0}
      <div class="list-empty">No connections match “{connFilter.trim()}”.</div>
    {:else}
      {#each connMatches as c (c.id)}
        {@render connRow(c, 0)}
      {/each}
    {/if}
  {:else}
    {#each tree as node (node.sec.id)}
      {@render sectionNode(node, 0)}
    {/each}

    {#if sections.length > 0}
      <!-- Ungrouped doubles as the root / no-section drop target. -->
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <div
        class="sec-head plain"
        class:drop-target={draggedConnId || draggedSectionId}
        ondragover={(e) => {
          if (draggedConnId || draggedSectionId) e.preventDefault();
        }}
        ondrop={(e) => {
          e.preventDefault();
          onRootDrop();
        }}
        title="Drop here to remove from a section / make a section top-level"
      >
        <span class="caret-spacer"></span>
        <span class="sec-name grow">Ungrouped</span>
        {#if ungrouped.length > 0}<span class="count">{ungrouped.length}</span>{/if}
      </div>
      {#each ungrouped as c (c.id)}
        {@render connRow(c, 1)}
      {/each}
    {:else}
      {#each ungrouped as c (c.id)}
        {@render connRow(c, 0)}
      {/each}
    {/if}
  {/if}
{/snippet}

{#snippet schemaSideBody()}
  {#if database.sideTab === 'saved'}
    {#if database.savedQueries.length === 0}
      <div class="list-empty">No saved queries. Save one from the Query tab.</div>
    {:else}
      {#each database.savedQueries as q (q.id)}
        <div class="saved-row">
          <button class="saved-open" onclick={() => database.openSavedQuery(q)} title={q.statement}>
            <Icon name="file" size={12} />
            <span class="ellipsis">{q.name}</span>
          </button>
          <button class="icon-btn row-del" onclick={() => database.deleteSavedQuery(q.id)} aria-label="Delete saved query"><Icon name="trash" size={11} /></button>
        </div>
      {/each}
    {/if}
  {:else if database.sideTab === 'history'}
    {#if database.history.length === 0}
      <div class="list-empty">No query history yet.</div>
    {:else}
      {#each database.history as h (h.id)}
        <button class="hist-row" class:bad={!h.ok} onclick={() => database.openHistory(h)} title={h.error ?? h.statement}>
          <span class="hist-dot" class:ok={h.ok}></span>
          <span class="hist-stmt ellipsis mono">{h.statement}</span>
          <span class="hist-meta">{h.ok ? `${h.row_count}r` : 'err'} · {fmtAgo(h.created_at)}</span>
        </button>
      {/each}
    {/if}
  {:else}
    <SchemaTree />
  {/if}
{/snippet}

{#if connFormOpen}
  <ConnectionForm
    existing={editingConn}
    kinds={DB_KINDS}
    onclose={() => (connFormOpen = false)}
    onsaved={onConnSaved}
  />
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

<style>
  .db-page {
    height: 100%;
    display: flex;
    min-height: 0;
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
    gap: 5px;
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
    font-size: 11.5px;
    outline: none;
    min-width: 0;
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
  .list-empty {
    font-size: 11.5px;
    color: var(--text-dim);
    padding: 8px 6px;
    line-height: 1.5;
  }
  .link {
    border: none;
    background: none;
    color: var(--accent);
    cursor: pointer;
    font-size: 11.5px;
    padding: 0;
  }
  .conn-item {
    display: flex;
    align-items: center;
    gap: 6px;
    /* min-height (not a fixed height) so the row grows when a long name wraps. */
    min-height: 26px;
    padding: 3px 6px;
    border: none;
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--text);
    cursor: pointer;
    text-align: start;
  }
  .conn-item:hover {
    background: color-mix(in srgb, var(--text-dim) 10%, transparent);
  }
  .conn-row.open:not(.active) .conn-item {
    background: color-mix(in srgb, var(--text-dim) 7%, transparent);
  }
  .conn-row.active .conn-item {
    background: color-mix(in srgb, var(--accent) 15%, transparent);
  }
  .conn-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 6px 8px 2px;
  }
  .conn-head-title {
    font-size: 10.5px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--text-dim);
  }
  .head-btns {
    display: flex;
    align-items: center;
    gap: 1px;
  }
  /* --- Section hierarchy rows --- */
  .sec-head {
    display: flex;
    align-items: center;
    gap: 4px;
    height: 24px;
    padding: 0 6px;
    border-radius: var(--radius-s);
    cursor: grab;
    user-select: none;
    color: var(--text-dim);
  }
  .sec-head:hover {
    background: color-mix(in srgb, var(--text-dim) 10%, transparent);
  }
  .sec-head.plain {
    cursor: default;
    margin-top: 4px;
  }
  .sec-head.drop-target {
    outline: 1px dashed color-mix(in srgb, var(--accent) 55%, transparent);
    outline-offset: -1px;
    background: color-mix(in srgb, var(--accent) 8%, transparent);
  }
  .sec-name {
    font-size: 10.5px;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.04em;
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
  .count {
    font-size: 9.5px;
    color: var(--text-dim);
    min-width: 14px;
    text-align: center;
    font-variant-numeric: tabular-nums;
  }
  .sec-actions {
    display: flex;
    gap: 0;
    flex-shrink: 0;
    opacity: 0;
  }
  .sec-head:hover .sec-actions {
    opacity: 1;
  }
  .conn-row.dragging {
    opacity: 0.5;
  }
  .conn-row {
    display: flex;
    align-items: center;
    gap: 2px;
  }
  .conn-row .conn-item {
    flex: 1;
    min-width: 0;
  }
  .conn-actions {
    display: flex;
    gap: 1px;
    flex-shrink: 0;
    opacity: 0;
    padding-inline-end: 2px;
  }
  .conn-row:hover .conn-actions {
    opacity: 1;
  }
  .conn-glyph {
    display: grid;
    place-items: center;
    flex-shrink: 0;
    color: var(--text-dim);
  }
  .conn-glyph.mysql,
  .conn-glyph.clickhouse {
    color: var(--accent);
  }
  .conn-glyph.redis {
    color: #d2691e;
  }
  .conn-glyph.mongodb {
    color: var(--status-working);
  }
  .conn-row.active .conn-item .conn-glyph {
    color: var(--accent);
  }
  .conn-name {
    flex: 1;
    min-width: 0;
    font-size: 12px;
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
    padding: 0 7px;
    border: none;
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--text-dim);
    font-size: 11.5px;
    font-weight: 500;
    cursor: pointer;
  }
  .ss:hover {
    background: var(--surface-2);
  }
  .ss.active {
    background: color-mix(in srgb, var(--accent) 16%, transparent);
    color: var(--accent);
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
    gap: 7px;
    min-width: 0;
    border: none;
    background: transparent;
    color: var(--text);
    cursor: pointer;
    height: 28px;
    padding: 0 6px;
    border-radius: var(--radius-s);
    font-size: 12px;
  }
  .saved-row:hover,
  .hist-row:hover {
    background: color-mix(in srgb, var(--text-dim) 9%, transparent);
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
    font-size: 11px;
  }
  .hist-meta {
    font-size: 10px;
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
    padding: 7px 14px;
    font-size: 12px;
    line-height: 1.4;
    color: var(--status-working);
    background: color-mix(in srgb, var(--status-working) 12%, transparent);
    border-bottom: 1px solid color-mix(in srgb, var(--status-working) 35%, transparent);
  }
  .guard-banner.prod {
    color: var(--status-exited);
    background: color-mix(in srgb, var(--status-exited) 13%, transparent);
    border-bottom-color: color-mix(in srgb, var(--status-exited) 40%, transparent);
    font-weight: 600;
  }
  /* Environment badge on connection tabs / rows. */
  .env-badge {
    flex-shrink: 0;
    font-size: 8.5px;
    font-weight: 700;
    letter-spacing: 0.04em;
    padding: 1px 5px;
    border-radius: 999px;
    color: var(--status-working);
    background: color-mix(in srgb, var(--status-working) 16%, transparent);
  }
  .env-badge.prod {
    color: var(--status-exited);
    background: color-mix(in srgb, var(--status-exited) 16%, transparent);
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
    gap: 3px;
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
    font-size: 9px;
    text-transform: uppercase;
    letter-spacing: 0.03em;
    color: var(--accent);
    background: color-mix(in srgb, var(--accent) 14%, transparent);
    border-radius: 999px;
    padding: 1px 6px;
    flex-shrink: 0;
    white-space: nowrap;
  }
  .conn-tab {
    display: flex;
    align-items: center;
    height: 26px;
    padding: 0 3px 0 9px;
    border-radius: var(--radius-s);
    border: 1px solid transparent;
    color: var(--text-dim);
    cursor: pointer;
    white-space: nowrap;
    max-width: 320px;
    flex-shrink: 0;
    transition: background 120ms ease-out, color 120ms ease-out;
  }
  .conn-tab:hover {
    background: var(--surface-2);
  }
  .conn-tab.active {
    background: var(--surface);
    border-color: var(--border);
    color: var(--text);
  }
  .conn-tab-main {
    display: flex;
    align-items: center;
    gap: 7px;
    min-width: 0;
    border: none;
    background: transparent;
    color: inherit;
    cursor: pointer;
    font-size: 12.5px;
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
  .conn-tab-glyph.mysql,
  .conn-tab-glyph.clickhouse {
    color: var(--accent);
  }
  .conn-tab-glyph.redis {
    color: #d2691e;
  }
  .conn-tab-glyph.mongodb {
    color: var(--status-working);
  }
  .conn-tab.active .conn-tab-glyph {
    color: var(--accent);
  }
  .conn-tab-name {
    min-width: 0;
    max-width: 220px;
  }
  .conn-tab-close {
    display: grid;
    place-items: center;
    width: 17px;
    height: 17px;
    margin-inline-start: 5px;
    border: none;
    border-radius: 3px;
    background: transparent;
    color: var(--text-dim);
    cursor: pointer;
    opacity: 0;
    flex-shrink: 0;
    transition: opacity 120ms ease-out, background 120ms ease-out, color 120ms ease-out;
  }
  .conn-tab:hover .conn-tab-close,
  .conn-tab.active .conn-tab-close {
    opacity: 1;
  }
  .conn-tab-close:hover {
    background: color-mix(in srgb, var(--text-dim) 22%, transparent);
    color: var(--text);
  }
  .main-tabs {
    display: flex;
    align-items: center;
    gap: 2px;
    padding: 8px 14px 0;
    border-bottom: 1px solid var(--border);
  }
  .mt {
    height: 30px;
    padding: 0 13px;
    border: none;
    background: transparent;
    color: var(--text-dim);
    font-size: 12.5px;
    font-weight: 500;
    cursor: pointer;
    border-bottom: 2px solid transparent;
    margin-bottom: -1px;
  }
  .mt:hover {
    color: var(--text);
  }
  .mt.active {
    color: var(--accent);
    border-bottom-color: var(--accent);
  }
  .conn-status {
    display: flex;
    align-items: center;
    gap: 8px;
    padding-bottom: 4px;
  }
  .cap-chip {
    font-size: 10px;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--text-dim);
    background: var(--surface-2);
    padding: 1px 7px;
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
    gap: 5px;
    font-size: 11px;
    color: var(--text-dim);
  }
  .conn-state.err {
    color: var(--status-exited);
    font-weight: 500;
  }
  /* Component-scoped spinner (SchemaTree's copy doesn't leak here). */
  .spin {
    display: grid;
    place-items: center;
    animation: spin 0.9s linear infinite;
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
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
  .assist-divider:hover {
    background: var(--accent);
  }
  /* Draggable divider between the connections sidebar and the main area. Sits flush
     against the sidebar's inline-end border; a hit-area wider than its visible line
     makes it easy to grab. */
  .side-resizer {
    flex: none;
    width: 5px;
    margin-inline-start: -3px;
    cursor: col-resize;
    background: transparent;
    position: relative;
    z-index: 2;
    touch-action: none;
  }
  .side-resizer:hover {
    background: color-mix(in srgb, var(--accent) 45%, transparent);
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
      height: 100%;
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
      font-size: 12.5px;
    }
    .acc-count {
      font-size: 11px;
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
      font-size: 15px;
    }
    .conn-item {
      min-height: 40px;
    }
    .conn-head-title {
      font-size: 12px;
    }
    .conn-empty,
    .list-empty {
      font-size: 13.5px;
    }
    .hist-stmt {
      font-size: 13px;
    }
    .hist-meta,
    .count {
      font-size: 12px;
    }
    .saved-open {
      font-size: 14px;
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
      padding: 8px 12px 0;
      flex-wrap: wrap;
      row-gap: 4px;
    }
    /* The flexible spacer would push conn-status onto an overflowing line —
       make it a full-width break so the status wraps cleanly below the tabs. */
    .main-tabs .grow {
      flex-basis: 100%;
      height: 0;
    }
    .conn-status {
      padding-bottom: 8px;
    }
    .mt {
      height: 36px;
      font-size: 13.5px;
      padding: 0 12px;
    }
    .conn-name {
      font-size: 14px;
    }
    .ss {
      height: 30px;
      font-size: 13px;
    }
    /* The status row (engine chip + Test) can wrap rather than overflow. */
    .conn-status {
      flex-wrap: wrap;
    }
  }

  /* ───────────────── Tablet (641–1024px) ─────────────────
     The tablet keeps the desktop side-by-side layout (280px sidebar + main),
     but the narrowed main column can't fit the dense tab/status row on one
     line, so the engine chip + Test button get pushed past the
     (.content overflow:hidden) edge and become unreachable — even though the
     page itself never scrolls sideways. Let the tab row wrap and break the
     status onto its own line (the same treatment the phone layout uses), WITHOUT
     collapsing the columns. */
  @media (min-width: 641px) and (max-width: 1024px) {
    .main-tabs {
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
