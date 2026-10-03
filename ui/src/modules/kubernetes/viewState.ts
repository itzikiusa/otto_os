// Pure view-state helpers for the Kubernetes module (K-1/K-2): the per-cluster
// UI blob the store persists, and the module's route grammar. No runes and no
// store imports, so `node --test` can exercise them directly.
import type { K8sResourceKind } from '../../lib/api/types';

export type K8sDrawerTab =
  | 'overview'
  | 'manifest'
  | 'describe'
  | 'events'
  | 'logs'
  | 'terminal'
  | 'metrics'
  | 'pods'
  | 'http';

const DRAWER_TABS: readonly K8sDrawerTab[] = ['overview', 'manifest', 'describe', 'events', 'logs', 'terminal', 'metrics', 'pods', 'http'];

/** Monitor view state for one cluster (K-1): kept in the store, persisted, so
 *  the Monitor comes back exactly as it was left after a module switch, a
 *  Resources ↔ Monitor switch or a reload. */
export interface K8sMonitorUi {
  /** '' = all configured namespaces. */
  ns: string;
  filter: string;
  sortKey: string;
  sortDir: 1 | -1;
  /** `<ns>/<workload>` of the expanded Trends row. */
  expanded: string | null;
  /** Events-tab class filter ('' = restarts + churn). */
  classFilter: string;
  scrollTop: number;
}

export const DEFAULT_MONITOR_UI: K8sMonitorUi = {
  ns: '',
  filter: '',
  sortKey: 'mem_max',
  sortDir: -1,
  expanded: null,
  classFilter: '',
  scrollTop: 0,
};

/** Console view state for one cluster that isn't in the URL (K-1). */
export interface K8sClusterUi {
  /** Text filter per kind — a filter belongs to the view it was typed in. */
  filters: Partial<Record<K8sResourceKind, string>>;
  drawerTab: K8sDrawerTab;
  /** Table scrollTop per `currentKey`. */
  scroll: Record<string, number>;
  monitor: K8sMonitorUi;
}

/** Parse a persisted per-cluster UI blob; anything malformed falls back to
 *  the defaults field by field (exported for unit tests). */
export function parseClusterUi(raw: string | null): K8sClusterUi {
  const out: K8sClusterUi = { filters: {}, drawerTab: 'overview', scroll: {}, monitor: { ...DEFAULT_MONITOR_UI } };
  if (!raw) return out;
  let v: unknown;
  try {
    v = JSON.parse(raw);
  } catch {
    return out;
  }
  if (!v || typeof v !== 'object') return out;
  const o = v as Record<string, unknown>;
  if (o.filters && typeof o.filters === 'object') {
    for (const [k, f] of Object.entries(o.filters as Record<string, unknown>)) {
      if (typeof f === 'string') out.filters[k as K8sResourceKind] = f;
    }
  }
  if (typeof o.drawerTab === 'string' && (DRAWER_TABS as readonly string[]).includes(o.drawerTab)) out.drawerTab = o.drawerTab as K8sDrawerTab;
  if (o.scroll && typeof o.scroll === 'object') {
    for (const [k, t] of Object.entries(o.scroll as Record<string, unknown>)) {
      if (typeof t === 'number' && Number.isFinite(t) && t > 0) out.scroll[k] = t;
    }
  }
  const m = o.monitor && typeof o.monitor === 'object' ? (o.monitor as Record<string, unknown>) : {};
  if (typeof m.ns === 'string') out.monitor.ns = m.ns;
  if (typeof m.filter === 'string') out.monitor.filter = m.filter;
  if (typeof m.sortKey === 'string') out.monitor.sortKey = m.sortKey;
  if (m.sortDir === 1 || m.sortDir === -1) out.monitor.sortDir = m.sortDir;
  if (typeof m.expanded === 'string' || m.expanded === null) out.monitor.expanded = m.expanded as string | null;
  if (typeof m.classFilter === 'string') out.monitor.classFilter = m.classFilter;
  if (typeof m.scrollTop === 'number' && Number.isFinite(m.scrollTop) && m.scrollTop > 0) out.monitor.scrollTop = m.scrollTop;
  return out;
}


/**
 * The Kubernetes module's routes (`router.parts`, already decoded):
 *
 * - `kubernetes` — clusters overview
 * - `kubernetes/monitor` — Monitor overview; `kubernetes/monitor/fleet[/<tab>]` — Fleet
 * - `kubernetes/<id>[/<kind>[/<ns>/<name>]]` — the cluster workspace, Resources view
 * - `kubernetes/<id>/monitor[/<tab>]` — the same workspace, Monitor view (K-2)
 * - `kubernetes/monitor/<id>[/<tab>]` — the old Monitor URL: `legacy`, which the
 *   page canonicalises (router.replace) to the workspace form above.
 */
export type K8sRoute =
  | { view: 'overview' }
  | { view: 'monitor-overview' }
  | { view: 'fleet'; tab: string }
  | { view: 'monitor'; clusterId: string; tab: string; legacy: boolean }
  | { view: 'resources'; clusterId: string; kind: string; ns?: string; name?: string };

export function parseK8sRoute(parts: readonly string[]): K8sRoute {
  const a = parts[1];
  if (!a) return { view: 'overview' };
  if (a === 'monitor') {
    const b = parts[2];
    if (!b) return { view: 'monitor-overview' };
    if (b === 'fleet') return { view: 'fleet', tab: parts[3] ?? 'overview' };
    return { view: 'monitor', clusterId: b, tab: parts[3] ?? 'workloads', legacy: true };
  }
  if (parts[2] === 'monitor') return { view: 'monitor', clusterId: a, tab: parts[3] ?? 'workloads', legacy: false };
  return { view: 'resources', clusterId: a, kind: parts[2] ?? '', ns: parts[3], name: parts[4] };
}

/** Route of a cluster's Monitor view. */
export function monitorPath(clusterId: string, tab = 'workloads'): string {
  return `kubernetes/${encodeURIComponent(clusterId)}/monitor/${tab}`;
}

/** Route of a cluster's Resources view (optionally a row → its drawer). */
export function resourcesPath(clusterId: string, kind?: string, ns?: string, name?: string): string {
  let p = `kubernetes/${encodeURIComponent(clusterId)}`;
  if (kind) p += `/${kind}`;
  if (kind && name !== undefined) p += `/${encodeURIComponent(ns || '-')}/${encodeURIComponent(name)}`;
  return p;
}
