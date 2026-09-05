<script lang="ts">
  import { resourceAccess } from '../../lib/stores/resource-access.svelte';
  import ResourceAccess from '../../lib/components/ResourceAccess.svelte';
  // Clusters overview: one card per saved cluster (env pill, color dot, server
  // version + capability chips), "Add cluster" wizard (Admin), card context menu
  // (open / test / refresh capabilities / edit / delete).
  import { router } from '../../lib/router.svelte';
  import { k8s } from '../../lib/stores/k8s.svelte';
  import { auth } from '../../lib/stores/auth.svelte';
  import { ctxMenu } from '../../lib/contextmenu.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { k8sApi } from '../../lib/api/k8s';
  import type { K8sCluster } from '../../lib/api/types';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import Skeleton from '../../lib/components/Skeleton.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import ClusterWizard from './ClusterWizard.svelte';
  import InstallPanel from './InstallPanel.svelte';
  import Modal from '../../lib/components/Modal.svelte';
  import { envBadge } from './k8s-util';

  const isAdmin = $derived(auth.isRoot);
  let wizardOpen = $state(false);
  let editing: K8sCluster | null = $state(null);
  let k9sSheet = $state(false);
  let testing: Record<string, boolean> = $state({});

  function open(c: K8sCluster): void {
    router.go(`kubernetes/${encodeURIComponent(c.id)}`);
  }

  async function test(c: K8sCluster): Promise<void> {
    testing = { ...testing, [c.id]: true };
    try {
      const r = await k8sApi.testCluster(c.id);
      if (r.ok) toasts.success(`${c.name}: reachable`, `${r.server_version ?? ''} · ${r.latency_ms} ms`.trim());
      else toasts.error(`${c.name}: unreachable`, r.message);
      void k8s.loadCapabilities(c.id, true);
    } catch (e) {
      toasts.error(`${c.name}: test failed`, e instanceof Error ? e.message : String(e));
    } finally {
      testing = { ...testing, [c.id]: false };
    }
  }

  async function remove(c: K8sCluster): Promise<void> {
    const ok = await confirmer.ask(
      `Delete cluster “${c.name}”? Otto only forgets the row${c.source === 'kubeconfig' ? ' (your kubeconfig file is untouched)' : ' and removes the kubeconfig Otto wrote for it'}; nothing in the cluster changes.`,
      { title: 'Delete cluster', confirmLabel: 'Delete' },
    );
    if (!ok) return;
    try {
      await k8s.deleteCluster(c.id);
      toasts.success('Cluster removed', c.name);
    } catch (e) {
      toasts.error('Delete failed', e instanceof Error ? e.message : String(e));
    }
  }

  let accessFor = $state<string | null>(null);
  $effect(() => { for (const resource of k8s.clusters) void resourceAccess.load('k8s_cluster', resource.id); });

  function menu(e: MouseEvent | KeyboardEvent, c: K8sCluster): void {
    ctxMenu.show(e, [
      ...(resourceAccess.can('k8s_cluster', c.id, 'manage_access', 'kubernetes', 'admin')
        ? [{ label: 'Manage access…', icon: 'key', action: () => { accessFor = c.id; } }] : []),
      { label: 'Open', icon: 'helm', action: () => open(c) },
      { label: 'Test connection', icon: 'zap', action: () => void test(c) },
      { label: 'Refresh capabilities', icon: 'refresh', action: () => void k8s.loadCapabilities(c.id, true) },
      ...(resourceAccess.can('k8s_cluster', c.id, 'configure', 'kubernetes', 'admin')
        ? [
            { separator: true },
            { label: 'Edit…', icon: 'edit', action: () => { editing = c; wizardOpen = true; } },
            { label: 'Delete', icon: 'trash', danger: true, action: () => void remove(c) },
          ]
        : []),
    ]);
  }

  function cardKey(e: KeyboardEvent, c: K8sCluster): void {
    if (e.key === 'Enter' || e.key === ' ') {
      e.preventDefault();
      open(c);
    } else if (e.key === 'ContextMenu' || (e.shiftKey && e.key === 'F10')) {
      menu(e, c);
    }
  }

  const sourceLabel: Record<string, string> = { kubeconfig: 'kubeconfig', imported: 'pasted', eks: 'EKS' };
</script>

