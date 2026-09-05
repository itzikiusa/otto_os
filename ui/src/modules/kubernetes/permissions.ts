import type { K8sResourceKind } from '../../lib/api/types';

export function readOperation(kind: K8sResourceKind): string {
  if (kind === 'secrets') return 'secrets_view';
  return ['pods', 'deployments', 'statefulsets', 'daemonsets', 'replicasets', 'jobs', 'cronjobs', 'rollouts', 'applications'].includes(kind) ? 'workloads_view' : 'resources_view';
}

export function actionOperation(action: string): string {
  if (action === 'restart' || action === 'argocd_app_restart') return 'restart';
  if (action === 'scale') return 'scale';
  if (action === 'delete_pod') return 'delete';
  if (action === 'rollout_status') return 'workloads_view';
  return 'apply';
}
