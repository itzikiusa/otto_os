<script lang="ts">
  // Cluster & SSH Network Bridge card shared by `IdeEnvironmentsModal` and
  // `IdeSshTargetModal`. Attaches a Kubernetes cluster + namespace and/or SSH
  // bastion tunnel to local Run & Debug sessions so cluster-internal hostnames
  // (`config:9090`, `confsrv.stgco.local`, `<svc>.<ns>.svc.cluster.local`,
  // ClusterIPs, Ingress hosts) resolve transparently without `sudo` or `/etc/hosts`.
  import { untrack } from 'svelte';
  import { api } from '../../../lib/api/client';
  import type { Connection, K8sCluster, K8sNamespace } from '../../../lib/api/types';
  import { announce, announceShown } from '../../../lib/announce.svelte';
  import Badge from '../../../lib/components/Badge.svelte';
  import EnvBadge from '../../../lib/components/EnvBadge.svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import LoadState from '../../../lib/components/LoadState.svelte';
  import Switch from '../../../lib/components/Switch.svelte';
  import { loadErrorText } from '../../../lib/loadError';
  import { plural } from '../../../lib/plural';
  import { router } from '../../../lib/router.svelte';
  import { SCRATCH_WORKSPACE_ID, ws } from '../../../lib/stores/workspace.svelte';
  import { ide, type IdeBridgedServiceSummary, type IdeNetBridgeStatusResp } from '../ideStore.svelte';
  import { bridgeEnvCache, confirmProdNetworkBridge } from './networkBridgeGuard.svelte';
  import {
    isProdEnvironment,
    mergeClusterNamespaceSuggestions,
    serializeNetworkBridgeDraft,
    type IdeNetworkBridgeDraft,
  } from './networkBridgeModel';

  interface Props {
    draft: IdeNetworkBridgeDraft;
    sshConnections?: Connection[];
    idPrefix?: string;
    saving?: boolean;
    onSave?: () => void;
    onDirty?: () => void;
  }

  let {
    draft = $bindable(),
    sshConnections,
    idPrefix = 'nb',
    saving = false,
    onSave,
    onDirty,
  }: Props = $props();

  let clusters = $state<K8sCluster[]>([]);
  let clustersLoading = $state(false);
  let clustersError = $state<string | null>(null);
  let loadedSshConns = $state<Connection[]>([]);
  let sshLoading = $state(false);
  let sshError = $state<string | null>(null);
  let liveNamespaces = $state<K8sNamespace[]>([]);
  let namespacesError = $state<string | null>(null);
  let nsLoadedForCluster = $state('');
  let mappingsOpen = $state(false);
  let testing = $state(false);
  let probeError = $state('');
  let probeResult = $state<IdeNetBridgeStatusResp | null>(null);

  const effectiveSshConns = $derived(sshConnections ?? loadedSshConns);
  const selectedCluster = $derived(
    clusters.find((c) => c.id === draft.k8s_cluster_id) ?? null,
  );
  const selectedSsh = $derived(
    effectiveSshConns.find((c) => c.id === draft.ssh_tunnel_connection_id) ?? null,
  );
  const isProdSelected = $derived(
    isProdEnvironment(selectedCluster?.environment) || isProdEnvironment(selectedSsh?.environment),
  );
  const namespaceChips = $derived(
    mergeClusterNamespaceSuggestions(selectedCluster, liveNamespaces).slice(0, 8),
  );

  $effect(() => {
    if (sshConnections && sshConnections.length > 0) {
      bridgeEnvCache.seedSshConns(sshConnections, ide.target.workspace_id);
    }
  });

  function markDirty(): void {
    probeError = '';
    onDirty?.();
  }

  $effect(() => {
    untrack(() => {
      void loadClusters();
      if (sshConnections === undefined) {
        void loadWorkspaceSshConns();
      }
      if (draft.mappings.length > 0) {
        mappingsOpen = true;
      }
    });
  });

  $effect(() => {
    const cid = draft.k8s_cluster_id.trim();
    if (!cid) {
      liveNamespaces = [];
      namespacesError = null;
      nsLoadedForCluster = '';
      return;
    }
    if (cid === nsLoadedForCluster) return;
    nsLoadedForCluster = cid;
    untrack(() => {
      void loadClusterNamespaces(cid);
    });
  });

  async function loadClusters(): Promise<void> {
    clustersLoading = true;
    clustersError = null;
    try {
      clusters = await api.get<K8sCluster[]>('/k8s/clusters');
      bridgeEnvCache.seedClusters(clusters);
    } catch (e) {
      clusters = [];
      clustersError = loadErrorText(e);
    } finally {
      clustersLoading = false;
    }
  }

  async function loadClusterNamespaces(clusterId: string): Promise<void> {
    if (!clusterId) return;
    namespacesError = null;
    try {
      const resp = await api.get<{ namespaces: K8sNamespace[] }>(
        `/k8s/clusters/${encodeURIComponent(clusterId)}/namespaces`,
      );
      if (nsLoadedForCluster === clusterId) {
        liveNamespaces = resp.namespaces ?? [];
      }
    } catch (e) {
      if (nsLoadedForCluster === clusterId) {
        liveNamespaces = [];
        namespacesError = loadErrorText(e);
      }
    }
  }

  async function loadWorkspaceSshConns(): Promise<void> {
    sshLoading = true;
    sshError = null;
    try {
      const scopes = Array.from(
        new Set(
          [
            ide.target.workspace_id,
            ws.currentId,
            ...ws.workspaces.map((w) => w.id),
            ws.scratch?.id ?? SCRATCH_WORKSPACE_ID,
          ].filter((id): id is string => Boolean(id)),
        ),
      );
      if (scopes.length === 0) scopes.push(SCRATCH_WORKSPACE_ID);
      const results = await Promise.allSettled(
        scopes.map((id) => api.get<Connection[]>(`/workspaces/${encodeURIComponent(id)}/connections`)),
      );
      const seen = new Map<string, Connection>();
      let anyOk = false;
      let firstErr: unknown = null;
      for (const r of results) {
        if (r.status === 'fulfilled') {
          anyOk = true;
          for (const c of r.value) {
            if (c.kind === 'ssh' && !seen.has(c.id)) seen.set(c.id, c);
          }
        } else if (!firstErr) {
          firstErr = r.reason;
        }
      }
      if (!anyOk && firstErr) {
        sshError = loadErrorText(firstErr);
      }
      loadedSshConns = Array.from(seen.values());
      if (anyOk) bridgeEnvCache.seedSshConns(loadedSshConns);
    } catch (e) {
      loadedSshConns = [];
      sshError = loadErrorText(e);
    } finally {
      sshLoading = false;
    }
  }

  async function setEnabledWithGuard(next: boolean): Promise<boolean> {
    if (!next) {
      draft.enabled = false;
      markDirty();
      return true;
    }
    const candidate = {
      enabled: true,
      k8s_cluster_id: draft.k8s_cluster_id.trim() || null,
      k8s_namespace: draft.k8s_namespace.trim() || null,
      ssh_tunnel_connection_id: draft.ssh_tunnel_connection_id.trim() || null,
    };
    const allowed = await confirmProdNetworkBridge(candidate, 'enable', ide.target.workspace_id);
    if (!allowed) {
      draft.enabled = false;
      return false;
    }
    draft.enabled = true;
    markDirty();
    return true;
  }

  async function handleClusterChange(nextClusterId: string): Promise<void> {
    const prevClusterId = draft.k8s_cluster_id;
    const prevNamespace = draft.k8s_namespace;
    const wasEnabled = draft.enabled;
    draft.k8s_cluster_id = nextClusterId;
    if (nextClusterId) {
      const picked = clusters.find((c) => c.id === nextClusterId);
      if (!draft.k8s_namespace.trim()) {
        draft.k8s_namespace =
          picked?.default_namespace?.trim() ||
          picked?.known_namespaces?.[0]?.trim() ||
          'default';
      }
      if (isProdEnvironment(picked?.environment)) {
        const allowed = await setEnabledWithGuard(true);
        if (!allowed) {
          draft.k8s_cluster_id = prevClusterId;
          draft.k8s_namespace = prevNamespace;
          draft.enabled = wasEnabled;
        }
        return;
      }
      if (!draft.enabled) draft.enabled = true;
    }
    markDirty();
  }

  async function handleSshTunnelChange(nextConnId: string): Promise<void> {
    const prevConnId = draft.ssh_tunnel_connection_id;
    const wasEnabled = draft.enabled;
    draft.ssh_tunnel_connection_id = nextConnId;
    if (nextConnId) {
      const picked = effectiveSshConns.find((c) => c.id === nextConnId);
      if (isProdEnvironment(picked?.environment)) {
        const allowed = await setEnabledWithGuard(true);
        if (!allowed) {
          draft.ssh_tunnel_connection_id = prevConnId;
          draft.enabled = wasEnabled;
        }
        return;
      }
      if (!draft.enabled) draft.enabled = true;
    }
    markDirty();
  }

  function addMappingRow(preset?: { from_host: string; from_port: string; to_host: string; to_port: string }): void {
    mappingsOpen = true;
    draft.mappings = [
      ...draft.mappings,
      {
        rowId: `hm-${Date.now()}-${draft.mappings.length}`,
        from_host: preset?.from_host ?? '',
        from_port: preset?.from_port ?? '',
        to_host: preset?.to_host ?? '',
        to_port: preset?.to_port ?? '',
      },
    ];
    markDirty();
  }

  function removeMappingRow(rowId: string): void {
    draft.mappings = draft.mappings.filter((r) => r.rowId !== rowId);
    markDirty();
  }

  function mapServiceLocalhost(svc: IdeBridgedServiceSummary): void {
    const port = svc.ports[0];
    if (!port) return;
    const pStr = String(port);
    const exists = draft.mappings.some(
      (m) => m.from_host.trim() === 'localhost' && m.from_port.trim() === pStr,
    );
    if (exists) return;
    addMappingRow({ from_host: 'localhost', from_port: pStr, to_host: svc.name, to_port: pStr });
  }

  async function runBridgeProbe(): Promise<void> {
    probeError = '';
    probeResult = null;
    const probeDraft: IdeNetworkBridgeDraft = { ...draft, enabled: true };
    const { config, error } = serializeNetworkBridgeDraft(probeDraft);
    if (error) {
      probeError = error;
      return;
    }
    testing = true;
    try {
      const res = await api.long.post<IdeNetBridgeStatusResp>('/ide/environments/bridge-test', {
        target: ide.targetPayload,
        bridge: config,
      });
      probeResult = res;
      if (res.message) announce(res.message);
    } catch (e) {
      probeError = loadErrorText(e);
    } finally {
      testing = false;
    }
  }
