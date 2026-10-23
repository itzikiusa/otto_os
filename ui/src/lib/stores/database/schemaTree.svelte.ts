// Database Explorer store — schema tree: the root load, lazy children,
// expansion, per-node prefix filters, row-count toggle, the server-side object
// search and the builder catalog (full database / table enumeration for the
// visual JOIN canvas). The DatabaseStore façade owns one instance as `tree`
// and delegates its public methods here; the tree's reactive state stays on the
// façade, because a connection switch snapshots and restores it with the rest.

import { api, isAbortError } from '../../api/client';
import type { DbSchemaGraph, Id, ObjectSearchResult, SchemaNode } from '../../api/types';
import { loadErrorText } from '../../loadError';
import { toastError } from '../../toastError';
import { saveFlag } from '../dbPrefs.svelte';
import type { DatabaseStore } from '../database.svelte';

/** The DatabaseStore members this part reads and calls (its whole contract with the façade). */
export type SchemaTreeHost = Pick<
  DatabaseStore,
  | 'activeDb'
  | 'builderTablesCache'
  | 'childrenCache'
  | 'conn'
  | 'connBase'
  | 'epochs'
  | 'expanded'
  | 'loadingNodes'
  | 'nodeFilters'
  | 'objectSearchHits'
  | 'objectSearchQuery'
  | 'objectSearchScanned'
  | 'objectSearchScope'
  | 'objectSearchTruncated'
  | 'objectSearching'
  | 'refreshCompletions'
  | 'schemaLoading'
  | 'schemaRoot'
  | 'selectedConnId'
  | 'showCounts'
  | 'sidebarCollapsed'
>;

export class DbSchemaTreeStore {
  private readonly db: SchemaTreeHost;
  constructor(db: SchemaTreeHost) {
    this.db = db;
  }

  async loadSchemaRoot(id: Id): Promise<void> {
    const epoch = this.db.epochs.of(id);
    this.db.schemaLoading = true;
    // Preserve any prior serverVersion/latency across a refresh's connecting dip.
    this.db.conn.mergeConnStatus(id, { phase: 'connecting', error: undefined });
    const started =
      typeof performance !== 'undefined' ? performance.now() : Date.now();
    try {
      const root = await api.get<SchemaNode[]>(`${this.db.connBase(id)}/schema`);
      // Closed mid-fetch: write NOTHING (a status merge would resurrect the
      // connStatus entry closeConnection just deleted).
      if (!this.db.epochs.live(id, epoch)) return;
      // The singleton tree belongs to the SELECTED connection only — a
      // background load (tab switched away mid-fetch) must not clobber it; its
      // snapshot re-fetches on restore (see restoreSnapshot's empty-root path).
      if (this.db.selectedConnId === id) {
        this.db.schemaRoot = root;
        // Redis: default the active keyspace to the first DB (db0) so commands have
        // a clear, visible target and the tree marks it. Won't override a restored
        // selection. (`kind === 'keyspace'` only matches Redis.)
        if (!this.db.activeDb) {
          const ks = root.find((n) => n.kind === 'keyspace');
          if (ks) this.db.activeDb = ks.id;
        }
      }
      // Ready — record the schema round-trip as an immediate latency figure for
      // the health chip (the db/test probe refines it + adds the server version).
      const now = typeof performance !== 'undefined' ? performance.now() : Date.now();
      this.db.conn.mergeConnStatus(id, {
        phase: 'ready',
        error: undefined,
        latencyMs: Math.round(now - started),
      });
    } catch (e) {
      if (!this.db.epochs.live(id, epoch)) return; // closed tab: no status, no toast
      // A hard failure drops any stale health data (replace, not merge).
      // SchemaTree renders this phase inline ("Couldn’t connect" + Retry) — no
      // toast on top of it.
      this.db.conn.setConnStatus(id, { phase: 'error', error: loadErrorText(e) });
    } finally {
      // The singleton loading flag tracks the selected connection's tree.
      if (this.db.selectedConnId === id) this.db.schemaLoading = false;
    }
  }

  /** Re-fetch the schema root, clearing the children + completion caches. */
  async refreshSchema(): Promise<void> {
    if (!this.db.selectedConnId) return;
    this.db.childrenCache = new Map();
    this.db.builderTablesCache = new Map();
    this.db.expanded = new Set();
    // Clear the daemon's smart-completion snapshot too so suggestions reflect
    // the schema the user just refreshed (fire-and-forget; non-blocking).
    void this.db.refreshCompletions(this.db.selectedConnId);
    await this.loadSchemaRoot(this.db.selectedConnId);
  }

  // ── Tree expansion ──────────────────────────────────────────────────────

