<script lang="ts">
  import { rowMenu } from '../../lib/rowMenu';
  import Badge from '../../lib/components/Badge.svelte';
  import { toastError } from '../../lib/toastError';
  import { CLUSTER_VIEWS, type ClusterView, type ReplayPrefill } from './types';
  import Icon from '../../lib/components/Icon.svelte';
  import Tabs, { type TabItem } from '../../lib/components/Tabs.svelte';
  import EnvBadge from '../../lib/components/EnvBadge.svelte';
  import PageHeader from '../../lib/components/PageHeader.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
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
  import PageBody from '../../lib/components/PageBody.svelte';
  import ListDetail from '../../lib/components/side/ListDetail.svelte';
  import ListPane from '../../lib/components/side/ListPane.svelte';
  import { router } from '../../lib/router.svelte';
  import { registry } from '../../lib/commands.svelte';
  import { untrack } from 'svelte';

  type Tab = ClusterView;
  let tab = $state<Tab>('overview');
  let replayPrefill = $state<ReplayPrefill | null>(null);
  let replayPrefillSeq = 0;
  let formOpen = $state(false);
  let editTarget = $state<BrokerCluster | null>(null);
  let testing = $state(false), topicOpen = $state(false); // reading a topic: cluster actions move to ⋯

  function openReplayFrom(prefill: ReplayPrefill) {
    replayPrefill = { ...prefill, seq: ++replayPrefillSeq };
    tab = 'replay';
  }

  // Phone: the cluster list is the page; a picked cluster is pushed over it
  // ("‹ Clusters" in the page header goes back, the cluster stays marked).
  let phoneOpen = $state(false);
  function pick(id: string): void {
    brokers.select(id);
    phoneOpen = true;
  }
  // ⌘K verbs.
  $effect(() =>
    registry.register('brokers', [
      { id: 'brokers.add', title: 'Add a Kafka cluster…', group: 'Message Brokers', keywords: 'kafka broker new', run: openAdd },
      { id: 'brokers.refresh', title: 'Refresh Kafka clusters', group: 'Message Brokers', keywords: 'kafka reload', run: () => { if (ws.currentId) void brokers.load(ws.currentId); } },
      ...brokers.clusters.map((c) => ({
        id: `brokers.open.${c.id}`, title: `Open cluster ${c.name}`, group: 'Message Brokers', detail: c.environment, keywords: 'kafka broker topics groups',
        run: () => pick(c.id),
      })),
    ]),
  );

  $effect(() => {
    const id = ws.currentId;
    if (id) void brokers.load(id);
  });

  $effect(() => {
    // reset to overview when the selected cluster changes
    void brokers.selectedId;
    tab = 'overview';
    replayPrefill = null;
  });
  // Agent UI control (lib/uiCommands/brokers.ts) switches the sub-tab here.
  $effect(() => brokersPagePort.bind({ setTab: (v) => (tab = v) }));

  const selected = $derived(brokers.selected);
  const openClusterTabs = $derived<TabItem<string>[]>(
    brokers.openClusters.map((c) => ({ id: c.id, label: c.name, title: c.name })),
  );

  // The URL carries the open cluster (`#/brokers/<id>`) so a reload / share /
  // link lands on it. Route → page: a link (or Back/Forward) opens that cluster
  // once the list knows it; page → route: picking a cluster rewrites the URL in
  // place.
  $effect(() => {
    const [mod, id] = router.parts;
    if (mod !== 'brokers' || !id || !brokers.clusters.some((c) => c.id === id)) return;
    untrack(() => {
      if (id !== brokers.selectedId) pick(id);
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
  // open tab is closed. On a phone it is only marked: the list is the page
  // until a cluster is tapped.
  let autoPickedFor = $state<string | null>(null);
  $effect(() => {
    const wsId = ws.currentId;
    if (!wsId || brokers.selectedId) return;
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
        { label: 'Open in tab', action: () => pick(c.id) },
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
        { title: 'Delete section', confirmLabel: 'Delete', danger: true },
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

<div class="brokers-root brokers-page">
<PageHeader
  class="cluster-head"
  title={selected?.name ?? 'Message Brokers'}
  subtitle={selected?.bootstrap_servers} more={() => (selected && topicOpen ? [{ label: 'Test connection', action: () => void testConn(selected) }, { label: 'Edit…', action: () => openEdit(selected) }, { label: 'Remove…', icon: 'trash', danger: true, action: () => void removeCluster(selected) }] : [])}
>
  {#snippet leading()}
    {#if selected}<span class="dot" style="background: {selected.color || 'var(--accent)'}"></span>{/if}
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
    {#if selected && !topicOpen}
      <button class="btn small danger" data-overflow="-1" onclick={() => removeCluster(selected)} title="Remove this cluster profile from Otto (topics on the broker are untouched)">Remove…</button>
      <button class="btn small" onclick={() => openEdit(selected)}>Edit</button>
      <button class="btn small" data-keep onclick={() => testConn(selected)} disabled={testing}>{testing ? 'Testing…' : 'Test'}</button>
    {/if}
  {/snippet}
</PageHeader>
<PageBody fill padded={false}>
  <ListDetail
    module="brokers"
    name="Clusters"
    open={phoneOpen && !!selected}
    onback={() => (phoneOpen = false)}
    empty={isEmpty}
    legacy={{ w: 'brokers.sideW' }}
  >
    {#snippet list()}
      <ListPane
        count={brokers.clusters.length}
        primary={{ icon: 'plus', label: 'Add cluster', onclick: openAdd, testid: 'brokers-add-cluster' }}
        menu={() => [
          { label: 'New section…', icon: 'folder', action: () => void newSection(null) },
          { label: 'Refresh', icon: 'refresh', action: () => ws.currentId && void brokers.load(ws.currentId) },
        ]}
        loading={brokers.loading && brokers.clusters.length === 0 && brokers.sections.length === 0}
        error={brokers.clusters.length === 0 && brokers.sections.length === 0 ? brokers.loadError : null}
        onretry={() => ws.currentId && void brokers.load(ws.currentId)}
        testid="brokers-cluster-list"
      >
        <div class="cluster-list" role="tree" aria-label="Clusters">
          {#each tree as node (node.sec.id)}
            {@render sectionNode(node, 0)}
          {/each}

          <!-- Ungrouped doubles as the top-level / no-section drop target. With no
               sections at all it is just noise, so it only shows once a section
               exists (or while something is being dragged). -->
          {#if brokers.sections.length > 0 || draggedClusterId || draggedSectionId}
            <div
              role="group"
              aria-label="Ungrouped"
              class="side-section-head ungrouped"
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
              <span class="ssh-title">Ungrouped</span>
              {#if ungrouped.length > 0}<span class="ssh-count">{ungrouped.length}</span>{/if}
            </div>
          {/if}
          {#each ungrouped as c (c.id)}
            {@render clusterRow(c, 0)}
          {/each}
        </div>
      </ListPane>
    {/snippet}

    <main class="cluster-main">
      {#if brokers.openClusters.length > 0}
        <div class="tabstrip">
          <Tabs
            label="Open clusters"
            tabs={openClusterTabs}
            value={brokers.selectedId ?? ''}
            onchange={(id) => brokers.select(id)}
            idBase="brokers-open"
          >
            {#snippet trailing()}
              {#if selected}
                <button
                  class="icon-btn"
                  aria-label="Close {selected.name}"
                  title="Close {selected.name}"
                  onclick={() => brokers.close(selected.id)}
                >
                  <Icon name="x" size={12} />
                </button>
              {/if}
            {/snippet}
          </Tabs>
        </div>
      {/if}
      {#if selected}
        <div class="tabs">
          <Tabs label="Kafka cluster views" tabs={CLUSTER_VIEWS} value={tab} onchange={(id) => (tab = id)} idBase="brokers-view" />
        </div>

        <div class="tab-body" role="tabpanel" id="brokers-view-panel-{tab}" aria-labelledby="brokers-view-tab-{tab}">
          {#key selected.id}
            {#if tab === 'overview'}
              <OverviewTab clusterId={selected.id} />
            {:else if tab === 'topics'}
              <TopicsTab cluster={selected} onreplayfrom={openReplayFrom} ontopic={(open) => (topicOpen = open)} />
            {:else if tab === 'groups'}
              <GroupsTab cluster={selected} />
            {:else if tab === 'schema'}
              <SchemaTab cluster={selected} />
            {:else if tab === 'replay'}
              <ReplayPanel cluster={selected} prefill={replayPrefill} />
            {:else if tab === 'alerts'}
              <LagAlertsPanel cluster={selected} />
            {/if}
          {/key}
        </div>
      {:else if isEmpty}
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
           (see the auto-pick above); on a phone the list IS the page. -->
    </main>
  </ListDetail>
</PageBody>
</div>

{#snippet sectionNode(node: TreeNode, depth: number)}
  {@const isOpen = !collapsed[node.sec.id]}
  {@const toggle = () => (collapsed[node.sec.id] = !collapsed[node.sec.id])}
  <!-- A folder row of the tree: it folds, drags into another section and takes
       clusters dropped on it; ⋯ / right-click / long-press hold its menu. -->
  <div
    role="presentation"
    class="side-row sec-head"
    class:drop={(draggedSectionId && draggedSectionId !== node.sec.id) || draggedClusterId}
    style:--side-depth={depth}
    draggable="true"
    ondragstart={(e) => {
      draggedSectionId = node.sec.id;
      e.stopPropagation();
    }}
    ondragend={() => (draggedSectionId = null)}
    ondragover={(e) => {
      if (draggedClusterId || (draggedSectionId && draggedSectionId !== node.sec.id)) e.preventDefault();
    }}
    ondrop={(e) => {
      e.preventDefault();
      e.stopPropagation();
      onSectionDrop(node.sec.id);
    }}
  >
    <button type="button" class="side-twisty" tabindex="-1" data-side-toggle aria-label="{isOpen ? 'Collapse' : 'Expand'} {node.sec.name}" title={isOpen ? 'Collapse' : 'Expand'} onclick={toggle}>
      <Icon name={isOpen ? 'chevronDown' : 'chevronRight'} size={12} />
    </button>
    <button
      type="button"
      class="side-row-main"
      data-side-row
      role="treeitem"
      aria-level={depth + 1}
      aria-expanded={isOpen}
      aria-selected="false"
      onclick={toggle}
      oncontextmenu={(e) => openMenu(e, 'section', node.sec.id)}
      use:rowMenu
    >
      <Icon name="folder" size={14} />
      <span class="side-row-text"><span class="side-row-title sec-name" dir="auto">{node.sec.name}</span></span>
      {#if node.items.length > 0}<span class="side-row-trailing">{node.items.length}</span>{/if}
    </button>
    <button type="button" class="icon-btn side-row-more" tabindex="-1" aria-label="Actions for {node.sec.name}" title="More" onclick={(e) => openMenu(e, 'section', node.sec.id)}>
      <Icon name="more" size={14} />
    </button>
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
  {@const sel = brokers.selectedId === c.id}
  <!-- The row drags into a section; it opens the cluster, and ⋯ / right-click /
       long-press hold its menu. -->
  <div
    role="presentation"
    class="side-row cluster"
    class:selected={sel}
    style:--side-depth={depth}
    draggable="true"
    ondragstart={(e) => {
      draggedClusterId = c.id;
      e.stopPropagation();
    }}
    ondragend={() => (draggedClusterId = null)}
  >
    <button
      type="button"
      class="side-row-main"
      data-side-row
      role="treeitem"
      aria-level={depth + 1}
      aria-selected={sel}
      title={c.name}
      onclick={() => pick(c.id)}
      oncontextmenu={(e) => openMenu(e, 'cluster', c.id)}
      use:rowMenu
    >
      <span class="dot" style="background: {c.color || 'var(--accent)'}"></span>
      <span class="side-row-text"><span class="side-row-title cn" dir="auto">{c.name}</span></span>
      <span class="side-row-trailing"><EnvBadge env={c.environment} /></span>
    </button>
    <button type="button" class="icon-btn side-row-more" tabindex="-1" aria-label="Actions for {c.name}" title="More" onclick={(e) => openMenu(e, 'cluster', c.id)}>
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
  /* A drop target while a cluster or section is dragged. */
  .sec-head.drop,
  .ungrouped.drop {
    background: var(--accent-soft);
  }
  .ungrouped {
    margin-block-start: 8px;
  }
  .dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    flex: none;
  }
  .cluster-main {
    flex: 1;
    display: flex;
    flex-direction: column;
    min-width: 0;
    min-height: 0;
  }
  .tabstrip {
    background: var(--surface);
    flex: none;
  }
  .tabs {
    flex: none;
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
</style>
