<script lang="ts">
  import Icon from '../../lib/components/Icon.svelte';
  import { api } from '../../lib/api/client';
  import { brokers } from '../../lib/stores/brokers.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import type { BrokerCluster, BrokerClusterSection, TestClusterResp } from '../../lib/api/types';
  import ClusterForm from './ClusterForm.svelte';
  import OverviewTab from './OverviewTab.svelte';
  import TopicsTab from './TopicsTab.svelte';
  import GroupsTab from './GroupsTab.svelte';
  import SchemaTab from './SchemaTab.svelte';
  import ReplayPanel from './ReplayPanel.svelte';
  import LagAlertsPanel from './LagAlertsPanel.svelte';

  type Tab = 'overview' | 'topics' | 'groups' | 'schema' | 'replay' | 'alerts';
  let tab = $state<Tab>('overview');
  let formOpen = $state(false);
  let editTarget = $state<BrokerCluster | null>(null);
  let testing = $state(false);

  // Mobile (≤640px): the cluster list and the cluster content stack vertically
  // and each is a collapsible, independently-scrollable section. These toggles
  // only affect the phone layout (the headers/carets are hidden on desktop).
  let clustersOpen = $state(true);
  let contentOpen = $state(true);
  // Auto-collapse the cluster list after picking a cluster on a phone so the
  // content gets the screen; expand it again when nothing is selected.
  $effect(() => {
    if (brokers.selectedId) clustersOpen = false;
    else clustersOpen = true;
  });

  $effect(() => {
    const id = ws.currentId;
    if (id) void brokers.load(id);
  });

  $effect(() => {
    // reset to overview when the selected cluster changes
    void brokers.selectedId;
    tab = 'overview';
  });

  const selected = $derived(brokers.selected);

  function envBadge(c: BrokerCluster): string {
    return c.environment === 'prod' ? 'prod' : c.environment === 'staging' ? 'stg' : 'dev';
  }

  async function testConn(c: BrokerCluster) {
    testing = true;
    try {
      const r = await api.post<TestClusterResp>(`/brokers/clusters/${c.id}/test`, {});
      if (r.ok) toasts.success('Connected', `${r.message} · ${r.latency_ms}ms`);
      else toasts.error('Connection failed', r.message);
    } catch (e) {
      toasts.error('Test failed', String(e));
    } finally {
      testing = false;
    }
  }

  async function removeCluster(c: BrokerCluster) {
    const ok = await confirmer.ask(
      `Remove cluster "${c.name}"? Topics on the broker are not touched.`,
      { title: 'Remove cluster', confirmLabel: 'Remove', danger: true },
    );
    if (!ok) return;
    try {
      await brokers.remove(c.id);
      toasts.success('Cluster removed');
    } catch (e) {
      toasts.error('Remove failed', String(e));
    }
  }

  // ---- warm tunnel on cluster select ----------------------------------------
  // When a cluster with an SSH tunnel is opened, fire the /test endpoint in the
  // background so the SOCKS proxy warms up and the tunnel pill shows quickly.
  let warmingId = $state<string | null>(null);
  let tunnelReady = $state(false);
  $effect(() => {
    const c = brokers.selected;
    tunnelReady = false;
    if (!c?.ssh) return;
    warmingId = c.id;
    const id = c.id;
    void api.post<TestClusterResp>(`/brokers/clusters/${id}/test`, {})
      .then((r) => {
        if (warmingId === id) tunnelReady = r.ok;
      })
      .catch(() => {
        // silent — tunnel pill stays grey until a real op succeeds
      });
  });

  function openEdit(c: BrokerCluster) {
    editTarget = c;
    formOpen = true;
  }
  function openAdd() {
    editTarget = null;
    formOpen = true;
  }

  // ---- sidebar sections (grouping tree) ------------------------------------
  interface TreeNode {
    sec: BrokerClusterSection;
    items: BrokerCluster[];
    children: TreeNode[];
  }
  const byName = (a: BrokerCluster, b: BrokerCluster) => a.name.localeCompare(b.name);

  function buildTree(parentId: string | null): TreeNode[] {
    return brokers.sections
      .filter((s) => (s.parent_id ?? null) === parentId)
      .sort((a, b) => a.position - b.position || a.name.localeCompare(b.name))
      .map((sec) => ({
        sec,
        items: brokers.clusters.filter((c) => c.section_id === sec.id).sort(byName),
        children: buildTree(sec.id),
      }));
  }
  const tree = $derived(buildTree(null));
  const knownSectionIds = $derived(new Set(brokers.sections.map((s) => s.id)));
  const ungrouped = $derived(
    brokers.clusters.filter((c) => !c.section_id || !knownSectionIds.has(c.section_id)).sort(byName),
  );

  let collapsed = $state<Record<string, boolean>>({});
  let draggedClusterId = $state<string | null>(null);
  let draggedSectionId = $state<string | null>(null);
  // Right-click context menu (cluster row or section header).
  let menu = $state<{ x: number; y: number; kind: 'cluster' | 'section'; id: string } | null>(null);

  function openMenu(e: MouseEvent, kind: 'cluster' | 'section', id: string): void {
    e.preventDefault();
    menu = { x: e.clientX, y: e.clientY, kind, id };
  }
  const menuCluster = $derived(
    menu?.kind === 'cluster' ? (brokers.clusters.find((c) => c.id === menu!.id) ?? null) : null,
  );
  const menuSection = $derived(
    menu?.kind === 'section' ? (brokers.sections.find((s) => s.id === menu!.id) ?? null) : null,
  );

  async function newSection(parentId: string | null): Promise<void> {
    const name = await confirmer.promptText(parentId ? 'Sub-section name' : 'Section name', {
      title: parentId ? 'New sub-section' : 'New section',
      confirmLabel: 'Create',
      placeholder: 'e.g. Production',
    });
    if (!name) return;
    try {
      await brokers.createSection(parentId, name);
    } catch (e) {
      toasts.error('Create section failed', e instanceof Error ? e.message : String(e));
    }
  }

  async function renameSec(sec: BrokerClusterSection): Promise<void> {
    const name = await confirmer.promptText('Rename section', {
      title: 'Rename section',
      confirmLabel: 'Rename',
      initial: sec.name,
    });
    if (!name || name === sec.name) return;
    try {
      await brokers.renameSection(sec.id, name);
    } catch (e) {
      toasts.error('Rename failed', e instanceof Error ? e.message : String(e));
    }
  }

  async function delSec(sec: BrokerClusterSection): Promise<void> {
    if (
      !(await confirmer.ask(
        `Delete section “${sec.name}”? Sub-sections are removed too and their clusters become ungrouped.`,
        { title: 'Delete section' },
      ))
    )
      return;
    try {
      await brokers.deleteSection(sec.id);
    } catch (e) {
      toasts.error('Delete failed', e instanceof Error ? e.message : String(e));
    }
  }

  function isDescendantOf(nodeId: string, ancestorId: string): boolean {
    let cur = brokers.sections.find((s) => s.id === nodeId);
    while (cur?.parent_id) {
      if (cur.parent_id === ancestorId) return true;
      cur = brokers.sections.find((s) => s.id === cur!.parent_id);
    }
    return false;
  }

  async function moveCluster(id: string, sectionId: string | null): Promise<void> {
    try {
      await brokers.moveCluster(id, sectionId);
    } catch (e) {
      toasts.error('Move failed', e instanceof Error ? e.message : String(e));
    }
  }

  async function reparent(id: string, parentId: string | null): Promise<void> {
    const sec = brokers.sections.find((s) => s.id === id);
    if (!sec || (sec.parent_id ?? null) === parentId) return;
    if (parentId && (parentId === id || isDescendantOf(parentId, id))) {
      toasts.error('Invalid move', 'Cannot nest a section inside itself');
      return;
    }
    try {
      await brokers.reparentSection(id, parentId);
    } catch (e) {
      toasts.error('Move failed', e instanceof Error ? e.message : String(e));
    }
  }

  function onSectionDrop(sectionId: string): void {
    if (draggedClusterId) {
      const id = draggedClusterId;
      draggedClusterId = null;
      void moveCluster(id, sectionId);
    } else if (draggedSectionId) {
      const src = draggedSectionId;
      draggedSectionId = null;
      void reparent(src, sectionId);
    }
  }
  function onRootDrop(): void {
    if (draggedClusterId) {
      const id = draggedClusterId;
      draggedClusterId = null;
      void moveCluster(id, null);
    } else if (draggedSectionId) {
      const src = draggedSectionId;
      draggedSectionId = null;
      void reparent(src, null);
    }
  }
