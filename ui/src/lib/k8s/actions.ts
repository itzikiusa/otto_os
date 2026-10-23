// Per-kind action registry (contract §4.6) and the typed-name confirm, shared
// by the console (modules/kubernetes/actions re-exports them and posts the
// actions) and the ⌘K/agent commands (lib/uiCommands/k8s), so a lib/ file
// never imports a module folder (arch-1-13).

import type { IconName } from '../components/Icon.svelte';
import type { K8sAction, K8sResourceKind, K8sRow } from '../api/types';
import { confirmer } from '../confirm.svelte';
import { toasts } from '../toast.svelte';

export interface ActionDef {
  id: K8sAction;
  label: string;
  icon?: IconName;
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
  { id: 'rollout_undo', label: 'Rollout undo…', icon: 'arrowUp', danger: true, confirm: true },
];

export const ACTIONS: Partial<Record<K8sResourceKind, ActionDef[]>> = {
  pods: [{ id: 'delete_pod', label: 'Delete pod…', icon: 'trash', danger: true, confirm: true }],
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
    { id: 'rollout_abort', label: 'Abort…', icon: 'x', danger: true },
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
    { id: 'argocd_terminate_op', label: 'Terminate operation…', icon: 'x', danger: true },
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
      danger: true,
    },
  );
  if (typed === null) return false;
  if (typed !== name) {
    // Same wording as every other typed gate (confirmProd, the database gate).
    toasts.info('Nothing was changed', 'What you typed didn’t match, so the action was canceled.');
    return false;
  }
  return true;
}
