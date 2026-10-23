// Per-kind action registry (contract §4.6) → `POST /k8s/clusters/{id}/actions`.
// Every entry is Edit-gated by the caller (`auth.can('kubernetes','edit')`);
// destructive ones go through `confirmProd` with a typed-name confirm and send
// `params.confirm_name` (the daemon refuses them without it). Dialog-backed
// actions (scale / ArgoCD sync) are surfaced via `needs` so the workspace can
// open the matching sheet instead of posting straight away.

import { k8sApi } from '../../lib/api/k8s';
import type { K8sActionResp, K8sResourceKind, K8sRow } from '../../lib/api/types';
import { confirmProd, isProdEnv } from '../../lib/confirmProd';
import { toasts } from '../../lib/toast.svelte';
import { k8s } from '../../lib/stores/k8s.svelte';
import { clusterLabel, kindDef } from './k8s-util';
import { toastError } from '../../lib/toastError';
import type { ActionDef } from '../../lib/k8s/actions';

export { ACTIONS, actionsFor, typedConfirm, type ActionDef } from '../../lib/k8s/actions';

/** Anything that changes cluster state — everything but the read-only status
 *  check and a plain (soft) ArgoCD refresh. */
function isMutating(def: ActionDef, params: Record<string, unknown>): boolean {
  if (def.id === 'rollout_status') return false;
  if (def.id === 'argocd_refresh') return params.hard === true;
  return true;
}

/** Post an action (after its typed confirm when required) and toast the
 *  outcome. Resolves the response, or `null` when cancelled / failed. */
export async function runAction(
  clusterId: string,
  kind: K8sResourceKind,
  row: K8sRow,
  def: ActionDef,
  params: Record<string, unknown> = {},
): Promise<K8sActionResp | null> {
  const singular = kindDef(kind).singular;
  const merged = { ...(def.params ?? {}), ...params };
  const destructive =
    def.confirm ||
    (def.id === 'scale' && Number(merged.replicas) === 0) ||
    (def.id === 'argocd_sync' && merged.prune === true);
  // Name the cluster (and flag prod) in every confirm, so a delete fired from
  // the wrong cluster tab reads as such before it's sent. confirmProd adds the
  // one shared PRODUCTION line and the typed-name gate.
  const cl = k8s.clusters.find((c) => c.id === clusterId) ?? null;
  const prod = isProdEnv(cl?.environment);
  const where = `${cl ? clusterLabel(cl) : 'this cluster'}${row.namespace ? ` · namespace ${row.namespace}` : ''}`;
  const verb = def.label.replace(/…$/, '');
  // "Delete pod Pod “x”" read twice — drop the kind when the label already has it,
  // and say what a scale actually does when it's the destructive 0.
  const noun = verb.toLowerCase().includes(singular.toLowerCase()) ? '' : ` ${singular}`;
  const what = def.id === 'scale' ? `Scale${noun} “${row.name}” to ${Number(merged.replicas)} replicas` : `${verb}${noun} “${row.name}”`;
  if (destructive) {
    const ok = await confirmProd({ env: cl?.environment, where, verb, what, typed: row.name, danger: true });
    if (!ok) return null;
    merged.confirm_name = row.name;
  } else if (def.danger || (prod && isMutating(def, merged))) {
    // Danger-flagged entries without a typed confirm (rollout abort, ArgoCD
    // terminate operation) were one click from the red menu item — ask once.
    // On a PRODUCTION cluster every mutating action asks too (restart, promote,
    // pause/resume, cronjob trigger/suspend, ArgoCD restart/hard refresh/sync),
    // naming the action and where it lands.
    const ok = await confirmProd({ env: cl?.environment, where, verb, what, danger: !!def.danger });
    if (!ok) return null;
  }
  try {
    const resp = await k8sApi.action(clusterId, {
      action: def.id,
      kind,
      ns: row.namespace,
      name: row.name,
      params: Object.keys(merged).length ? merged : undefined,
    });
    if (resp.ok) toasts.success(`${def.label.replace(/…$/, '')} · ${row.name}`, resp.message || undefined);
    else toasts.warn(`${def.label.replace(/…$/, '')} · ${row.name}`, resp.message);
    return resp;
  } catch (e) {
    toastError(`Couldn’t ${verb.charAt(0).toLowerCase()}${verb.slice(1)} ${row.name}`, e);
    return null;
  }
}