</script>

<div class="brokers-page">
  <aside class="clusters" class:collapsed={!clustersOpen}>
    <div class="aside-head">
      <button
        class="sec-toggle"
        onclick={() => (clustersOpen = !clustersOpen)}
        aria-expanded={clustersOpen}
        title={clustersOpen ? 'Collapse clusters' : 'Expand clusters'}
      >
        <Icon name={clustersOpen ? 'chevronDown' : 'chevronRight'} size={13} />
        <span class="title">Clusters</span>
        {#if brokers.clusters.length > 0}<span class="hcount">{brokers.clusters.length}</span>{/if}
      </button>
      <div class="head-btns">
        <button class="btn small" onclick={() => newSection(null)} title="New section">
          <Icon name="folder" size={13} />
        </button>
        <button class="btn small" onclick={openAdd} title="Add cluster"><Icon name="plus" size={13} /></button>
      </div>
    </div>
    <div class="cluster-list">
      {#if brokers.loading && brokers.clusters.length === 0}
        <p class="muted pad">Loading…</p>
      {:else}
        {#each tree as node (node.sec.id)}
          {@render sectionNode(node, 0)}
        {/each}

        <!-- Ungrouped doubles as the top-level / no-section drop target. -->
        <!-- svelte-ignore a11y_no_static_element_interactions -->
        <div
          class="sec-head plain"
          class:drop={draggedClusterId || draggedSectionId}
          ondragover={(e) => {
            if (draggedClusterId || draggedSectionId) e.preventDefault();
          }}
          ondrop={(e) => {
            e.preventDefault();
            onRootDrop();
          }}
          title="Clusters with no section (drop here to remove from a section / make a section top-level)"
        >
          <span class="caret-spacer"></span>
          <span class="sec-name grow">Ungrouped</span>
          {#if ungrouped.length > 0}<span class="count">{ungrouped.length}</span>{/if}
        </div>
        {#each ungrouped as c (c.id)}
          {@render clusterRow(c, 1)}
        {/each}

        {#if brokers.clusters.length === 0 && brokers.sections.length === 0}
          <p class="muted pad small">No clusters yet. Add one to connect to Kafka.</p>
        {/if}
      {/if}
    </div>
  </aside>

  <main class="cluster-main" class:collapsed={!contentOpen}>
    {#if brokers.openClusters.length > 0}
      <div class="tabstrip">
        {#each brokers.openClusters as c (c.id)}
          <div
            class="ctab"
            class:on={brokers.selectedId === c.id}
            role="tab"
            tabindex="0"
            onclick={() => brokers.select(c.id)}
            onkeydown={(e) => e.key === 'Enter' && brokers.select(c.id)}
          >
            <span class="dot" style="background: {c.color || 'var(--accent)'}"></span>
            <span class="ctab-name">{c.name}</span>
            <button
              class="ctab-x"
              title="Close tab"
              onclick={(e) => {
                e.stopPropagation();
                brokers.close(c.id);
              }}
            >
              <Icon name="x" size={11} />
            </button>
          </div>
        {/each}
      </div>
    {/if}
    {#if selected}
      <header class="cluster-head">
        <button
          class="content-toggle"
          onclick={() => (contentOpen = !contentOpen)}
          aria-expanded={contentOpen}
          title={contentOpen ? 'Collapse details' : 'Expand details'}
        >
          <Icon name={contentOpen ? 'chevronDown' : 'chevronRight'} size={14} />
        </button>
        <div class="ch-title">
          <span class="dot" style="background: {selected.color || 'var(--accent)'}"></span>
          <span class="name">{selected.name}</span>
          <span class="env {selected.environment}">{selected.environment}</span>
          {#if selected.read_only}<span class="ro">read-only</span>{/if}
          {#if selected.ssh}
            <span class="tunnel-pill" class:ready={tunnelReady} title={tunnelReady ? 'SSH tunnel connected' : 'SSH tunnel warming…'}>
              <Icon name="zap" size={10} /> {tunnelReady ? 'Tunnel' : 'Connecting…'}
            </span>
          {/if}
          <span class="boot mono">{selected.bootstrap_servers}</span>
        </div>
        <div class="actions">
          <button class="btn small" onclick={() => testConn(selected)} disabled={testing}>
            {testing ? 'Testing…' : 'Test'}
          </button>
          <button class="btn small" onclick={() => openEdit(selected)}>Edit</button>
          <button class="btn small danger" onclick={() => removeCluster(selected)}>Remove</button>
        </div>
      </header>

      <nav class="tabs">
        <button class:on={tab === 'overview'} onclick={() => (tab = 'overview')}>Overview</button>
        <button class:on={tab === 'topics'} onclick={() => (tab = 'topics')}>Topics</button>
        <button class:on={tab === 'groups'} onclick={() => (tab = 'groups')}>Consumer Groups</button>
        <button class:on={tab === 'schema'} onclick={() => (tab = 'schema')}>Schema Registry</button>
        <button class:on={tab === 'replay'} onclick={() => (tab = 'replay')}>Replay</button>
        <button class:on={tab === 'alerts'} onclick={() => (tab = 'alerts')}>Lag Alerts</button>
      </nav>

      <div class="tab-body">
        {#key selected.id}
          {#if tab === 'overview'}
            <OverviewTab clusterId={selected.id} />
          {:else if tab === 'topics'}
            <TopicsTab cluster={selected} />
          {:else if tab === 'groups'}
            <GroupsTab cluster={selected} />
          {:else if tab === 'schema'}
            <SchemaTab cluster={selected} />
          {:else if tab === 'replay'}
            <ReplayPanel cluster={selected} />
          {:else if tab === 'alerts'}
            <LagAlertsPanel cluster={selected} />
          {/if}
        {/key}
      </div>
    {:else}
      <div class="empty">
        <Icon name="box" size={30} />
        <h3>Message Brokers</h3>
        <p>Connect a Kafka cluster to browse topics, peek messages, inspect consumer-group lag, and watch broker CPU / RAM.</p>
        <button class="btn primary" onclick={openAdd}>Add a cluster</button>
      </div>
    {/if}
  </main>
</div>

{#snippet sectionNode(node: TreeNode, depth: number)}
  {@const isOpen = !collapsed[node.sec.id]}
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div
    class="sec-head"
    class:drop={(draggedSectionId && draggedSectionId !== node.sec.id) || draggedClusterId}
    style="padding-left: {depth * 14 + 6}px"
    draggable="true"
    ondragstart={(e) => {
      draggedSectionId = node.sec.id;
      e.stopPropagation();
    }}
    ondragend={() => (draggedSectionId = null)}
    ondragover={(e) => {
      if (draggedClusterId || (draggedSectionId && draggedSectionId !== node.sec.id))
        e.preventDefault();
    }}
    ondrop={(e) => {
      e.preventDefault();
      e.stopPropagation();
      onSectionDrop(node.sec.id);
    }}
    oncontextmenu={(e) => openMenu(e, 'section', node.sec.id)}
  >
    <button
      class="caret"
      onclick={() => (collapsed[node.sec.id] = !collapsed[node.sec.id])}
      title={isOpen ? 'Collapse' : 'Expand'}
    >
      <Icon name={isOpen ? 'chevronDown' : 'chevronRight'} size={12} />
    </button>
    <Icon name="folder" size={13} />
    <span class="sec-name grow">{node.sec.name}</span>
    {#if node.items.length > 0}<span class="count">{node.items.length}</span>{/if}
  </div>
  {#if isOpen}
    {#each node.children as child (child.sec.id)}
      {@render sectionNode(child, depth + 1)}
    {/each}
    {#each node.items as c (c.id)}
      {@render clusterRow(c, depth + 1)}
    {/each}
  {/if}
{/snippet}

{#snippet clusterRow(c: BrokerCluster, depth: number)}
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div
    class="cluster"
    class:sel={brokers.selectedId === c.id}
    style="padding-left: {depth * 14 + 6}px"
    draggable="true"
    ondragstart={(e) => {
      draggedClusterId = c.id;
      e.stopPropagation();
    }}
    ondragend={() => (draggedClusterId = null)}
    onclick={() => brokers.select(c.id)}
    onkeydown={(e) => e.key === 'Enter' && brokers.select(c.id)}
    oncontextmenu={(e) => openMenu(e, 'cluster', c.id)}
    role="button"
    tabindex="0"
  >
    <span class="dot" style="background: {c.color || 'var(--accent)'}"></span>
    <span class="cn">{c.name}</span>
    <span class="env {c.environment}">{envBadge(c)}</span>
  </div>
{/snippet}

{#if menu}
  <button class="menu-backdrop" aria-label="Close menu" onclick={() => (menu = null)}></button>
  <div class="ctxmenu" style="left: {menu.x}px; top: {menu.y}px" role="menu">
    {#if menuCluster}
      {@const c = menuCluster}
      <button role="menuitem" onclick={() => { brokers.select(c.id); menu = null; }}>Open in tab</button>
      <button role="menuitem" onclick={() => { void testConn(c); menu = null; }}>Test</button>
      <button role="menuitem" onclick={() => { openEdit(c); menu = null; }}>Edit…</button>
      <button role="menuitem" class="danger" onclick={() => { void removeCluster(c); menu = null; }}>Remove</button>
    {:else if menuSection}
      {@const s = menuSection}
      <button role="menuitem" onclick={() => { void newSection(s.id); menu = null; }}>New sub-section</button>
      <button role="menuitem" onclick={() => { void renameSec(s); menu = null; }}>Rename…</button>
      <button role="menuitem" class="danger" onclick={() => { void delSec(s); menu = null; }}>Delete</button>
    {/if}
  </div>
{/if}

{#if formOpen}
  <ClusterForm cluster={editTarget} onclose={() => (formOpen = false)} />
{/if}

<style>
  .brokers-page {
    display: flex;
    height: 100%;
    min-height: 0;
  }
  .clusters {
    width: 220px;
    border-inline-end: 1px solid var(--border);
    display: flex;
    flex-direction: column;
    min-height: 0;
  }
  .aside-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 12px 12px 8px;
  }
  .aside-head .title {
    font-size: 12px;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--text-dim);
  }
  .head-btns {
    display: flex;
    gap: 4px;
  }
  .cluster-list {
    flex: 1;
    overflow: auto;
    padding-bottom: 8px;
    min-height: 0;
  }
  /* Section headers (folders) */
  .sec-head {
    display: flex;
    align-items: center;
    gap: 4px;
    padding: 6px 8px 6px 6px;
    cursor: grab;
    color: var(--text-dim);
    border-inline-start: 2px solid transparent;
    user-select: none;
  }
  .sec-head:hover {
    background: color-mix(in srgb, var(--text-dim) 6%, transparent);
  }
  .sec-head.drop {
    background: color-mix(in srgb, var(--accent) 12%, transparent);
    border-inline-start-color: var(--accent);
  }
  .sec-head.plain {
    cursor: default;
    font-size: 11px;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    opacity: 0.8;
    margin-top: 4px;
  }
  .sec-name {
    font-size: 12px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .grow {
    flex: 1;
    min-width: 0;
  }
  .caret {
    border: none;
    background: transparent;
    color: var(--text-dim);
    cursor: pointer;
    display: flex;
    align-items: center;
    padding: 0;
    width: 14px;
    flex: none;
  }
  .caret-spacer {
    width: 14px;
    flex: none;
  }
  .count {
    font-size: 10px;
    color: var(--text-dim);
    background: color-mix(in srgb, var(--text-dim) 14%, transparent);
    border-radius: 8px;
    padding: 0 6px;
    flex: none;
  }
  /* Right-click context menu */
  .menu-backdrop {
    position: fixed;
    inset: 0;
    z-index: 40;
    border: none;
    background: transparent;
    cursor: default;
  }
  .ctxmenu {
    position: fixed;
    z-index: 41;
    min-width: 150px;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-m, 8px);
    box-shadow: 0 8px 24px rgba(0, 0, 0, 0.25);
    padding: 4px;
    display: flex;
    flex-direction: column;
  }
  .ctxmenu button {
    text-align: start;
    border: none;
    background: transparent;
    color: var(--text);
    padding: 6px 10px;
    border-radius: var(--radius-s, 6px);
    cursor: pointer;
    font-size: 12.5px;
  }
  .ctxmenu button:hover {
    background: color-mix(in srgb, var(--text-dim) 12%, transparent);
  }
  .ctxmenu button.danger {
    color: var(--status-exited, #ff5f57);
  }
  .cluster {
    width: 100%;
    text-align: start;
    border: none;
    background: transparent;
    padding: 8px 12px;
    display: flex;
    align-items: center;
    gap: 8px;
    cursor: pointer;
    border-inline-start: 2px solid transparent;
  }
  .cluster:hover {
    background: color-mix(in srgb, var(--text-dim) 8%, transparent);
  }
  .cluster.sel {
    background: color-mix(in srgb, var(--accent) 14%, transparent);
    border-inline-start-color: var(--accent);
  }
  .dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    flex: none;
  }
  .cn {
    flex: 1;
    font-size: 13px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .env {
    font-size: 9px;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    padding: 1px 5px;
    border-radius: 4px;
    background: color-mix(in srgb, var(--text-dim) 16%, transparent);
    color: var(--text-dim);
  }
  .env.prod {
    background: color-mix(in srgb, var(--status-exited, #ff5f57) 22%, transparent);
    color: var(--status-exited, #ff5f57);
  }
  .cluster-main {
    flex: 1;
    display: flex;
    flex-direction: column;
    min-width: 0;
    min-height: 0;
  }
  .tabstrip {
    display: flex;
    align-items: stretch;
    gap: 1px;
    border-bottom: 1px solid var(--border);
    background: var(--surface);
    overflow-x: auto;
    min-height: 36px;
  }
  .ctab {
    display: flex;
    align-items: center;
    gap: 7px;
    padding: 0 10px;
    cursor: pointer;
    border-inline-end: 1px solid var(--border);
    font-size: 12.5px;
    color: var(--text-dim);
    white-space: nowrap;
    border-top: 2px solid transparent;
  }
  .ctab:hover {
    background: color-mix(in srgb, var(--text-dim) 8%, transparent);
  }
  .ctab.on {
    color: var(--text);
    background: var(--bg);
    border-top-color: var(--accent);
  }
  .ctab-name {
    max-width: 180px;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .ctab-x {
    display: flex;
    align-items: center;
    justify-content: center;
    border: none;
    background: transparent;
    color: var(--text-dim);
    cursor: pointer;
    border-radius: 3px;
    padding: 2px;
    opacity: 0.6;
  }
  .ctab-x:hover {
    opacity: 1;
    background: color-mix(in srgb, var(--text-dim) 18%, transparent);
  }
  .cluster-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 12px 16px;
    border-bottom: 1px solid var(--border);
    gap: 12px;
  }
  .ch-title {
    display: flex;
    align-items: center;
    gap: 10px;
    min-width: 0;
  }
  .ch-title .name {
    font-size: 15px;
    font-weight: 600;
  }
  .ch-title .ro {
    font-size: 10px;
    color: var(--status-exited, #ff5f57);
    border: 1px solid currentColor;
    border-radius: 4px;
    padding: 0 5px;
  }
  .tunnel-pill {
    display: inline-flex;
    align-items: center;
    gap: 3px;
    font-size: 10px;
    padding: 1px 6px;
    border-radius: 4px;
    background: color-mix(in srgb, var(--text-dim) 14%, transparent);
    color: var(--text-dim);
  }
  .tunnel-pill.ready {
    background: color-mix(in srgb, var(--status-working, #28c840) 18%, transparent);
    color: var(--status-working, #28c840);
  }
  .boot {
    font-size: 11px;
    color: var(--text-dim);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .actions {
    display: flex;
    gap: 6px;
    flex: none;
  }
  .tabs {
    display: flex;
    gap: 2px;
    padding: 6px 14px 0;
    border-bottom: 1px solid var(--border);
    /* The six tabs can be wider than the content pane on narrow desktop-layout
       widths (e.g. tablet portrait 834px, phone landscape). Scroll the strip
       horizontally inside itself rather than letting the last tabs jut off the
       right edge where they become unreachable. */
    overflow-x: auto;
    flex-wrap: nowrap;
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
    overflow: hidden;
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
  .empty p {
    max-width: 440px;
  }
  .mono {
    font-family: var(--font-mono);
  }
  .muted {
    color: var(--text-dim);
  }
  .pad {
    padding: 12px;
  }
  .small {
    font-size: 11px;
  }
  .btn.danger {
    color: var(--status-exited, #ff5f57);
  }

  /* Collapse toggles. On desktop the cluster-list toggle is a plain inert label
     (no caret / count chrome) and the content toggle is hidden entirely, so the
     desktop layout looks exactly as before. They light up only on phones. */
  .sec-toggle {
    display: flex;
    align-items: center;
    gap: 6px;
    border: none;
    background: transparent;
    color: var(--text-dim);
    padding: 0;
    cursor: default;
    min-width: 0;
  }
  .sec-toggle > :global(svg),
  .sec-toggle .hcount {
    display: none;
  }
  .content-toggle {
    display: none;
    border: none;
    background: transparent;
    color: var(--text-dim);
    padding: 2px;
    cursor: pointer;
    align-items: center;
    flex: none;
  }

  /* Short viewports (phones in landscape, ~430px tall) keep the desktop
     two-column layout (they're >640px wide) but the cluster header + tab strips
     leave the tab body very little room. Let the tab body scroll so panels with
     a guaranteed min-height (e.g. the topics grid) stay fully reachable instead
     of being clipped behind the sticky chrome. */
  @media (max-height: 600px) {
    .tab-body {
      overflow-y: auto;
      -webkit-overflow-scrolling: touch;
    }
  }

  @media (max-width: 640px) {
    /* Stack the cluster list above the content; each is its own collapsible,
       independently-scrollable section, and text gets bumped for readability.
       The page keeps its bounded height (the host clips), so each section
       scrolls inside itself rather than the whole page scrolling. */
    .brokers-page {
      flex-direction: column;
    }
    .clusters {
      width: 100%;
      border-inline-end: none;
      border-bottom: 1px solid var(--border);
      flex: none;
      min-height: 0;
    }
    .aside-head {
      padding: 12px 14px;
    }
    .sec-toggle {
      flex: 1;
      cursor: pointer;
      padding: 6px 0;
    }
    .sec-toggle > :global(svg) {
      display: inline-flex;
      flex: none;
    }
    .sec-toggle .title {
      font-size: 14px;
    }
    .sec-toggle .hcount {
      display: inline-block;
      font-size: 11px;
      color: var(--text-dim);
      background: color-mix(in srgb, var(--text-dim) 14%, transparent);
      border-radius: 9px;
      padding: 1px 8px;
    }
    .head-btns .btn.small {
      font-size: 13px;
      padding: 6px 8px;
    }
    /* Expanded: scroll within a capped height. Collapsed: hidden. */
    .cluster-list {
      max-height: 45vh;
      overflow-y: auto;
      flex: none;
    }
    .clusters.collapsed .cluster-list {
      display: none;
    }
    /* Bigger sidebar text + roomier tap targets. */
    .cluster {
      padding: 11px 14px;
    }
    .cn {
      font-size: 15px;
    }
    .env {
      font-size: 10px;
      padding: 2px 6px;
    }
    .sec-name {
      font-size: 14px;
    }
    .sec-head {
      padding: 8px 10px 8px 8px;
    }
    .count {
      font-size: 11px;
    }
    .sec-head.plain {
      font-size: 12px;
    }

    /* Content section. */
    .cluster-main {
      flex: 1;
      min-height: 0;
    }
    .tabstrip {
      min-height: 42px;
    }
    .ctab {
      font-size: 14px;
      padding: 0 12px;
    }
    /* The cluster header wraps so the name + bootstrap + actions never collide
       or clip off the right edge. */
    .content-toggle {
      display: inline-flex;
    }
    .cluster-head {
      flex-wrap: wrap;
      align-items: flex-start;
      padding: 12px 14px;
      gap: 8px 10px;
    }
    .ch-title {
      flex: 1 1 100%;
      flex-wrap: wrap;
      gap: 6px 8px;
      align-items: center;
    }
    .ch-title .name {
      font-size: 17px;
    }
    .ch-title .boot {
      flex: 1 1 100%;
      font-size: 12px;
      white-space: normal;
      word-break: break-all;
    }
    .env {
      font-size: 10px;
    }
    .actions {
      flex: 1 1 100%;
    }
    .actions .btn.small {
      flex: 1;
      font-size: 13px;
      padding: 8px 10px;
    }
    /* Collapsed content: keep only the header (with its caret). */
    .cluster-main.collapsed .tabstrip,
    .cluster-main.collapsed .tabs,
    .cluster-main.collapsed .tab-body {
      display: none;
    }
    /* Tabs scroll horizontally with comfortable tap targets. */
    .tabs {
      overflow-x: auto;
      flex-wrap: nowrap;
      padding: 6px 10px 0;
      gap: 4px;
      -webkit-overflow-scrolling: touch;
    }
    .tabs button {
      font-size: 14px;
      padding: 10px 12px;
      white-space: nowrap;
      flex: none;
    }
    /* The active tab's content is its own scroll region. */
    .tab-body {
      overflow-y: auto;
      -webkit-overflow-scrolling: touch;
    }
    .empty {
      padding: 28px 18px;
    }
    .empty p {
      font-size: 14px;
    }
  }
</style>
