// Per-kind action registry (contract §4.6) → `POST /k8s/clusters/{id}/actions`.
// Every entry is Edit-gated by the caller (`auth.can('kubernetes','edit')`);
// destructive ones go through `confirmer` with a typed-name confirm and send
// `params.confirm_name` (the daemon refuses them without it). Dialog-backed
// actions (scale / ArgoCD sync) are surfaced via `needs` so the workspace can
// open the matching sheet instead of posting straight away.

import { k8sApi } from '../../lib/api/k8s';
import type { K8sAction, K8sActionResp, K8sResourceKind, K8sRow } from '../../lib/api/types';
import { confirmer } from '../../lib/confirm.svelte';
import { toasts } from '../../lib/toast.svelte';
import { kindDef } from './k8s-util';

export interface ActionDef {
  id: K8sAction;
  label: string;
  icon?: string;
  danger?: boolean;
  /** Extra request params (fixed per menu entry, e.g. promote full / hard refresh). */
  params?: Record<string, unknown>;
  /** A dialog collects the params (scale replicas / sync revision+prune). */
  needs?: 'scale' | 'sync';
  /** Typed-name confirm before posting. */
  confirm?: boolean;
  /** Only offered when the row's `extra[key]` matches (e.g. paused rollouts). */
  when?: (r: K8sRow) => boolean;
}

/** The `restart` action isn't in this list for the drawer — see `restartLabel`. */
const WORKLOAD: ActionDef[] = [
  { id: 'restart', label: 'Restart (rollout restart)', icon: 'refresh' },
  { id: 'scale', label: 'Scale…', icon: 'layers', needs: 'scale' },
  { id: 'rollout_status', label: 'Rollout status', icon: 'info' },
  { id: 'rollout_undo', label: 'Rollout undo', icon: 'arrowUp', danger: true, confirm: true },
];

export const ACTIONS: Partial<Record<K8sResourceKind, ActionDef[]>> = {
  pods: [{ id: 'delete_pod', label: 'Delete pod', icon: 'trash', danger: true, confirm: true }],
  deployments: [
    ...WORKLOAD,
    { id: 'rollout_pause', label: 'Pause rollout', icon: 'square' },
    { id: 'rollout_resume', label: 'Resume rollout', icon: 'play' },
  ],
  statefulsets: WORKLOAD,
  daemonsets: WORKLOAD.filter((a) => a.id !== 'scale'),
  rollouts: [
    { id: 'restart', label: 'Restart', icon: 'refresh' },
    { id: 'rollout_promote', label: 'Promote', icon: 'arrowUp' },
    { id: 'rollout_promote', label: 'Promote (full)', icon: 'zap', params: { full: true } },
    { id: 'rollout_abort', label: 'Abort', icon: 'x', danger: true },
    { id: 'rollout_retry', label: 'Retry', icon: 'refresh' },
    { id: 'rollout_pause', label: 'Pause', icon: 'square', when: (r) => r.extra?.paused !== 'true' },
    { id: 'rollout_resume', label: 'Resume', icon: 'play', when: (r) => r.extra?.paused === 'true' },
    { id: 'scale', label: 'Scale…', icon: 'layers', needs: 'scale' },
  ],
  applications: [
    { id: 'argocd_sync', label: 'Sync…', icon: 'refresh', needs: 'sync' },
    { id: 'argocd_refresh', label: 'Refresh', icon: 'refresh' },
    { id: 'argocd_refresh', label: 'Hard refresh', icon: 'zap', params: { hard: true } },
    { id: 'argocd_app_restart', label: 'Restart workloads (redeploy)', icon: 'play' },
    { id: 'argocd_terminate_op', label: 'Terminate operation', icon: 'x', danger: true },
  ],
  cronjobs: [
    { id: 'cronjob_trigger', label: 'Trigger now', icon: 'play' },
    { id: 'cronjob_suspend', label: 'Suspend', icon: 'square', when: (r) => r.extra?.suspend !== 'true' },
    { id: 'cronjob_resume', label: 'Resume', icon: 'play', when: (r) => r.extra?.suspend === 'true' },
  ],
};

export function actionsFor(kind: K8sResourceKind, row: K8sRow): ActionDef[] {
  return (ACTIONS[kind] ?? []).filter((a) => !a.when || a.when(row));
}

/** Ask for the resource name to be typed back before a destructive action. */
export async function typedConfirm(
  what: string,
  name: string,
  opts?: { title?: string; confirmLabel?: string },
): Promise<boolean> {
  const typed = await confirmer.promptText(
    `${what}\n\nType the resource name (${name}) to confirm.`,
    {
      title: opts?.title ?? 'Confirm',
      confirmLabel: opts?.confirmLabel ?? 'Confirm',
      placeholder: name,
    },
  );
  if (typed === null) return false;
  if (typed !== name) {
    toasts.warn('Name did not match', 'Nothing was changed.');
    return false;
  }
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
  if (destructive) {
    const ok = await typedConfirm(
      `${def.label.replace(/…$/, '')} ${singular} “${row.name}”${row.namespace ? ` in ${row.namespace}` : ''}?`,
      row.name,
      { title: def.label.replace(/…$/, ''), confirmLabel: def.label.replace(/…$/, '') },
    );
    if (!ok) return null;
    merged.confirm_name = row.name;
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
    toasts.error(`${def.label.replace(/…$/, '')} failed`, e instanceof Error ? e.message : String(e));
    return null;
  }
}
