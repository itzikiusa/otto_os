<script lang="ts">
  // k9s-like cluster workspace. Top bar: cluster switcher · namespace combobox
  // · text filter · refresh / auto-refresh · k9s · key hints. Left: kinds rail
  // (Argo Rollouts / ArgoCD Apps only when the capability probe says so; a
  // <select> on phones). Center: ResourceTable. Right: ResourceDrawer behind a
  // hand-rolled splitter (a full sheet on phones). Cluster / kind / selected
  // row live in the URL; namespace + filter + cache live in the store.
  import { untrack } from 'svelte';
  import { router } from '../../lib/router.svelte';
  import { k8s } from '../../lib/stores/k8s.svelte';
  import type { K8sDrawerTab } from '../../lib/stores/k8s.svelte';
  import { auth } from '../../lib/stores/auth.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { ui } from '../../lib/stores/ui.svelte';
  import { viewport } from '../../lib/stores/viewport.svelte';
  import { ctxMenu } from '../../lib/contextmenu.svelte';
  import type { MenuItem } from '../../lib/contextmenu.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { keyContext } from '../../lib/keys';
  import { api } from '../../lib/api/client';
  import { k8sApi } from '../../lib/api/k8s';
  import type { K8sCluster, K8sResourceKind, K8sRow } from '../../lib/api/types';
  import Icon from '../../lib/components/Icon.svelte';
  import Modal from '../../lib/components/Modal.svelte';
  import Terminal from '../../lib/components/Terminal.svelte';
  import NamespacePicker from './NamespacePicker.svelte';
  import ResourceTable from './ResourceTable.svelte';
  import ResourceDrawer from './ResourceDrawer.svelte';
  import ScaleDialog from './ScaleDialog.svelte';
  import SyncDialog from './SyncDialog.svelte';
  import InstallPanel from './InstallPanel.svelte';
  import type { ActionDef } from './actions';
  import { actionsFor, runAction } from './actions';
  import { envBadge, formatAge, kindDef, visibleKinds } from './k8s-util';

  interface Props {
    cluster: K8sCluster;
  }
  let { cluster }: Props = $props();

  const canEdit = $derived(auth.can('kubernetes', 'edit'));
  const isAdmin = $derived(auth.can('kubernetes', 'admin'));
  const kinds = $derived(visibleKinds(k8s.caps));
  const kind = $derived(k8s.kind);
  const def = $derived(kindDef(kind));
  const clusterScoped = $derived(!!def.clusterScoped);
  const allNs = $derived(k8s.namespace === '' && !clusterScoped);
  const sel = $derived(k8s.selected);
  const selRow = $derived(k8s.selectedRow);
  const rowsForKey = $derived(k8s.rowsKey === k8s.currentKey ? k8s.rows : []);

  let filterEl = $state<HTMLInputElement | null>(null);
  let nsPicker = $state<NamespacePicker | null>(null);
  let hintsOpen = $state(false);
  let k9sInstallOpen = $state(false);
  let k9sOpening = $state(false);
  let scaleFor = $state<{ row: K8sRow; def: ActionDef } | null>(null);
  let syncFor = $state<{ row: K8sRow; def: ActionDef } | null>(null);
  /** Set by the `s` shortcut so the drawer's Terminal tab opens the shell at once. */
  let autoExec = $state(false);

  // Drawer width (desktop) — persisted; clamped so neither pane collapses.
  const DRAWER_KEY = 'otto_k8s_drawer_w';
  let drawerW = $state(
    (() => {
      try {
        const v = Number(localStorage.getItem(DRAWER_KEY));
        return v >= 320 ? v : 520;
      } catch {
        return 520;
      }
    })(),
  );
  let resizing = $state(false);
  let bodyEl = $state<HTMLDivElement | null>(null);

  function startResize(e: PointerEvent): void {
    e.preventDefault();
    resizing = true;
    const startX = e.clientX;
    const startW = drawerW;
    const maxW = Math.max(360, (bodyEl?.clientWidth ?? 1200) - 420);
    const onMove = (ev: PointerEvent): void => {
      // Drawer sits on the right: dragging left widens it.
      drawerW = Math.max(320, Math.min(maxW, startW - (ev.clientX - startX)));
    };
    const onUp = (): void => {
      resizing = false;
      try {
        localStorage.setItem(DRAWER_KEY, String(Math.round(drawerW)));
      } catch {
        /* ignore */
      }
      window.removeEventListener('pointermove', onMove);
      window.removeEventListener('pointerup', onUp);
    };
    window.addEventListener('pointermove', onMove);
    window.addEventListener('pointerup', onUp);
  }

  // --- routing helpers ----------------------------------------------------------
  const base = $derived(`kubernetes/${encodeURIComponent(cluster.id)}`);
  function goKind(k: K8sResourceKind): void {
    router.go(`${base}/${k}`);
  }
  function openRow(r: K8sRow, tab?: K8sDrawerTab): void {
    if (tab) k8s.drawerTab = tab;
    router.go(`${base}/${kind}/${encodeURIComponent(r.namespace || '-')}/${encodeURIComponent(r.name)}`);
  }
  /** From a workload drawer to one of its pods: switch the table to Pods and
   *  route to that pod so its own drawer (Logs / Terminal / Metrics) opens. */
  function openPod(podNs: string, pod: string, tab?: K8sDrawerTab): void {
    autoExec = false;
    k8s.drawerTab = tab ?? 'overview';
    if (podNs && podNs !== k8s.namespace && k8s.namespace !== '') k8s.setNamespace(podNs);
    router.go(`${base}/pods/${encodeURIComponent(podNs || '-')}/${encodeURIComponent(pod)}`);
  }
  function closeDrawer(): void {
    autoExec = false;
    // A single click selects WITHOUT touching the URL, so navigating to the
    // kind route alone is a no-op when nothing deeper was ever routed to —
    // clear the selection explicitly as well.
    k8s.select(null);
    router.go(`${base}/${kind}`);
  }
  function switchCluster(id: string): void {
    if (id && id !== cluster.id) router.go(`kubernetes/${encodeURIComponent(id)}`);
  }

  // --- data loading ---------------------------------------------------------------
  // Reload whenever (cluster, kind, namespace) changes; the load itself is
  // untracked so store writes during the fetch can't re-trigger it.
  $effect(() => {
    void k8s.currentKey;
    if (!k8s.clusterId) return;
    untrack(() => void k8s.loadResources());
  });
  $effect(() => {
    k8s.startAutoRefresh();
    return () => k8s.stopAutoRefresh();
  });
  // Kinds that need a capability that's gone (or a CRD kind on a cluster
  // without it) fall back to pods.
  $effect(() => {
    const ks = kinds;
    const k = kind;
    if (k8s.caps && !ks.some((x) => x.id === k)) untrack(() => goKind('pods'));
  });

  // --- actions --------------------------------------------------------------------
  async function doAction(a: ActionDef, r: K8sRow): Promise<void> {
    if (!canEdit) return;
    if (a.needs === 'scale') {
      scaleFor = { row: r, def: a };
      return;
    }
    if (a.needs === 'sync') {
      syncFor = { row: r, def: a };
      return;
    }
    const resp = await runAction(cluster.id, kind, r, a);
    if (resp?.output && a.id === 'rollout_status') toasts.info('Rollout status', resp.output);
    if (resp) void k8s.loadResources(true);
  }

  function rowMenu(e: MouseEvent | KeyboardEvent, r: K8sRow): void {
    const items: MenuItem[] = [
      { label: 'Details', icon: 'info', action: () => openRow(r, 'overview') },
      { label: 'Manifest (YAML)', icon: 'file', action: () => openRow(r, 'manifest') },
      { label: 'Describe', icon: 'note', action: () => openRow(r, 'describe') },
    ];
    if (kind === 'pods') {
      items.push({ label: 'Logs', icon: 'file', action: () => openRow(r, 'logs') });
      if (canEdit) items.push({ label: 'Shell (exec)', icon: 'terminal', action: () => { autoExec = true; openRow(r, 'terminal'); } });
    } else if (r.extra?.selector) {
      items.push({ label: 'Pods', icon: 'box', action: () => openRow(r, 'pods') });
      items.push({ label: 'Logs (all pods)', icon: 'file', action: () => openRow(r, 'logs') });
    }
    const acts = canEdit ? actionsFor(kind, r) : [];
    if (acts.length) {
      items.push({ separator: true });
      for (const a of acts) items.push({ label: a.label, icon: a.icon, danger: a.danger, action: () => void doAction(a, r) });
    }
    ctxMenu.show(e, items);
  }

  // --- k9s -------------------------------------------------------------------------
  async function openK9s(): Promise<void> {
    if (!canEdit || k9sOpening) return;
    if (!k8s.status?.k9s.installed) {
      if (isAdmin) k9sInstallOpen = true;
      else toasts.warn('k9s is not installed', 'Ask an Otto admin to install it from the Kubernetes overview.');
      return;
    }
    const wsId = ws.currentId;
    if (!wsId) {
      toasts.error('No workspace', 'Select a workspace to attach the k9s session to.');
      return;
    }
    k9sOpening = true;
    try {
      const s = await k8sApi.k9s(cluster.id, { workspace_id: wsId, ns: k8s.namespace || null });
      k8s.k9sSessionId = s.id;
    } catch (e) {
      toasts.error('k9s failed to start', e instanceof Error ? e.message : String(e));
    } finally {
      k9sOpening = false;
    }
  }
  async function closeK9s(): Promise<void> {
    const id = k8s.k9sSessionId;
    k8s.k9sSessionId = null;
    if (!id) return;
    try {
      await api.del(`/sessions/${id}`);
    } catch {
      /* best-effort */
    }
  }

  // --- keyboard (k9s muscle memory) ---------------------------------------------------
  function isTyping(t: EventTarget | null): boolean {
    const el = t as HTMLElement | null;
    if (!el) return false;
    const tag = el.tagName;
    return tag === 'INPUT' || tag === 'TEXTAREA' || tag === 'SELECT' || el.isContentEditable;
  }
  $effect(() => {
    const onKey = (e: KeyboardEvent): void => {
      if (e.metaKey || e.ctrlKey || e.altKey) return;
      if (keyContext.terminalFocused || ui.modalCount > 0 || ctxMenu.open || k8s.k9sSessionId) return;
      if (e.key === 'Escape') {
        if (isTyping(e.target)) {
          (e.target as HTMLElement).blur();
          return;
        }
        if (k8s.selected) {
          e.preventDefault();
          closeDrawer();
        }
        return;
      }
      if (isTyping(e.target)) return;
      const row = k8s.selectedRow ?? (k8s.selected ? null : k8s.filteredRows[0] ?? null);
      switch (e.key) {
        case '/':
          e.preventDefault();
          filterEl?.focus();
          filterEl?.select();
          break;
        case '?':
          // The shell binds `?` too (global cheat-sheet). We listen in the
          // capture phase and stop propagation so only the module sheet opens.
          e.preventDefault();
          e.stopPropagation();
          hintsOpen = !hintsOpen;
          break;
        case 'n':
          e.preventDefault();
          nsPicker?.focus();
          break;
        case 'r':
          e.preventDefault();
          void k8s.loadResources();
          break;
        case 'l':
          if (kind === 'pods' && row) {
            e.preventDefault();
            openRow(row, 'logs');
          }
          break;
        case 's':
          if (kind === 'pods' && row && canEdit) {
            e.preventDefault();
            autoExec = true;
            openRow(row, 'terminal');
          }
          break;
        case 'd':
          if (row) {
            e.preventDefault();
            openRow(row, 'describe');
          }
          break;
        case 'y':
          if (row) {
            e.preventDefault();
            openRow(row, 'manifest');
          }
          break;
        case 'j':
        case 'k': {
          const rows = k8s.filteredRows;
          if (!rows.length) break;
          e.preventDefault();
          const i = k8s.selectedRow ? rows.indexOf(k8s.selectedRow) : -1;
          const j = e.key === 'j' ? Math.min(rows.length - 1, i + 1) : Math.max(0, i - 1);
          const next = rows[j];
          if (next) {
            if (k8s.selected) openRow(next);
            else k8s.select({ ns: next.namespace, name: next.name });
          }
          break;
        }
        case 'Enter':
          if (row) {
            e.preventDefault();
            openRow(row);
          }
          break;
      }
    };
    window.addEventListener('keydown', onKey, true);
    return () => window.removeEventListener('keydown', onKey, true);
  });

  const lastLoaded = $derived(k8s.rowsLoadedAt ? formatAge(Math.floor((Date.now() - k8s.rowsLoadedAt) / 1000)) : '');
  const drawerOpen = $derived(!!sel);
