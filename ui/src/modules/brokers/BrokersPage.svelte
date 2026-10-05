<script lang="ts">
  import Badge from '../../lib/components/Badge.svelte';
  import { toastError } from '../../lib/toastError';
  import { CLUSTER_VIEWS, type ClusterView } from './types';
  import Icon from '../../lib/components/Icon.svelte';
  import EnvBadge from '../../lib/components/EnvBadge.svelte';
  import PageHeader from '../../lib/components/PageHeader.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import { viewport } from '../../lib/stores/viewport.svelte';
  import { initialSelection, rememberSelection } from '../../lib/lastSelection';
  import { api } from '../../lib/api/client';
  import { brokers } from '../../lib/stores/brokers.svelte';
  import { brokersPagePort } from '../../lib/uiCommands/brokers';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import { ctxMenu } from '../../lib/contextmenu.svelte';
  import type { BrokerCluster, ConnectionSection, TestClusterResp } from '../../lib/api/types';
  import ClusterForm from './ClusterForm.svelte';
  import OverviewTab from './OverviewTab.svelte';
  import TopicsTab from './TopicsTab.svelte';
  import GroupsTab from './GroupsTab.svelte';
  import SchemaTab from './SchemaTab.svelte';
  import ReplayPanel from './ReplayPanel.svelte';
  import LagAlertsPanel from './LagAlertsPanel.svelte';
  import PaneDivider from '../../lib/components/PaneDivider.svelte';
  import PageBody from '../../lib/components/PageBody.svelte';
  import { LIST_PANE, loadPaneWidth } from '../../lib/paneResizer';
  import { onTabKey } from '../../lib/tabKeys';
  import { router } from '../../lib/router.svelte';
  import { untrack } from 'svelte';

  type Tab = ClusterView;
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

  // ---- resizable cluster sidebar --------------------------------------------
  // Drag the divider between the cluster list and the content to resize it; the
  // chosen width survives reloads. Mirrors the Database page's sidebar resizer.
  // (On phones the list is a full-width stacked band — the width var is ignored
  // by the media query there and the divider is hidden.)
  let sideW = $state(loadPaneWidth('brokers.sideW', LIST_PANE.default, LIST_PANE.min, LIST_PANE.max));
  // Tablet/desktop: the header `sidebar` icon hides the list pane (the one
  // collapse affordance, as on Database / Canvas / Skills Lab). Phone keeps its
  // stacked accordions.
  let listHidden = $state(false);

  $effect(() => {
    const id = ws.currentId;
    if (id) void brokers.load(id);
  });

  $effect(() => {
    // reset to overview when the selected cluster changes
    void brokers.selectedId;
    tab = 'overview';
  });
  // Agent UI control (lib/uiCommands/brokers.ts) switches the sub-tab here.
  $effect(() => brokersPagePort.bind({ setTab: (v) => (tab = v) }));

  const selected = $derived(brokers.selected);

  // The URL carries the open cluster (`#/brokers/<id>`) so a reload / share /
  // link lands on it. Route → page: a link (or Back/Forward) opens that cluster
  // once the list knows it; page → route: picking a cluster rewrites the URL in
  // place.
  $effect(() => {
    const [mod, id] = router.parts;
    if (mod !== 'brokers' || !id || !brokers.clusters.some((c) => c.id === id)) return;
    untrack(() => {
      if (id !== brokers.selectedId) brokers.select(id);
    });
  });
  let routedId: string | null = null;
  $effect(() => {
    const id = brokers.selectedId;
    const was = routedId;
    routedId = id;
    if (!id && !was) return;
    untrack(() => {
      if (router.module === 'brokers' && (router.parts[1] ?? null) !== id) router.replace(id ? `brokers/${id}` : 'brokers');
    });
  });

  // Never show an empty "pick a cluster" pane when clusters exist: open the
  // last-selected cluster (or the first) on load, and again whenever the last
  // open tab is closed. Not on a phone — selecting collapses the cluster list
  // there, which is the first screen.
  let autoPickedFor = $state<string | null>(null);
  $effect(() => {
    const wsId = ws.currentId;
    if (!wsId || viewport.isPhone || brokers.selectedId) return;
    if (brokers.loading || brokers.clusters.length === 0) return;
    const first = autoPickedFor !== wsId;
    autoPickedFor = wsId;
    const id = first
      ? initialSelection('brokers', brokers.clusters, (c) => c.id)
      : [...brokers.clusters].sort(byName)[0]?.id;
    if (id) untrack(() => brokers.select(id));
  });
  $effect(() => {
    if (brokers.selectedId) rememberSelection('brokers', brokers.selectedId);
  });
  // Nothing to list (no clusters, no sections): hide the list pane — the one
  // page-level empty state owns the page and its "Add a cluster" CTA.
  const isEmpty = $derived(!brokers.loading && !brokers.loadError && brokers.clusters.length === 0 && brokers.sections.length === 0);


  async function testConn(c: BrokerCluster) {
    testing = true;
    try {
      const r = await api.post<TestClusterResp>(`/brokers/clusters/${c.id}/test`, {});
      if (r.ok) toasts.success('Connected', `${r.message} · ${r.latency_ms}ms`);
      else toasts.error('Couldn’t connect', r.message);
    } catch (e) {
      toastError('Couldn’t run the test', e);
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
      toastError('Couldn’t remove', e);
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
    sec: ConnectionSection;
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
  // Right-click context menu (cluster row or section header) — routed through
  // the global ctxMenu overlay, which clamps into the viewport.
  function openMenu(e: MouseEvent, kind: 'cluster' | 'section', id: string): void {
    if (kind === 'cluster') {
      const c = brokers.clusters.find((x) => x.id === id);
      if (!c) return;
      ctxMenu.show(e, [
        { label: 'Open in tab', action: () => brokers.select(c.id) },
        { label: 'Test', action: () => void testConn(c) },
        { label: 'Edit…', action: () => openEdit(c) },
        { label: 'Remove…', icon: 'trash', danger: true, action: () => void removeCluster(c) },
      ]);
    } else {
      const s = brokers.sections.find((x) => x.id === id);
      if (!s) return;
      ctxMenu.show(e, [
        { label: 'New sub-section', action: () => void newSection(s.id) },
        { label: 'Rename…', action: () => void renameSec(s) },
        { label: 'Delete…', danger: true, action: () => void delSec(s) },
      ]);
    }
  }

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
      toastError('Couldn’t create the section', e);
    }
  }

  async function renameSec(sec: ConnectionSection): Promise<void> {
    const name = await confirmer.promptText('Rename section', {
      title: 'Rename section',
      confirmLabel: 'Rename',
      initial: sec.name,
    });
    if (!name || name === sec.name) return;
    try {
      await brokers.renameSection(sec.id, name);
    } catch (e) {
      toastError('Couldn’t rename', e);
    }
  }

  async function delSec(sec: ConnectionSection): Promise<void> {
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
      toastError('Couldn’t delete', e);
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
      toastError('Couldn’t move', e);
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
      toastError('Couldn’t move', e);
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

<div class="brokers-root">
<PageHeader
  class="cluster-head"
  title={selected?.name ?? 'Message Brokers'}
  subtitle={selected?.bootstrap_servers}
>
  {#snippet leading()}
    {#if !viewport.isPhone && !isEmpty}
      <button
        class="icon-btn"
        onclick={() => (listHidden = !listHidden)}
        aria-label={listHidden ? 'Show cluster list' : 'Hide cluster list'}
        title={listHidden ? 'Show cluster list' : 'Hide cluster list'}
        aria-expanded={!listHidden}
        aria-controls="brokers-cluster-list"
      >
        <Icon name="sidebar" size={16} />
      </button>
    {/if}
    {#if selected}
      <button
        class="content-toggle"
        onclick={() => (contentOpen = !contentOpen)}
        aria-expanded={contentOpen}
        aria-label={contentOpen ? 'Collapse details' : 'Expand details'}
        title={contentOpen ? 'Collapse details' : 'Expand details'}
      >
        <Icon name={contentOpen ? 'chevronDown' : 'chevronRight'} size={14} />
      </button>
      <span class="dot" style="background: {selected.color || 'var(--accent)'}"></span>
    {/if}
  {/snippet}
  {#snippet badge()}
    {#if selected}
      <EnvBadge env={selected.environment} />
      {#if selected.read_only}<Badge variant="outline" label="Read-only" />{/if}
      {#if selected.ssh}
        <Badge tone={tunnelReady ? 'ok' : 'neutral'} dot live={!tunnelReady} title={tunnelReady ? 'SSH tunnel connected' : 'SSH tunnel warming…'} label={tunnelReady ? 'Tunnel' : 'Connecting…'} />
      {/if}
    {/if}
  {/snippet}
  {#snippet actions()}
    {#if selected}
      <button class="btn small danger" data-overflow="-1" onclick={() => removeCluster(selected)} title="Remove this cluster profile from Otto (topics on the broker are untouched)">Remove…</button>
      <button class="btn small" onclick={() => openEdit(selected)}>Edit</button>
      <button class="btn small" data-keep onclick={() => testConn(selected)} disabled={testing}>
        {testing ? 'Testing…' : 'Test'}
      </button>
    {/if}
  {/snippet}
</PageHeader>
<PageBody fill padded={false}>
<div class="brokers-page">
  {#if !isEmpty && (viewport.isPhone || !listHidden)}
  <aside id="brokers-cluster-list" class="clusters" class:collapsed={!clustersOpen} style="--clusters-w:{sideW}px">
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
        <button class="icon-btn" onclick={() => newSection(null)} aria-label="New section" title="New section">
          <Icon name="folder" size={13} />
        </button>
        <button class="icon-btn" onclick={openAdd} aria-label="Add cluster" title="Add cluster"><Icon name="plus" size={13} /></button>
      </div>
    </div>
    <div class="cluster-list">
      <!-- A failed load is an error with Retry, never "No clusters yet"; the first
           load is a skeleton. A failed refresh keeps the list + a stale bar. -->
      <LoadState
        what="clusters"
        variant="compact"
        loading={brokers.loading}
        error={brokers.loadError}
        empty={brokers.clusters.length === 0 && brokers.sections.length === 0}
        rows={3}
        onretry={() => ws.currentId && void brokers.load(ws.currentId)}
      >
        {#each tree as node (node.sec.id)}
          {@render sectionNode(node, 0)}
        {/each}

        <!-- Ungrouped doubles as the top-level / no-section drop target. With no
             sections at all it is just noise, so it only shows once a section
             exists (or while something is being dragged). -->
        {#if brokers.sections.length > 0 || draggedClusterId || draggedSectionId}
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
        {/if}
        {#each ungrouped as c (c.id)}
          {@render clusterRow(c, brokers.sections.length > 0 ? 1 : 0)}
        {/each}

      </LoadState>
    </div>
  </aside>

  <PaneDivider bind:width={sideW} storageKey="brokers.sideW" label="Resize cluster list" />
  {/if}

  <main class="cluster-main" class:collapsed={!contentOpen}>
    {#if brokers.openClusters.length > 0}
      <div class="tabstrip" role="tablist" aria-label="Open clusters">
        {#each brokers.openClusters as c (c.id)}
          <!-- The tab and its close are TWO real buttons, so the close control
               isn't nested inside an interactive role=tab. ←/→ move between
               the open clusters (roving tabindex). -->
          <div class="ctab" class:on={brokers.selectedId === c.id} role="presentation">
            <button
              class="ctab-main"
              role="tab"
              aria-selected={brokers.selectedId === c.id}
              tabindex={brokers.selectedId === c.id ? 0 : -1}
              title={c.name}
              onclick={() => brokers.select(c.id)}
              onkeydown={onTabKey}
            >
              <span class="dot" style="background: {c.color || 'var(--accent)'}"></span>
              <span class="ctab-name">{c.name}</span>
            </button>
            <button
              class="ctab-x"
              aria-label="Close {c.name}"
              title="Close tab"
              onclick={(e) => {
                e.stopPropagation();
                brokers.close(c.id);
              }}
            >
              <Icon name="x" size={12} />
            </button>
          </div>
        {/each}
      </div>
    {/if}
    {#if selected}
      <div class="tabs" role="tablist" aria-label="Kafka cluster views" tabindex="-1" onkeydown={onTabKey}>
        {#each CLUSTER_VIEWS as v (v.id)}
          <button class:on={tab === v.id} role="tab" aria-selected={tab === v.id} tabindex={tab === v.id ? 0 : -1} onclick={() => (tab = v.id)}>{v.label}</button>
        {/each}
      </div>

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
      {#if isEmpty}
        <EmptyState
          variant="page"
          icon="box"
          title="Connect a Kafka cluster"
          body="Browse topics, peek messages, inspect consumer-group lag, and watch broker CPU / RAM."
          actionLabel="Add a cluster"
          actionIcon="plus"
          onaction={openAdd}
        />
      {/if}
      <!-- No "pick a cluster" pane: with clusters present one is always open
           (see the auto-pick above); on a phone the list above IS the page. -->
    {/if}
  </main>
</div>
</PageBody>
</div>

{#snippet sectionNode(node: TreeNode, depth: number)}
  {@const isOpen = !collapsed[node.sec.id]}
  <!-- The whole header toggles (the caret button stays the keyboard/AT
       control); clicks on its own buttons don't. -->
  <!-- svelte-ignore a11y_no_static_element_interactions, a11y_click_events_have_key_events -->
  <div
    class="sec-head"
    onclick={(e) => {
      if (!(e.target as Element).closest('button')) collapsed[node.sec.id] = !collapsed[node.sec.id];
    }}
    class:drop={(draggedSectionId && draggedSectionId !== node.sec.id) || draggedClusterId}
    style="padding-inline-start: {depth * 14 + 6}px"
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
      aria-label={isOpen ? `Collapse ${node.sec.name}` : `Expand ${node.sec.name}`}
      aria-expanded={isOpen}
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
  <!-- The row drags; its name button opens the cluster and ⋯ holds the same
       menu as right-click (no button nested inside a button). -->
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div
    class="cluster"
    class:sel={brokers.selectedId === c.id}
    style="padding-inline-start: {depth * 14 + 6}px"
    draggable="true"
    ondragstart={(e) => {
      draggedClusterId = c.id;
      e.stopPropagation();
    }}
    ondragend={() => (draggedClusterId = null)}
    oncontextmenu={(e) => openMenu(e, 'cluster', c.id)}
  >
    <button class="cluster-open" onclick={() => brokers.select(c.id)} aria-current={brokers.selectedId === c.id ? 'true' : undefined} title={c.name}>
      <span class="dot" style="background: {c.color || 'var(--accent)'}"></span>
      <span class="cn">{c.name}</span>
    </button>
    <EnvBadge env={c.environment} />
    <button class="icon-btn cluster-more" aria-label={`Actions for ${c.name}`} title="Actions" onclick={(e) => openMenu(e, 'cluster', c.id)}>
      <Icon name="more" size={14} />
    </button>
  </div>
{/snippet}


{#if formOpen}
  <ClusterForm cluster={editTarget} onclose={() => (formOpen = false)} />
{/if}

<style>
  .brokers-root {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }
  .brokers-page {
    flex: 1;
    display: flex;
    min-height: 0;
  }
  .clusters {
    /* Default width; drag-resizable via the PaneDivider (persisted). The
       phone media query below overrides back to a full-width band. */
    width: var(--clusters-w, 280px);
    border-inline-end: 1px solid var(--border);
    display: flex;
    flex-direction: column;
    min-height: 0;
    flex: none;
  }
  .aside-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 12px 12px 8px;
  }
  .aside-head .title {
    font-size: var(--fs-s);
    text-transform: uppercase;
    letter-spacing: .06em;
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
    padding-block: 6px; padding-inline: 6px 8px;
    cursor: pointer;
    color: var(--text-dim);
    border-inline-start: 2px solid transparent;
    user-select: none;
  }
  .sec-head:hover {
    background: var(--hover);
  }
  .sec-head.drop {
    background: var(--accent-soft);
    border-inline-start-color: var(--accent);
  }
  .sec-head.plain {
    cursor: default;
    font-size: var(--fs-xs);
    text-transform: uppercase;
    letter-spacing: .06em;
    opacity: 0.8;
    margin-top: 4px;
  }
  .sec-name {
    font-size: var(--fs-s);
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
    font-size: var(--fs-xs);
    color: var(--text-dim);
    background: color-mix(in srgb, var(--text-dim) 14%, transparent);
    border-radius: var(--radius-m);
    padding: 0 6px;
    flex: none;
  }
  .cluster {
    width: 100%;
    padding-block: 4px;
    padding-inline-end: 6px;
    display: flex;
    align-items: center;
    gap: 6px;
    border-inline-start: 2px solid transparent;
  }
  .cluster-open {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 4px 6px;
    border: none;
    background: transparent;
    color: var(--text);
    font: inherit;
    text-align: start;
    cursor: pointer;
  }
  .cluster-more {
    flex-shrink: 0;
    opacity: 0;
  }
  .cluster:hover .cluster-more,
  .cluster:focus-within .cluster-more {
    opacity: 1;
  }
  @media (hover: none) {
    .cluster-more {
      opacity: 1;
    }
  }
  .cluster:hover {
    background: var(--hover);
  }
  .cluster.sel {
    background: var(--accent-soft);
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
    font-size: var(--fs-m);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
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
    gap: 6px;
    padding: 0 10px;
    border-inline-end: 1px solid var(--border);
    font-size: var(--fs-m);
    color: var(--text-dim);
    white-space: nowrap;
    border-top: 2px solid transparent;
  }
  .ctab:hover {
    background: var(--hover);
  }
  .ctab.on {
    color: var(--text);
    background: var(--bg);
    border-top-color: var(--accent);
  }
  .ctab-main {
    display: flex;
    align-items: center;
    gap: 6px;
    align-self: stretch;
    padding: 0;
    border: none;
    background: transparent;
    color: inherit;
    font: inherit;
    cursor: pointer;
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
    border-radius: var(--radius-s);
    padding: 2px;
    opacity: 0.6;
  }
  .ctab-x:hover {
    opacity: 1;
    background: var(--hover);
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
    font-size: var(--fs-m);
    border-bottom: 2px solid transparent;
    white-space: nowrap;
    flex: none;
  }
  .tabs button.on {
    color: var(--text);
    border-bottom-color: var(--accent);
  }
  /* The panels size themselves to the tab body (height: 100%) and scroll
     inside; the body itself scrolls only when a panel's guaranteed minimum (the
     topics grid, the message panes) doesn't fit — e.g. a phone in landscape —
     so those rows stay reachable instead of being clipped. No height query. */
  .tab-body {
    flex: 1;
    min-height: 0;
    overflow-x: hidden;
    overflow-y: auto;
  }
  .small {
    font-size: var(--fs-xs);
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
      font-size: var(--fs-l);
    }
    .sec-toggle .hcount {
      display: inline-block;
      font-size: var(--fs-xs);
      color: var(--text-dim);
      background: color-mix(in srgb, var(--text-dim) 14%, transparent);
      border-radius: var(--radius-m);
      padding: 1px 8px;
    }
    .head-btns .icon-btn {
      width: 36px;
      height: 36px;
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
      padding-block: 6px;
    }
    .cn {
      font-size: var(--fs-l);
    }
    .sec-name {
      font-size: var(--fs-l);
    }
    .sec-head {
      padding-block: 8px; padding-inline: 8px 10px;
    }
    .count {
      font-size: var(--fs-xs);
    }
    .sec-head.plain {
      font-size: var(--fs-s);
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
      font-size: var(--fs-l);
      padding: 0 12px;
    }
    /* The page header carries the phone-only content toggle. */
    .content-toggle {
      display: inline-flex;
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
      font-size: var(--fs-l);
      padding: 10px 12px;
      white-space: nowrap;
      flex: none;
    }
    /* The active tab's content is its own scroll region. */
    .tab-body {
      overflow-y: auto;
      -webkit-overflow-scrolling: touch;
    }
  }
</style>
