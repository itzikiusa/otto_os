// Message Brokers (Kafka) store: cluster list + selection. Tab components fetch
// their own per-cluster data (topics, groups, metrics) on demand.

import { api } from '../api/client';
import type { BrokerCluster, BrokerClusterSection, Id, UpsertClusterReq } from '../api/types';
import { ws } from './workspace.svelte';
import { toasts } from '../toast.svelte';

class BrokersStore {
  clusters: BrokerCluster[] = $state([]);
  /** User-defined sidebar sections (folders), a nestable tree via parent_id. */
  sections: BrokerClusterSection[] = $state([]);
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
    // Sections are best-effort: a failure shouldn't blank the cluster list.
    try {
      this.sections = await api.get<BrokerClusterSection[]>(
        `/workspaces/${wsId}/brokers/cluster-sections`,
      );
    } catch {
      this.sections = [];
    }
  }

  // ---- sections (sidebar grouping) ----------------------------------------

  async createSection(parentId: Id | null, name: string): Promise<void> {
    if (!ws.currentId) return;
    const sec = await api.post<BrokerClusterSection>(
      `/workspaces/${ws.currentId}/brokers/cluster-sections`,
      { name, parent_id: parentId },
    );
    this.sections = [...this.sections, sec];
  }

  async renameSection(id: Id, name: string): Promise<void> {
    const updated = await api.patch<BrokerClusterSection>(`/brokers/cluster-sections/${id}`, {
      name,
    });
    this.sections = this.sections.map((s) => (s.id === id ? updated : s));
  }

  async deleteSection(id: Id): Promise<void> {
    await api.del(`/brokers/cluster-sections/${id}`);
    // Drop the section + descendants locally; ungroup their clusters.
    const removed = new Set<Id>();
    const collect = (sid: Id) => {
      removed.add(sid);
      for (const s of this.sections) if (s.parent_id === sid) collect(s.id);
    };
    collect(id);
    this.sections = this.sections.filter((s) => !removed.has(s.id));
    this.clusters = this.clusters.map((c) =>
      c.section_id && removed.has(c.section_id) ? { ...c, section_id: null } : c,
    );
  }

  async reparentSection(id: Id, parentId: Id | null): Promise<void> {
    const updated = await api.post<BrokerClusterSection>(`/brokers/cluster-sections/${id}/move`, {
      parent_id: parentId,
    });
    this.sections = this.sections.map((s) => (s.id === id ? updated : s));
  }

  /** File a cluster into a section (null = ungrouped). Sends the full non-secret
   *  state (+ section_id); omitted passwords/ssh are kept by PATCH semantics. */
  async moveCluster(id: Id, sectionId: Id | null): Promise<void> {
    const c = this.clusters.find((x) => x.id === id);
    if (!c || (c.section_id ?? null) === sectionId) return;
    const saved = await api.patch<BrokerCluster>(`/brokers/clusters/${id}`, {
      name: c.name,
      bootstrap_servers: c.bootstrap_servers,
      security_protocol: c.security_protocol,
      sasl_mechanism: c.sasl_mechanism,
      sasl_username: c.sasl_username,
      tls_skip_verify: c.tls_skip_verify,
      schema_registry_url: c.schema_registry_url,
      schema_registry_username: c.schema_registry_username,
      metrics_url: c.metrics_url,
      color: c.color,
      environment: c.environment,
      read_only: c.read_only,
      section_id: sectionId,
    } as UpsertClusterReq);
    this.clusters = this.clusters.map((x) => (x.id === id ? saved : x));
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