</script>

<div class="wsp" class:prod={cluster.environment === 'prod'} data-testid="k8s-workspace">
  <div class="topbar">
    <button class="icon-btn" onclick={() => router.go('kubernetes')} title="All clusters" aria-label="Back to clusters"><Icon name="chevronLeft" size={14} /></button>
    <span class="dot" style="background:{cluster.color || 'var(--accent)'}"></span>
    <select class="input cluster-sel" aria-label="Cluster" value={cluster.id} onchange={(e) => switchCluster((e.currentTarget as HTMLSelectElement).value)} data-testid="k8s-cluster-switcher">
      {#each k8s.clusters as c (c.id)}<option value={c.id}>{c.name}</option>{/each}
    </select>
    <span class="env-badge mono" class:prod={cluster.environment === 'prod'}>{envBadge(cluster.environment)}</span>
    {#if k8s.caps?.server_version}<span class="ver mono" title="Server version">{k8s.caps.server_version}</span>{/if}

    <NamespacePicker bind:this={nsPicker} value={k8s.namespace} namespaces={k8s.namespaces} error={k8s.namespacesError} disabled={clusterScoped} onchange={(ns) => k8s.setNamespace(ns)} />

    <div class="filter">
      <Icon name="search" size={12} />
      <input bind:this={filterEl} class="filter-in" placeholder="Filter  ( / )" bind:value={k8s.filter} aria-label="Filter rows" data-testid="k8s-filter" />
      {#if k8s.filter}<button class="icon-btn" onclick={() => (k8s.filter = '')} aria-label="Clear filter"><Icon name="x" size={11} /></button>{/if}
    </div>

    <span class="spacer"></span>

    <span class="meta dim" title={k8s.rowsLoadedAt ? new Date(k8s.rowsLoadedAt).toLocaleTimeString() : ''}>
      {#if k8s.rowsLoading}loading…{:else if lastLoaded}{k8s.filteredRows.length}{k8s.filter ? `/${rowsForKey.length}` : ''} · {lastLoaded} ago{/if}
    </span>
    <button class="icon-btn" onclick={() => void k8s.loadResources()} title="Refresh (r)" aria-label="Refresh"><Icon name="refresh" size={14} /></button>
    <button class="pill-toggle" class:on={k8s.autoRefresh} onclick={() => k8s.setAutoRefresh(!k8s.autoRefresh)} aria-pressed={k8s.autoRefresh} title="Auto-refresh every 10 s">
      <Icon name="clock" size={11} /> Auto
    </button>
    {#if canEdit}
      <button class="btn small" onclick={() => void openK9s()} disabled={k9sOpening} title="Open k9s in a terminal" data-testid="k8s-k9s-btn">
        <Icon name="terminal" size={12} /> k9s
      </button>
    {/if}
    <button class="icon-btn" onclick={() => (hintsOpen = true)} title="Keyboard shortcuts (?)" aria-label="Keyboard shortcuts"><Icon name="command" size={14} /></button>
  </div>

  {#if k8s.k9sSessionId}
    <div class="k9s">
      <div class="k9s-bar">
        <Icon name="terminal" size={13} />
        <span>k9s · {cluster.name}{k8s.namespace ? ` · ${k8s.namespace}` : ''}</span>
        <span class="spacer"></span>
        <button class="btn small" onclick={() => void closeK9s()}>Close k9s</button>
      </div>
      <div class="k9s-term">
        {#key k8s.k9sSessionId}
          <Terminal sessionId={k8s.k9sSessionId} preferDom autoFocus forceDark />
        {/key}
      </div>
    </div>
  {:else}
    <div class="body" bind:this={bodyEl} class:resizing>
      {#if viewport.isPhone}
        <div class="kinds-mobile">
          <select class="input" aria-label="Resource kind" value={kind} onchange={(e) => goKind((e.currentTarget as HTMLSelectElement).value as K8sResourceKind)} data-testid="k8s-kinds">
            {#each kinds as k (k.id)}<option value={k.id}>{k.label}</option>{/each}
          </select>
        </div>
      {:else}
        <nav class="kinds" aria-label="Resource kinds" data-testid="k8s-kinds">
          {#each kinds as k (k.id)}
            <button class="kind" class:active={k.id === kind} class:crd={!!k.requires} onclick={() => goKind(k.id)} aria-current={k.id === kind ? 'page' : undefined}>
              {k.label}
              {#if k.id === kind && !k8s.rowsLoading}<span class="cnt mono">{rowsForKey.length}</span>{/if}
            </button>
          {/each}
        </nav>
      {/if}

      <div class="center">
        <ResourceTable
          {kind}
          rows={k8s.filteredRows}
          total={rowsForKey.length}
          hasMetrics={k8s.hasMetrics}
          allNamespaces={allNs}
          loading={k8s.rowsLoading}
          error={k8s.rowsError}
          selected={sel}
          onselect={(r) => k8s.select({ ns: r.namespace, name: r.name })}
          onopen={(r) => openRow(r)}
          onmenu={rowMenu}
          onretry={() => void k8s.loadResources()}
        />
      </div>

      {#if drawerOpen && sel}
        {#if !viewport.isPhone}
          <!-- svelte-ignore a11y_no_static_element_interactions -->
          <div class="splitter" role="separator" aria-orientation="vertical" aria-label="Drag to resize details" onpointerdown={startResize}><span class="grip"></span></div>
        {/if}
        <div class="drawer-host" class:sheet={viewport.isPhone} style={viewport.isPhone ? '' : `width:${drawerW}px`}>
          {#key `${cluster.id}/${kind}/${sel.ns}/${sel.name}`}
            <ResourceDrawer
              clusterId={cluster.id}
              {kind}
              ns={sel.ns}
              name={sel.name}
              row={selRow}
              tab={k8s.drawerTab}
              {canEdit}
              {autoExec}
              ontab={(t) => (k8s.drawerTab = t)}
              onclose={closeDrawer}
              onopenpod={openPod}
              onaction={(a, r) => void doAction(a, r)}
            />
          {/key}
        </div>
      {/if}
    </div>
  {/if}
</div>

{#if scaleFor}
  <ScaleDialog
    row={scaleFor.row}
    kindLabel={def.singular}
    onclose={() => (scaleFor = null)}
    onsubmit={(n) => {
      const f = scaleFor!;
      scaleFor = null;
      void runAction(cluster.id, kind, f.row, f.def, { replicas: n }).then((r) => r && k8s.loadResources(true));
    }}
  />
{/if}
{#if syncFor}
  <SyncDialog
    row={syncFor.row}
    onclose={() => (syncFor = null)}
    onsubmit={(p) => {
      const f = syncFor!;
      syncFor = null;
      void runAction(cluster.id, kind, f.row, f.def, p).then((r) => r && k8s.loadResources(true));
    }}
  />
{/if}
{#if k9sInstallOpen}
  <Modal title="Install k9s" width={520} onclose={() => (k9sInstallOpen = false)}>
    <InstallPanel tool="k9s" compact />
    {#snippet footer()}
      <button class="btn" onclick={() => (k9sInstallOpen = false)}>Close</button>
      <button class="btn primary" disabled={!k8s.status?.k9s.installed} onclick={() => { k9sInstallOpen = false; void openK9s(); }}>Open k9s</button>
    {/snippet}
  </Modal>
{/if}
{#if hintsOpen}
  <Modal title="Keyboard shortcuts" width={420} onclose={() => (hintsOpen = false)}>
    <dl class="hints">
      <dt><kbd>/</kbd></dt><dd>Focus the filter</dd>
      <dt><kbd>n</kbd></dt><dd>Pick a namespace</dd>
      <dt><kbd>j</kbd> / <kbd>k</kbd></dt><dd>Next / previous row</dd>
      <dt><kbd>Enter</kbd></dt><dd>Open details for the selected row</dd>
      <dt><kbd>d</kbd></dt><dd>Describe</dd>
      <dt><kbd>y</kbd></dt><dd>Manifest (YAML)</dd>
      <dt><kbd>l</kbd></dt><dd>Logs (pods)</dd>
      <dt><kbd>s</kbd></dt><dd>Shell into the pod (edit)</dd>
      <dt><kbd>r</kbd></dt><dd>Refresh</dd>
      <dt><kbd>Esc</kbd></dt><dd>Close details / leave the filter</dd>
      <dt><kbd>?</kbd></dt><dd>This list</dd>
    </dl>
    {#snippet footer()}
      <button class="btn" onclick={() => (hintsOpen = false)}>Close</button>
    {/snippet}
  </Modal>
{/if}

<style>
  .wsp {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
    overflow: hidden;
  }
  .topbar {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 10px;
    border-bottom: 1px solid var(--border);
    background: var(--surface);
    flex-wrap: wrap;
  }
  .wsp.prod .topbar {
    box-shadow: inset 0 2px 0 color-mix(in srgb, var(--status-exited) 60%, transparent);
  }
  .dot {
    width: 9px;
    height: 9px;
    border-radius: 50%;
    flex-shrink: 0;
  }
  .cluster-sel {
    max-width: 200px;
    font-weight: 600;
  }
  .ver {
    font-size: 11px;
    color: var(--text-dim);
  }
  .filter {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    height: 27px;
    padding: 0 6px 0 8px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--surface-2);
    color: var(--text-dim);
    min-width: 0;
    flex: 0 1 260px;
  }
  .filter:focus-within {
    border-color: var(--accent);
    box-shadow: 0 0 0 3px color-mix(in srgb, var(--accent) 22%, transparent);
  }
  .filter-in {
    flex: 1;
    min-width: 40px;
    border: none;
    background: transparent;
    color: var(--text);
    font-size: 12.5px;
    outline: none;
  }
  .spacer {
    flex: 1;
  }
  .meta {
    font-size: 11px;
    white-space: nowrap;
  }
  .body {
    flex: 1;
    min-height: 0;
    display: flex;
    overflow: hidden;
    position: relative;
  }
  .body.resizing {
    cursor: col-resize;
    user-select: none;
  }
  .kinds {
    width: 168px;
    flex-shrink: 0;
    overflow-y: auto;
    border-right: 1px solid var(--border);
    padding: 6px;
    display: flex;
    flex-direction: column;
    gap: 1px;
    background: var(--surface);
  }
  .kind {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 6px;
    border: none;
    background: transparent;
    text-align: left;
    padding: 5px 8px;
    border-radius: var(--radius-s);
    font-size: 12.5px;
    color: var(--text-dim);
    cursor: pointer;
  }
  .kind:hover {
    background: var(--surface-2);
    color: var(--text);
  }
  .kind.active {
    background: color-mix(in srgb, var(--accent) 14%, transparent);
    color: var(--text);
    font-weight: 500;
  }
  .kind.crd {
    margin-top: 2px;
  }
  .cnt {
    font-size: 10.5px;
    color: var(--text-dim);
  }
  .kinds-mobile {
    position: absolute;
    top: 6px;
    left: 10px;
    right: 10px;
    z-index: 2;
  }
  .kinds-mobile .input {
    width: 100%;
  }
  .center {
    flex: 1;
    min-width: 0;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }
  @media (max-width: 640px) {
    .center {
      padding-top: 40px;
    }
  }
  .splitter {
    width: 6px;
    flex-shrink: 0;
    cursor: col-resize;
    background: var(--border);
    display: grid;
    place-items: center;
    touch-action: none;
  }
  .splitter:hover,
  .resizing .splitter {
    background: color-mix(in srgb, var(--accent) 45%, var(--border));
  }
  .grip {
    width: 2px;
    height: 28px;
    border-radius: 2px;
    background: var(--text-dim);
    opacity: 0.6;
  }
  .drawer-host {
    flex-shrink: 0;
    min-width: 0;
    height: 100%;
    overflow: hidden;
    border-left: 1px solid var(--border);
  }
  .drawer-host.sheet {
    position: fixed;
    inset: 0;
    z-index: 40;
    width: auto;
    border-left: none;
  }
  .k9s {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }
  .k9s-bar {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 4px 10px;
    font-size: 12px;
    border-bottom: 1px solid var(--border);
  }
  .k9s-term {
    flex: 1;
    min-height: 0;
    background: #000;
  }
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
  .hints {
    display: grid;
    grid-template-columns: auto 1fr;
    gap: 6px 14px;
    margin: 0;
    font-size: 12.5px;
  }
  .hints dt {
    text-align: right;
  }
  .hints dd {
    margin: 0;
  }
  kbd {
    display: inline-block;
    min-width: 18px;
    padding: 1px 5px;
    border-radius: 4px;
    border: 1px solid var(--border);
    background: var(--surface-2);
    font-family: var(--font-mono);
    font-size: 11px;
    text-align: center;
  }
  .dim {
    color: var(--text-dim);
  }
  .mono {
    font-family: var(--font-mono);
  }
</style>