  isExpanded(nodeId: string): boolean {
    return this.db.expanded.has(nodeId);
  }
  childrenOf(nodeId: string): SchemaNode[] | undefined {
    return this.db.childrenCache.get(nodeId);
  }
  isLoadingNode(nodeId: string): boolean {
    return this.db.loadingNodes.has(nodeId);
  }

  /** Toggle/lazy-load a node's children. */
  async expand(node: SchemaNode): Promise<void> {
    const id = this.db.selectedConnId;
    if (!id || !node.has_children) return;

    if (this.db.expanded.has(node.id)) {
      this.db.expanded.delete(node.id);
      this.db.expanded = new Set(this.db.expanded);
      return;
    }
    this.db.expanded.add(node.id);
    this.db.expanded = new Set(this.db.expanded);

    if (this.db.childrenCache.has(node.id)) return; // already loaded
    await this.loadChildren(id, node.id);
  }

  /** Current prefix filter typed for a node (empty string when none). */
  nodeFilter(nodeId: string): string {
    return this.db.nodeFilters.get(nodeId) ?? '';
  }

  /**
   * Apply (or clear) a prefix filter on a node and reload its children. Used by
   * the Redis keyspace filter so huge databases load a narrowed, bounded set
   * instead of attempting to list every key.
   */
  async applyNodeFilter(node: SchemaNode, value: string): Promise<void> {
    const id = this.db.selectedConnId;
    if (!id) return;
    const v = value.trim();
    if (v) this.db.nodeFilters.set(node.id, v);
    else this.db.nodeFilters.delete(node.id);
    this.db.nodeFilters = new Map(this.db.nodeFilters);

    // Bust the cache and (re)load with the new filter; keep the node expanded.
    this.db.childrenCache.delete(node.id);
    this.db.childrenCache = new Map(this.db.childrenCache);
    this.db.expanded.add(node.id);
    this.db.expanded = new Set(this.db.expanded);
    await this.loadChildren(id, node.id);
  }

  /** Fetch a node's children (honouring any stored filter) into the cache. */
  /** Toggle sticky row estimates and drop the cache so the tree refetches. */
  setShowCounts(on: boolean): void {
    this.db.showCounts = on;
    saveFlag('db.showCounts', on);
    // Cached children were fetched without (or with) counts — drop the cache and
    // refetch what's on screen, or every expanded node would sit childless with
    // no spinner and no way to reload short of collapsing it by hand.
    this.db.childrenCache = new Map();
    const id = this.db.selectedConnId;
    if (!id) return;
    for (const nodeId of this.db.expanded) void this.loadChildren(id, nodeId);
  }

  toggleSidebar(): void {
    this.db.sidebarCollapsed = !this.db.sidebarCollapsed;
    saveFlag('db.sidebarCollapsed', this.db.sidebarCollapsed);
  }

  /**
   * Server-side object lookup. The tree's own filter only ever sees nodes that
   * are already loaded, so finding a table inside a never-opened schema has to
   * go to the catalog. A blank needle clears the hits instead of searching.
   */
  async searchObjects(q: string, schema?: string): Promise<void> {
    const connId = this.db.selectedConnId;
    if (!connId || !q.trim()) {
      this.db.objectSearchHits = null;
      this.db.objectSearchTruncated = false;
      this.db.objectSearchScanned = 0;
      return;
    }
    const seq = ++this.objectSearchSeq;
    // A newer keystroke ABORTS the superseded catalog search instead of only
    // ignoring its answer (the daemon stops scanning a big catalog early).
    this.objectSearchAbort?.abort();
    const ac = (this.objectSearchAbort = new AbortController());
    this.db.objectSearching = true;
    try {
      const r = await api.post<ObjectSearchResult>(
        `${this.db.connBase(connId)}/search-objects`,
        {
          q: q.trim(),
          schema: this.db.objectSearchScope === 'schema' ? schema : undefined,
          scope: this.db.objectSearchScope,
        },
        ac.signal,
      );
      if (seq !== this.objectSearchSeq) return; // a newer keystroke won
      this.db.objectSearchHits = r.hits;
      this.db.objectSearchTruncated = r.truncated;
      this.db.objectSearchScanned = r.scanned;
    } catch (e) {
      if (seq === this.objectSearchSeq && !isAbortError(e)) {
        this.db.objectSearchHits = [];
        toastError('Couldn’t object search', e);
      }
    } finally {
      if (seq === this.objectSearchSeq) {
        this.db.objectSearching = false;
        this.objectSearchAbort = null;
      }
    }
  }

  clearObjectSearch(): void {
    this.objectSearchAbort?.abort();
    this.objectSearchAbort = null;
    this.objectSearchSeq++;
    this.db.objectSearchQuery = '';
    this.db.objectSearchHits = null;
    this.db.objectSearching = false;
    this.db.objectSearchTruncated = false;
    this.db.objectSearchScanned = 0;
  }

