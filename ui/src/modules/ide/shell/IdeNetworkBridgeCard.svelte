<script lang="ts">
  // Cluster & SSH Network Bridge card shared by `IdeEnvironmentsModal` and
  // `IdeSshTargetModal`. Attaches a Kubernetes cluster + namespace and/or SSH
  // bastion tunnel to local Run & Debug sessions so cluster-internal hostnames
  // (`config:9090`, `confsrv.stgco.local`, `<svc>.<ns>.svc.cluster.local`,
  // ClusterIPs, Ingress hosts) resolve transparently without `sudo` or `/etc/hosts`.
  import { untrack } from 'svelte';
  import { api } from '../../../lib/api/client';
  import type { Connection, K8sCluster, K8sNamespace } from '../../../lib/api/types';
  import { announceShown } from '../../../lib/announce.svelte';
  import Badge from '../../../lib/components/Badge.svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import Switch from '../../../lib/components/Switch.svelte';
  import { loadErrorText } from '../../../lib/loadError';
  import { plural } from '../../../lib/plural';
  import { SCRATCH_WORKSPACE_ID, ws } from '../../../lib/stores/workspace.svelte';
  import { ide, type IdeBridgedServiceSummary, type IdeNetBridgeStatusResp } from '../ideStore.svelte';
  import {
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
  let loadedSshConns = $state<Connection[]>([]);
  let liveNamespaces = $state<K8sNamespace[]>([]);
  let nsLoadedForCluster = $state('');
  let mappingsOpen = $state(false);
  let testing = $state(false);
  let probeError = $state('');
  let probeResult = $state<IdeNetBridgeStatusResp | null>(null);

  const effectiveSshConns = $derived(sshConnections ?? loadedSshConns);
  const selectedCluster = $derived(
    clusters.find((c) => c.id === draft.k8s_cluster_id) ?? null,
  );
  const namespaceChips = $derived(
    mergeClusterNamespaceSuggestions(selectedCluster, liveNamespaces).slice(0, 8),
  );

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
    try {
      clusters = await api.get<K8sCluster[]>('/k8s/clusters');
    } catch {
      clusters = [];
    } finally {
      clustersLoading = false;
    }
  }

  async function loadClusterNamespaces(clusterId: string): Promise<void> {
    try {
      const resp = await api.get<{ namespaces: K8sNamespace[] }>(
        `/k8s/clusters/${encodeURIComponent(clusterId)}/namespaces`,
      );
      if (nsLoadedForCluster === clusterId) {
        liveNamespaces = resp.namespaces ?? [];
      }
    } catch {
      if (nsLoadedForCluster === clusterId) {
        liveNamespaces = [];
      }
    }
  }

  async function loadWorkspaceSshConns(): Promise<void> {
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
      for (const r of results) {
        if (r.status !== 'fulfilled') continue;
        for (const c of r.value) {
          if (c.kind === 'ssh' && !seen.has(c.id)) seen.set(c.id, c);
        }
      }
      loadedSshConns = Array.from(seen.values());
    } catch {
      loadedSshConns = [];
    }
  }

  function handleClusterChange(nextClusterId: string): void {
    draft.k8s_cluster_id = nextClusterId;
    if (nextClusterId) {
      const picked = clusters.find((c) => c.id === nextClusterId);
      if (!draft.k8s_namespace.trim()) {
        draft.k8s_namespace =
          picked?.default_namespace?.trim() ||
          picked?.known_namespaces?.[0]?.trim() ||
          'default';
      }
      if (!draft.enabled) draft.enabled = true;
    }
    markDirty();
  }

  function handleSshTunnelChange(nextConnId: string): void {
    draft.ssh_tunnel_connection_id = nextConnId;
    if (nextConnId && !draft.enabled) draft.enabled = true;
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
      probeResult = await api.long.post<IdeNetBridgeStatusResp>('/ide/environments/bridge-test', {
        target: ide.targetPayload,
        bridge: config,
      });
    } catch (e) {
      probeError = loadErrorText(e);
    } finally {
      testing = false;
    }
  }
