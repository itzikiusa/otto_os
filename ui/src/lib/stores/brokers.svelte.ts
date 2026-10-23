// Message Brokers (Kafka) store: cluster list + selection. Tab components fetch
// their own per-cluster data (topics, groups, metrics) on demand.

import { api } from '../api/client';
import type { BrokerCluster, ConnectionSection, Id, UpsertClusterReq } from '../api/types';
import { ws } from './workspace.svelte';
import { loadErrorText } from '../loadError';
import { latestOnly } from '../latest';

/** The per-cluster views, in tab order. Lives in lib/ (not the brokers
 *  module) because the agent UI commands (lib/uiCommands/brokers.ts) address
 *  these tabs too, and lib/ never imports from modules/. */
export const CLUSTER_VIEW_IDS = ['overview', 'topics', 'groups', 'schema', 'replay', 'alerts'] as const;
export type ClusterView = (typeof CLUSTER_VIEW_IDS)[number];

class BrokersStore {
  clusters: BrokerCluster[] = $state([]);
  /** User-defined sidebar sections (folders), a nestable tree via parent_id.
   *  Shared with Connections + the DB Explorer — the ONE global connection-section
   *  tree; clusters carry `section_id` into it (see the unified Connections hub). */
  sections: ConnectionSection[] = $state([]);
  selectedId: Id | null = $state(null);
  /** Clusters opened as tabs (Workbench-style), in tab order. */
  openIds: Id[] = $state([]);
  loading = $state(false);
  /** Cause of the last failed cluster-list load (null = ok); rendered inline with Retry. */
  loadError: string | null = $state(null);

  selected: BrokerCluster | null = $derived(
    this.clusters.find((c) => c.id === this.selectedId) ?? null,
  );

  /** The open clusters resolved to objects, in tab order. */
  openClusters: BrokerCluster[] = $derived(
    this.openIds.map((id) => this.clusters.find((c) => c.id === id)).filter(Boolean) as BrokerCluster[],
  );

  /** The workspace the lists above belong to, and the latest-load guard: a
   *  switch A→B must never let A's slower response land under B — Test,
   *  Remove and Move act on these ids by id alone (`/brokers/clusters/{id}`). */
  private loadedWs: Id | null = null;
  private loadGen = latestOnly();

  async load(wsId: Id): Promise<void> {
    if (this.loadedWs !== wsId) {
      // Entering another workspace: drop the previous one's clusters, tabs and
      // sections so a failed load can't leave them listed under this one.
      this.clusters = [];
      this.sections = [];
      this.openIds = [];
      this.selectedId = null;
      this.loadError = null;
      this.loadedWs = wsId;
    }
    const t = this.loadGen.begin();
    const current = () => t.current && wsId === ws.currentId;
    this.loading = true;
    try {
      const clusters = await api.get<BrokerCluster[]>(`/workspaces/${wsId}/brokers/clusters`);
      if (!current()) return;
      this.clusters = clusters;
      this.loadError = null;
      // Prune tabs/selection for clusters that no longer exist.
      const exists = (id: Id) => clusters.some((c) => c.id === id);
      this.openIds = this.openIds.filter(exists);
      if (this.selectedId && !exists(this.selectedId)) this.selectedId = null;
      if (!this.selectedId && this.openIds.length > 0) this.selectedId = this.openIds[0];
    } catch (e) {
      // Inline (LoadState + Retry), never a toast — and this workspace's previous list stays.
      if (current()) this.loadError = loadErrorText(e);
    } finally {
      // A superseded load must not clear the spinner of the load that replaced it.
      if (current()) this.loading = false;
    }
    if (!current()) return;
    // Sections are best-effort: a failure shouldn't blank the cluster list.
    // Read the ONE global connection-section tree (shared with Connections + DB).
    try {
      const sections = await api.get<ConnectionSection[]>(
        `/workspaces/${wsId}/connection-sections`,
      );
      if (current()) this.sections = sections;
    } catch {
      if (current()) this.sections = [];
    }
  }

  // ---- sections (sidebar grouping) ----------------------------------------
  // The section tree is the shared connection-section tree; these mirror the
  // request/response shapes the Connections page uses for the same endpoints.

  async createSection(parentId: Id | null, name: string): Promise<void> {
    if (!ws.currentId) return;
    const sec = await api.post<ConnectionSection>(
      `/workspaces/${ws.currentId}/connection-sections`,
      { name, parent_id: parentId },
    );
    this.sections = [...this.sections, sec];
  }

  async renameSection(id: Id, name: string): Promise<void> {
    const updated = await api.patch<ConnectionSection>(`/connection-sections/${id}`, {
      name,
    });
    this.sections = this.sections.map((s) => (s.id === id ? updated : s));
  }

  async deleteSection(id: Id): Promise<void> {
    await api.del(`/connection-sections/${id}`);
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
    const updated = await api.post<ConnectionSection>(`/connection-sections/${id}/move`, {
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