  objectSearchSeq = 0;
  private objectSearchAbort: AbortController | null = null;

  private async loadChildren(connId: string, nodeId: string): Promise<void> {
    const epoch = this.db.epochs.of(connId);
    // The tree singletons (children/expanded/loading) always describe the
    // SELECTED connection — after any await, a completion for a connection
    // that was closed or switched away must not touch them.
    const stale = (): boolean => !this.db.epochs.live(connId, epoch) || this.db.selectedConnId !== connId;
    this.db.loadingNodes.add(nodeId);
    this.db.loadingNodes = new Set(this.db.loadingNodes);
    try {
      const filter = this.db.nodeFilters.get(nodeId);
      const children = await api.post<SchemaNode[]>(`${this.db.connBase(connId)}/schema/children`, {
        path: nodeId,
        filter: filter || undefined,
        counts: this.db.showCounts || undefined,
      });
      if (stale()) return;
      this.db.childrenCache.set(nodeId, children);
      this.db.childrenCache = new Map(this.db.childrenCache);
    } catch (e) {
      if (stale()) return; // no toast / collapse for a dead view
      toastError('Couldn’t load children', e);
      this.db.expanded.delete(nodeId);
      this.db.expanded = new Set(this.db.expanded);
    } finally {
      if (!stale()) {
        this.db.loadingNodes.delete(nodeId);
        this.db.loadingNodes = new Set(this.db.loadingNodes);
      }
    }
  }

  /**
   * Fetch the relationship graph (ERD) for a schema/database: tables (with
   * columns + PK/FK flags) and the FK edges between them. Read-only; backed by
   * the same introspection the tree uses. `maxTables` caps the fan-out.
   */
  /** The ERD graph for `schema`. Null ONLY with no connection selected; a
   *  failed load throws so the diagram renders it inline with Retry — a null
   *  must never read as "No schema selected" (same rule as listBuilderTables). */
  async fetchSchemaGraph(schema: string, maxTables = 60): Promise<DbSchemaGraph | null> {
    const id = this.db.selectedConnId;
    if (!id) return null;
    return await api.post<DbSchemaGraph>(`${this.db.connBase(id)}/schema-graph`, {
      schema,
      max_tables: maxTables,
    });
  }

  // ── Builder catalog (full enumeration for the visual JOIN canvas) ────────

  /**
   * Flat list of databases for the active connection. Sourced from the schema
   * root (kind === 'database'). When the engine exposes no database level
   * (single implicit db), returns one empty entry so the palette still works.
   */
  async listBuilderDatabases(): Promise<{ name: string; path: string }[]> {
    const dbs = this.db.schemaRoot
      .filter((n) => n.kind === 'database')
      .map((n) => ({ name: n.label, path: n.id }));
    return dbs.length ? dbs : [{ name: '', path: '' }];
  }

  /**
   * Flat list of tables + views in a database, resolving any intermediate
   * Folder nodes (MySQL returns `folder:tables`/`folder:views`; ClickHouse
   * returns tables directly). Cached per db path.
   */
  /** The tables/views of `dbPath`, cached per path. THROWS on a failed load:
   *  the palette renders it inline with Retry and the import path names the
   *  database — an empty list must only ever mean "this database has none". */
  async listBuilderTables(dbPath: string): Promise<{ label: string; path: string; kind: string }[]> {
    const id = this.db.selectedConnId;
    if (!id) return [];
    const cached = this.db.builderTablesCache.get(dbPath);
    if (cached) return cached;
    const out: { label: string; path: string; kind: string }[] = [];
    const seen = new Set<string>();
    // For an empty implicit-db path, query the schema root's children path.
    const first = await api.post<SchemaNode[]>(`${this.db.connBase(id)}/schema/children`, {
      path: dbPath,
    });
    for (const node of first) {
      if (node.kind === 'folder') {
        const kids = await api.post<SchemaNode[]>(`${this.db.connBase(id)}/schema/children`, {
          path: node.id,
        });
        for (const k of kids) {
          if ((k.kind === 'table' || k.kind === 'view') && !seen.has(k.id)) {
            seen.add(k.id);
            out.push({ label: k.label, path: k.id, kind: k.kind });
          }
        }
      } else if ((node.kind === 'table' || node.kind === 'view') && !seen.has(node.id)) {
        seen.add(node.id);
        out.push({ label: node.label, path: node.id, kind: node.kind });
      }
    }
    out.sort((a, b) => a.label.localeCompare(b.label));
    this.db.builderTablesCache.set(dbPath, out);
    this.db.builderTablesCache = new Map(this.db.builderTablesCache);
    return out;
  }
}
