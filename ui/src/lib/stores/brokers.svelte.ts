// Message Brokers (Kafka) store: cluster list + selection. Tab components fetch
// their own per-cluster data (topics, groups, metrics) on demand.

import { api } from '../api/client';
import type { BrokerCluster, Id, UpsertClusterReq } from '../api/types';
import { ws } from './workspace.svelte';
import { toasts } from '../toast.svelte';

class BrokersStore {
  clusters: BrokerCluster[] = $state([]);
  selectedId: Id | null = $state(null);
  /** Clusters opened as tabs (Workbench-style), in tab order. */
  openIds: Id[] = $state([]);
  loading = $state(false);

  selected: BrokerCluster | null = $derived(
    this.clusters.find((c) => c.id === this.selectedId) ?? null,
  );

  /** The open clusters resolved to objects, in tab order. */
  openClusters: BrokerCluster[] = $derived(
    this.openIds.map((id) => this.clusters.find((c) => c.id === id)).filter(Boolean) as BrokerCluster[],
  );

  async load(wsId: Id): Promise<void> {
    this.loading = true;
    try {
      this.clusters = await api.get<BrokerCluster[]>(`/workspaces/${wsId}/brokers/clusters`);
      // Prune tabs/selection for clusters that no longer exist.
      const exists = (id: Id) => this.clusters.some((c) => c.id === id);
      this.openIds = this.openIds.filter(exists);
      if (this.selectedId && !exists(this.selectedId)) this.selectedId = null;
      if (!this.selectedId && this.openIds.length > 0) this.selectedId = this.openIds[0];
    } catch (e) {
      toasts.error('Failed to load clusters', String(e));
    } finally {
      this.loading = false;
    }
  }

  async refresh(): Promise<void> {
    if (ws.currentId) await this.load(ws.currentId);
  }

  /** Open a cluster as a tab (if not already) and make it active. */
  select(id: Id): void {
    if (!this.openIds.includes(id)) this.openIds = [...this.openIds, id];
    this.selectedId = id;
  }

  /** Close a cluster tab; activate a neighbour if it was the active one. */
  close(id: Id): void {
    const idx = this.openIds.indexOf(id);
    this.openIds = this.openIds.filter((x) => x !== id);
    if (this.selectedId === id) {
      this.selectedId = this.openIds[Math.min(idx, this.openIds.length - 1)] ?? null;
    }
  }

  async create(req: UpsertClusterReq): Promise<BrokerCluster | null> {
    if (!ws.currentId) return null;
    const cluster = await api.post<BrokerCluster>(
      `/workspaces/${ws.currentId}/brokers/clusters`,
      req,
    );
    await this.refresh();
    this.select(cluster.id);
    return cluster;
  }

  async update(id: Id, req: UpsertClusterReq): Promise<BrokerCluster> {
    const cluster = await api.patch<BrokerCluster>(`/brokers/clusters/${id}`, req);
    await this.refresh();
    return cluster;
  }

  async remove(id: Id): Promise<void> {
    await api.del(`/brokers/clusters/${id}`);
    this.close(id);
    await this.refresh();
  }
}

export const brokers = new BrokersStore();