</script>

<section
  class="nb-card"
  class:is-prod={draft.enabled && isProdSelected}
  aria-labelledby="{idPrefix}-heading"
  data-testid="ide-network-bridge-card"
>
  <div class="nb-head">
    <div class="nb-title-row">
      <Switch
        checked={draft.enabled}
        label="Enable Kubernetes & SSH network bridge for local Run & Debug"
        onchange={(next) => {
          void setEnabledWithGuard(next);
        }}
      />
      <div class="nb-copy">
        <div class="nb-headline">
          <h3 id="{idPrefix}-heading" class="nb-title">Cluster &amp; SSH network bridge (Run &amp; Debug)</h3>
          {#if draft.enabled}
            <Badge
              tone={isProdSelected ? 'bad' : 'ok'}
              dot
              label={selectedCluster
                ? `${isProdSelected ? 'PROD · ' : ''}${selectedCluster.name} · ${draft.k8s_namespace.trim() || 'default'}`
                : isProdSelected
                  ? 'PROD · Active'
                  : 'Active'}
            />
          {:else}
            <Badge tone="neutral" variant="outline" label="Off" />
          {/if}
        </div>
        <p class="nb-sub">
          Routes internal Kubernetes services (<code>config:9090</code>, <code>confsrv.stgco.local</code>,
          <code>&lt;svc&gt;.&lt;ns&gt;.svc.cluster.local</code>, Ingresses, ClusterIPs) and private VPC hosts
          from local processes without <code>sudo</code> or <code>/etc/hosts</code>.
        </p>
      </div>
    </div>

    <div class="nb-actions">
      <button
        type="button"
        class="btn small"
        disabled={testing || (!draft.k8s_cluster_id && !draft.ssh_tunnel_connection_id && draft.mappings.length === 0)}
        onclick={() => void runBridgeProbe()}
        data-testid="ide-netbridge-test-btn"
      >
        <Icon name="helm" size={12} />
        {testing ? 'Testing…' : 'Test & discover'}
      </button>
      {#if onSave}
        <button
          type="button"
          class="btn small"
          disabled={saving}
          onclick={onSave}
          data-testid="ide-netbridge-save-btn"
        >
          {saving ? 'Saving…' : 'Save bridge'}
        </button>
      {/if}
    </div>
  </div>

  <div class="nb-grid">
    <div class="field">
      <div class="nb-field-top">
        <label for="{idPrefix}-cluster">Kubernetes cluster</label>
        {#if selectedCluster}
          <EnvBadge env={selectedCluster.environment} />
        {/if}
      </div>
      <select
        id="{idPrefix}-cluster"
        class="input"
        value={draft.k8s_cluster_id}
        onchange={(e) => void handleClusterChange((e.currentTarget as HTMLSelectElement).value)}
        data-testid="ide-netbridge-cluster-select"
      >
        <option value="">{clustersLoading ? 'Fetching clusters…' : 'None (no K8s cluster)'}</option>
        {#each clusters as c (c.id)}
          <option value={c.id}>{c.name} ({c.environment} · {c.context_name})</option>
        {/each}
      </select>
      {#if clustersError}
        <LoadState
          variant="compact"
          what="Kubernetes clusters"
          error={clustersError}
          empty
          onretry={() => void loadClusters()}
        />
      {:else if !clustersLoading && clusters.length === 0}
        <div class="nb-empty-link">
          <span class="nb-note">No Kubernetes clusters registered yet.</span>
          <button type="button" class="btn small ghost" onclick={() => router.go('kubernetes')}>
            Open Kubernetes…
          </button>
        </div>
      {/if}
    </div>

    <div class="field">
      <label for="{idPrefix}-ns">Kubernetes namespace</label>
      <input
        id="{idPrefix}-ns"
        class="input"
        type="text"
        dir="ltr"
        list="{idPrefix}-ns-datalist"
        placeholder={selectedCluster?.default_namespace || 'default (e.g. cbo, mscasino)'}
        bind:value={draft.k8s_namespace}
        oninput={markDirty}
        data-testid="ide-netbridge-namespace-input"
      />
      <datalist id="{idPrefix}-ns-datalist">
        {#each namespaceChips as ns (ns)}
          <option value={ns}>{ns}</option>
        {/each}
      </datalist>
      {#if namespacesError && draft.k8s_cluster_id.trim()}
        <LoadState
          variant="compact"
          what="cluster namespaces"
          error={namespacesError}
          empty
          onretry={() => void loadClusterNamespaces(draft.k8s_cluster_id.trim())}
        />
      {/if}
      {#if selectedCluster && namespaceChips.length > 0}
        <div class="nb-chip-row">
          {#each namespaceChips as ns (ns)}
            <button
              type="button"
              class="chip"
              aria-pressed={draft.k8s_namespace.trim() === ns}
              onclick={() => {
                draft.k8s_namespace = ns;
                if (!draft.enabled) void setEnabledWithGuard(true);
                else markDirty();
              }}
            >
              {ns}
            </button>
          {/each}
        </div>
      {/if}
    </div>

    <div class="field">
      <div class="nb-field-top">
        <label for="{idPrefix}-ssh">SSH bastion SOCKS tunnel (optional)</label>
        {#if selectedSsh}
          <EnvBadge env={selectedSsh.environment} readOnly={selectedSsh.read_only} />
        {/if}
      </div>
      <select
        id="{idPrefix}-ssh"
        class="input"
        value={draft.ssh_tunnel_connection_id}
        onchange={(e) => void handleSshTunnelChange((e.currentTarget as HTMLSelectElement).value)}
        data-testid="ide-netbridge-ssh-select"
      >
        <option value="">{sshLoading ? 'Fetching SSH hosts…' : 'None (direct / K8s only)'}</option>
        {#each effectiveSshConns as c (c.id)}
          <option value={c.id}>{c.name} ({c.environment})</option>
        {/each}
      </select>
      {#if sshError && sshConnections === undefined}
        <LoadState
          variant="compact"
          what="SSH connections"
          error={sshError}
          empty
          onretry={() => void loadWorkspaceSshConns()}
        />
      {:else if !sshLoading && effectiveSshConns.length === 0}
        <div class="nb-empty-link">
          <span class="nb-note">No SSH hosts configured yet.</span>
          <button type="button" class="btn small ghost" onclick={() => router.go('connections')}>
            Open Connections…
          </button>
        </div>
      {/if}
    </div>

    <div class="nb-flags">
      <div class="nb-flag-item">
        <Switch
          checked={draft.inject_k8s_service_env}
          label="Inject Kubernetes *_SERVICE_HOST and POD_NAMESPACE variables"
          onchange={(next) => {
            draft.inject_k8s_service_env = next;
            markDirty();
          }}
        />
        <span>Inject <code>*_SERVICE_HOST</code> &amp; <code>POD_NAMESPACE</code></span>
      </div>
      <div class="nb-flag-item">
        <Switch
          checked={draft.containerized}
          label="Export CONTAINERIZED=true for local config loaders"
          onchange={(next) => {
            draft.containerized = next;
            markDirty();
          }}
        />
        <span>Export <code>CONTAINERIZED=true</code></span>
      </div>
    </div>
  </div>

  <div class="nb-map-bar">
    <button
      type="button"
      class="btn small ghost"
      aria-expanded={mappingsOpen}
      onclick={() => (mappingsOpen = !mappingsOpen)}
    >
      <Icon name={mappingsOpen ? 'chevronUp' : 'chevronDown'} size={12} />
      Host &amp; localhost port overrides ({draft.mappings.length})
    </button>
    <button type="button" class="btn small ghost" onclick={() => addMappingRow()}>
      <Icon name="plus" size={12} />
      Add override
    </button>
  </div>

  {#if mappingsOpen}
    <div class="nb-map-list">
      {#if draft.mappings.length === 0}
        <p class="nb-note">
          Optional: pin <code>localhost:9090 → confsrv:9090</code> or map custom external hostnames to cluster services.
        </p>
      {:else}
        {#each draft.mappings as row (row.rowId)}
          <div class="nb-map-row" dir="ltr">
            <input
              class="input nb-host-in"
              type="text"
              dir="ltr"
              aria-label="Source hostname"
              placeholder="localhost or confsrv.stgco.local"
              bind:value={row.from_host}
              oninput={markDirty}
            />
            <input
              class="input nb-port-in"
              type="text"
              inputmode="numeric"
              dir="ltr"
              aria-label="Source port"
              placeholder="port"
              bind:value={row.from_port}
              oninput={markDirty}
            />
            <span class="nb-arrow" aria-hidden="true"><Icon name="chevronRight" size={12} noflip /></span>
            <input
              class="input nb-host-in"
              type="text"
              dir="ltr"
              aria-label="Destination service or host"
              placeholder="confsrv or db.internal"
              bind:value={row.to_host}
              oninput={markDirty}
            />
            <input
              class="input nb-port-in"
              type="text"
              inputmode="numeric"
              dir="ltr"
              aria-label="Destination port"
              placeholder="port"
              bind:value={row.to_port}
              oninput={markDirty}
            />
            <button
              type="button"
              class="icon-btn"
              aria-label="Remove host override"
              title="Remove host override"
              onclick={() => removeMappingRow(row.rowId)}
            >
              <Icon name="trash" size={13} />
            </button>
          </div>
        {/each}
      {/if}
    </div>
  {/if}

  {#if probeError}
    <div class="nb-err" use:announceShown={{ text: probeError, assertive: true }}>{probeError}</div>
  {/if}

  {#if probeResult}
    <div class="nb-probe" data-testid="ide-netbridge-probe-result">
      <div class="nb-probe-head">
        <Badge tone={probeResult.ok ? 'ok' : 'bad'} label={probeResult.ok ? 'Connected' : 'Check failed'} />
        <span class="nb-probe-msg">{probeResult.message}</span>
        <span class="nb-probe-lat">{probeResult.latency_ms} ms</span>
      </div>
      {#if probeResult.services.length > 0}
        <div class="nb-svc-list" role="list" aria-label="Discovered Kubernetes services">
          {#each probeResult.services.slice(0, 24) as svc (`${svc.namespace}/${svc.name}`)}
            <div class="nb-svc-pill" role="listitem">
              <span class="nb-svc-name" dir="ltr">
                {svc.name}{svc.ports.length > 0 ? `:${svc.ports.join(',')}` : ''}
              </span>
              {#if svc.ports.length > 0}
                <button
                  type="button"
                  class="btn small ghost"
                  title="Pin localhost:{svc.ports[0]} to {svc.name}:{svc.ports[0]}"
                  onclick={() => mapServiceLocalhost(svc)}
                >
                  + localhost:{svc.ports[0]}
                </button>
              {/if}
            </div>
          {/each}
        </div>
        {#if probeResult.services.length > 24}
          <p class="nb-note">
            +{plural(probeResult.services.length - 24, 'more service')} reachable automatically via DNS &amp; SOCKS5.
          </p>
        {/if}
      {/if}
    </div>
  {/if}
</section>

<style>
  .nb-card {
    display: flex; flex-direction: column; gap: 10px; padding: 12px;
    border-radius: var(--radius-m); background: var(--surface-2); border: 1px solid var(--border);
  }
  .nb-card.is-prod {
    border-color: color-mix(in srgb, var(--danger) 40%, var(--border));
    background: color-mix(in srgb, var(--danger-soft) 45%, var(--surface-2));
  }
  .nb-head { display: flex; flex-wrap: wrap; align-items: flex-start; justify-content: space-between; gap: 10px; }
  .nb-title-row { display: flex; align-items: flex-start; gap: 10px; flex: 1; min-inline-size: 240px; }
  .nb-copy { display: flex; flex-direction: column; gap: 4px; min-inline-size: 0; }
  .nb-headline { display: flex; flex-wrap: wrap; align-items: center; gap: 8px; }
  .nb-title { margin: 0; font-size: var(--fs-s); font-weight: 600; color: var(--text); }
  .nb-sub, .nb-note { margin: 0; font-size: var(--fs-xs); color: var(--text-dim); line-height: 1.4; }
  .nb-sub code, .nb-note code, .nb-flag-item code { font-family: var(--font-mono); font-size: var(--fs-xs); color: var(--text); }
  .nb-actions { display: flex; flex-wrap: wrap; align-items: center; gap: 6px; }
  .nb-grid { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 10px; align-items: start; }
  .nb-field-top { display: flex; align-items: center; justify-content: space-between; gap: 6px; }
  .nb-empty-link { display: flex; flex-wrap: wrap; align-items: center; justify-content: space-between; gap: 6px; margin-block-start: 4px; }
  .nb-chip-row { display: flex; flex-wrap: wrap; gap: 6px; margin-block-start: 4px; }
  .nb-flags { display: flex; flex-direction: column; gap: 6px; padding-block-start: 4px; }
  .nb-flag-item { display: flex; align-items: center; gap: 8px; font-size: var(--fs-xs); color: var(--text); }
  .nb-map-bar { display: flex; flex-wrap: wrap; align-items: center; justify-content: space-between; gap: 6px; }
  .nb-map-list {
    display: flex; flex-direction: column; gap: 6px; padding: 8px;
    border-radius: var(--radius-s); background: var(--surface); border: 1px solid var(--border);
  }
  .nb-map-row { display: flex; align-items: center; gap: 6px; min-inline-size: 0; }
  .nb-host-in { flex: 1; min-inline-size: 96px; font-family: var(--font-mono); }
  .nb-port-in { inline-size: 68px; flex-shrink: 0; font-family: var(--font-mono); }
  .nb-arrow { display: inline-flex; align-items: center; color: var(--text-dim); flex-shrink: 0; }
  .nb-err { font-size: var(--fs-xs); color: var(--danger); }
  .nb-probe {
    display: flex; flex-direction: column; gap: 8px; padding: 8px;
    border-radius: var(--radius-s); background: var(--surface); border: 1px solid var(--border);
  }
  .nb-probe-head { display: flex; flex-wrap: wrap; align-items: center; gap: 8px; font-size: var(--fs-xs); }
  .nb-probe-msg { flex: 1; min-inline-size: 160px; color: var(--text); }
  .nb-probe-lat { font-family: var(--font-mono); color: var(--text-dim); }
  .nb-svc-list { display: flex; flex-wrap: wrap; gap: 6px; max-block-size: 132px; overflow-y: auto; }
  .nb-svc-pill {
    display: inline-flex; align-items: center; gap: 4px; padding-block: 2px; padding-inline: 6px;
    border-radius: var(--radius-s); background: var(--surface-2); border: 1px solid var(--border);
  }
  .nb-svc-name { font-family: var(--font-mono); font-size: var(--fs-xs); color: var(--text); }

  @media (max-width: 640px) {
    .nb-grid { grid-template-columns: 1fr; }
    .nb-map-row { flex-wrap: wrap; }
  }
</style>
