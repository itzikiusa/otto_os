// Message Brokers (Kafka) store: cluster list + selection. Tab components fetch
// their own per-cluster data (topics, groups, metrics) on demand.

import { api } from '../api/client';
import type { BrokerCluster, Id, UpsertClusterReq } from '../api/types';
import { ws } from './workspace.svelte';
import { toasts } from '../toast.svelte';

class BrokersStore {
  clusters: BrokerCluster[] = $state([]);
  selectedId: Id | null = $state(null);
  loading = $state(false);

  selected: BrokerCluster | null = $derived(
    this.clusters.find((c) => c.id === this.selectedId) ?? null,
  );

  async load(wsId: Id): Promise<void> {
    this.loading = true;
    try {
      this.clusters = await api.get<BrokerCluster[]>(`/workspaces/${wsId}/brokers/clusters`);
      if (this.selectedId && !this.clusters.some((c) => c.id === this.selectedId)) {
        this.selectedId = null;
      }
      if (!this.selectedId && this.clusters.length > 0) {
        this.selectedId = this.clusters[0].id;
      }
    } catch (e) {
      toasts.error('Failed to load clusters', String(e));
    } finally {
      this.loading = false;
    }
  }

  async refresh(): Promise<void> {
    if (ws.currentId) await this.load(ws.currentId);
  }

  select(id: Id): void {
    this.selectedId = id;
  }

  async create(req: UpsertClusterReq): Promise<BrokerCluster | null> {
    if (!ws.currentId) return null;
    const cluster = await api.post<BrokerCluster>(
      `/workspaces/${ws.currentId}/brokers/clusters`,
      req,
    );
    await this.refresh();
    this.selectedId = cluster.id;
    return cluster;
  }

  async update(id: Id, req: UpsertClusterReq): Promise<BrokerCluster> {
    const cluster = await api.patch<BrokerCluster>(`/brokers/clusters/${id}`, req);
    await this.refresh();
    return cluster;
  }

  async remove(id: Id): Promise<void> {
    await api.del(`/brokers/clusters/${id}`);
    if (this.selectedId === id) this.selectedId = null;
    await this.refresh();
  }
}

export const brokers = new BrokersStore();