<div class="page">
  <div class="page-header">
    <div>
      <h1>Kubernetes</h1>
      <div class="sub">
        kubectl {k8s.status?.kubectl.version ?? ''}
        {#if k8s.status?.k9s.installed}· k9s {k8s.status.k9s.version ?? ''}{:else if isAdmin}
          · <button class="link" onclick={() => (k9sSheet = true)}>Install k9s</button>{/if}
      </div>
    </div>
    <div class="actions">
      <button class="btn" onclick={() => router.go('kubernetes/monitor')} title="Monitoring dashboard: pod metrics, restarts, health" data-testid="k8s-monitor-btn">
        <Icon name="gauge" size={14} /> Monitor
      </button>
      <button class="btn ghost" onclick={() => void k8s.loadClusters()} title="Refresh" aria-label="Refresh clusters">
        <Icon name="refresh" size={14} />
      </button>
      {#if isAdmin}
        <button class="btn primary" data-testid="k8s-add-cluster" onclick={() => { editing = null; wizardOpen = true; }}>
          <Icon name="plus" size={14} /> Add cluster
        </button>
      {/if}
    </div>
  </div>

  {#if k8s.clustersError && !k8s.clusters.length}
    <EmptyState icon="helm" title="Couldn't load clusters" body={k8s.clustersError} actionLabel="Retry" onaction={() => void k8s.loadClusters()} />
  {:else if !k8s.clustersLoaded}
    <Skeleton rows={3} height={96} />
  {:else if !k8s.clusters.length}
    <EmptyState
      icon="helm"
      title="No clusters yet"
      body={isAdmin
        ? 'Add a cluster from a context in your kubeconfig, paste a kubeconfig, or import one from EKS in the AWS module.'
        : 'No clusters have been added. Ask an Otto admin to add one.'}
      actionLabel={isAdmin ? 'Add cluster' : undefined}
      onaction={isAdmin ? () => { editing = null; wizardOpen = true; } : undefined}
    />
  {:else}
    <div class="grid" data-testid="k8s-cluster-grid">
      {#each k8s.clusters as c (c.id)}
        {@const caps = k8s.capabilities[c.id]}
        <div
          class="card cluster"
          class:prod={c.environment === 'prod'}
          role="button"
          tabindex="0"
          data-testid="k8s-cluster-card"
          onclick={() => open(c)}
          onkeydown={(e) => cardKey(e, c)}
          oncontextmenu={(e) => menu(e, c)}
        >
          <div class="row1">
            <span class="dot" style="background:{c.color || 'var(--accent)'}"></span>
            <span class="name" title={c.name}>{c.name}</span>
            <span class="env-badge mono" class:prod={c.environment === 'prod'}>{envBadge(c.environment)}</span>
            <button class="icon-btn more" aria-label="Cluster actions" onclick={(e) => { e.stopPropagation(); menu(e, c); }}>
              <Icon name="grip" size={13} />
            </button>
          </div>
          <div class="row2 mono" title={c.context_name}>
            <span class="dim">context</span> {c.context_name}
            {#if c.default_namespace}<span class="dim"> · ns</span> {c.default_namespace}{/if}
          </div>
          <div class="row3">
            <span class="chip">{sourceLabel[c.source] ?? c.source}</span>
            {#if caps?.server_version}<span class="chip mono">{caps.server_version}</span>{/if}
            {#if caps}
              <span class="chip" class:ok={caps.metrics_server} title="metrics-server">metrics</span>
              {#if caps.argo_rollouts}<span class="chip ok" title="Argo Rollouts CRD present">rollouts</span>{/if}
              {#if caps.argocd}<span class="chip ok" title="ArgoCD Application CRD present">argocd</span>{/if}
            {:else}
              <span class="chip dim">capabilities pending</span>
            {/if}
            {#if testing[c.id]}<span class="chip accent">testing…</span>{/if}
          </div>
        </div>
      {/each}
    </div>
  {/if}
</div>

{#if wizardOpen}
  <ClusterWizard existing={editing} onclose={() => { wizardOpen = false; editing = null; }} />
{/if}

{#if k9sSheet}
  <Modal title="Install k9s" width={520} onclose={() => (k9sSheet = false)}>
    <InstallPanel tool="k9s" compact />
    {#snippet footer()}
      <button class="btn" onclick={() => (k9sSheet = false)}>Close</button>
    {/snippet}
  </Modal>
{/if}

{#if accessFor}
  <Modal title="Manage access" width={780} onclose={() => (accessFor = null)}>
    <ResourceAccess kind="k8s_cluster" resourceId={accessFor} />
  </Modal>
{/if}

<style>
  .actions {
    display: flex;
    gap: 8px;
    align-items: center;
  }
  .link {
    background: none;
    border: none;
    padding: 0;
    color: var(--accent);
    cursor: pointer;
    font-size: inherit;
  }
  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(300px, 1fr));
    gap: 12px;
  }
  .cluster {
    padding: 14px 16px;
    display: flex;
    flex-direction: column;
    gap: 8px;
    cursor: pointer;
    transition: border-color 130ms ease-out, background 130ms ease-out;
  }
  .cluster:hover,
  .cluster:focus-visible {
    border-color: color-mix(in srgb, var(--accent) 45%, var(--border));
    background: color-mix(in srgb, var(--accent) 4%, var(--surface));
  }
  .cluster.prod {
    border-color: color-mix(in srgb, var(--status-exited) 35%, var(--border));
  }
  .row1 {
    display: flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
  }
  .dot {
    width: 9px;
    height: 9px;
    border-radius: 50%;
    flex-shrink: 0;
  }
  .name {
    font-weight: 600;
    font-size: 13.5px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    flex: 1;
    min-width: 0;
  }
  .more {
    opacity: 0.5;
  }
  .cluster:hover .more {
    opacity: 1;
  }
  .row2 {
    font-size: 11.5px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .row3 {
    display: flex;
    gap: 6px;
    flex-wrap: wrap;
  }
  .dim {
    color: var(--text-dim);
  }
  .mono {
    font-family: var(--font-mono);
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
  @media (max-width: 640px) {
    .grid {
      grid-template-columns: 1fr;
    }
  }
</style>
