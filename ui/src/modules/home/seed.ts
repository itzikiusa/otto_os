// First-visit seed for Home (home.svelte.ts `ensureDefault`). A new user's
// first Home must not be a wall of empty widgets: Agents is always seeded (it
// also lists threads); Kubernetes only when a cluster is registered and a DB
// dashboard only when one exists — and then pointed at that dashboard.

import { api } from '../../lib/api/client';
import { k8sApi } from '../../lib/api/k8s';
import type { DbDashboard, Feature } from '../../lib/api/types';
import type { HomeBoxKind } from '../../lib/home/kinds';

export interface SeedData {
  /** At least one Kubernetes cluster is registered. */
  clusters: boolean;
  /** The first DB dashboard of the workspace, or null. */
  dashboardId: string | null;
}

export interface SeedBox {
  kind: HomeBoxKind;
  config: Record<string, unknown>;
}

/** The boxes to seed, given what the user can view and what has data. */
export function seedBoxes(can: (f: Feature) => boolean, data: SeedData): SeedBox[] {
  const out: SeedBox[] = [];
  if (can('agents')) out.push({ kind: 'sessions', config: {} });
  if (can('kubernetes') && data.clusters) out.push({ kind: 'k8s', config: {} });
  if (can('database') && data.dashboardId) out.push({ kind: 'db-dashboard', config: { dashboardId: data.dashboardId } });
  return out;
}

/** Ask the daemon whether clusters / dashboards exist. A failed read counts
 *  as "none" — the widget can still be added from Add widget. */
export async function probeSeedData(wsId: string | null, can: (f: Feature) => boolean): Promise<SeedData> {
  const [clusters, dashboardId] = await Promise.all([
    can('kubernetes')
      ? k8sApi.listClusters().then((l) => l.length > 0, () => false)
      : Promise.resolve(false),
    can('database') && wsId
      ? api.get<DbDashboard[]>(`/workspaces/${encodeURIComponent(wsId)}/db/dashboards`).then((l) => l[0]?.id ?? null, () => null)
      : Promise.resolve(null),
  ]);
  return { clusters, dashboardId };
}