</script>

<section class="nb-card" aria-labelledby="{idPrefix}-heading" data-testid="ide-network-bridge-card">
  <div class="nb-head">
    <div class="nb-title-row">
      <Switch
        checked={draft.enabled}
        label="Enable Kubernetes & SSH network bridge for local Run & Debug"
        onchange={(next) => {
          draft.enabled = next;
          markDirty();
        }}
      />
      <div class="nb-copy">
        <div class="nb-headline">
          <h3 id="{idPrefix}-heading" class="nb-title">Cluster &amp; SSH network bridge (Run &amp; Debug)</h3>
          {#if draft.enabled}
            <Badge
              tone="ok"
              dot
              label={selectedCluster
                ? `${selectedCluster.name} · ${draft.k8s_namespace.trim() || 'default'}`
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
          class="btn small primary"
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
      <label for="{idPrefix}-cluster">Kubernetes cluster</label>
      <select
        id="{idPrefix}-cluster"
        class="input"
        value={draft.k8s_cluster_id}
        onchange={(e) => handleClusterChange((e.currentTarget as HTMLSelectElement).value)}
        data-testid="ide-netbridge-cluster-select"
      >
        <option value="">{clustersLoading ? 'Fetching clusters…' : 'None (no K8s cluster)'}</option>
        {#each clusters as c (c.id)}
          <option value={c.id}>{c.name} ({c.environment} · {c.context_name})</option>
        {/each}
      </select>
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
      {#if selectedCluster && namespaceChips.length > 0}
        <div class="nb-chip-row">
          {#each namespaceChips as ns (ns)}
            <button
              type="button"
              class="chip"
              aria-pressed={draft.k8s_namespace.trim() === ns}
              onclick={() => {
                draft.k8s_namespace = ns;
                if (!draft.enabled) draft.enabled = true;
                markDirty();
              }}
            >
              {ns}
            </button>
          {/each}
        </div>
      {/if}
    </div>

    <div class="field">
      <label for="{idPrefix}-ssh">SSH bastion SOCKS tunnel (optional)</label>
      <select
        id="{idPrefix}-ssh"
        class="input"
        value={draft.ssh_tunnel_connection_id}
        onchange={(e) => handleSshTunnelChange((e.currentTarget as HTMLSelectElement).value)}
        data-testid="ide-netbridge-ssh-select"
      >
        <option value="">None (direct / K8s only)</option>
        {#each effectiveSshConns as c (c.id)}
          <option value={c.id}>{c.name} ({c.environment})</option>
        {/each}
      </select>
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
          <div class="nb-map-row">
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
            <span class="nb-arrow" aria-hidden="true">→</span>
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
  .nb-head { display: flex; flex-wrap: wrap; align-items: flex-start; justify-content: space-between; gap: 10px; }
  .nb-title-row { display: flex; align-items: flex-start; gap: 10px; flex: 1; min-inline-size: 240px; }
  .nb-copy { display: flex; flex-direction: column; gap: 4px; min-inline-size: 0; }
  .nb-headline { display: flex; flex-wrap: wrap; align-items: center; gap: 8px; }
  .nb-title { margin: 0; font-size: var(--fs-s); font-weight: 600; color: var(--text); }
  .nb-sub, .nb-note { margin: 0; font-size: var(--fs-xs); color: var(--text-dim); line-height: 1.4; }
  .nb-sub code, .nb-note code, .nb-flag-item code { font-family: var(--font-mono); font-size: var(--fs-xs); color: var(--text); }
  .nb-actions { display: flex; flex-wrap: wrap; align-items: center; gap: 6px; }
  .nb-grid { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 10px; align-items: start; }
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
  .nb-arrow { font-size: var(--fs-xs); color: var(--text-dim); flex-shrink: 0; }
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
