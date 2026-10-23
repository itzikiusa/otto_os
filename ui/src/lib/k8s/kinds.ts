// Kubernetes kinds, sizes and cluster labels shared by the console
// (modules/kubernetes), the k8s store and the ⌘K/agent commands. Lives in
// lib/ so stores and commands never import a module folder (arch-1-13);
// modules/kubernetes/k8s-util re-exports every name.

import { formatBytes as sharedFormatBytes } from '../metric-format';
import type { K8sCapabilities, K8sCluster, K8sResourceKind } from '../api/types';

export interface KindDef {
  id: K8sResourceKind;
  label: string;
  /** Singular label for the drawer header / confirms. */
  singular: string;
  /** Cluster-scoped kinds ignore the namespace filter. */
  clusterScoped?: boolean;
  /** Only shown when the capability probe says the CRD exists. */
  requires?: keyof Pick<K8sCapabilities, 'argo_rollouts' | 'argocd'>;
}

export const KINDS: KindDef[] = [
  { id: 'pods', label: 'Pods', singular: 'Pod' },
  { id: 'deployments', label: 'Deployments', singular: 'Deployment' },
  { id: 'statefulsets', label: 'StatefulSets', singular: 'StatefulSet' },
  { id: 'daemonsets', label: 'DaemonSets', singular: 'DaemonSet' },
  { id: 'replicasets', label: 'ReplicaSets', singular: 'ReplicaSet' },
  { id: 'jobs', label: 'Jobs', singular: 'Job' },
  { id: 'cronjobs', label: 'CronJobs', singular: 'CronJob' },
  { id: 'services', label: 'Services', singular: 'Service' },
  { id: 'ingresses', label: 'Ingresses', singular: 'Ingress' },
  { id: 'configmaps', label: 'ConfigMaps', singular: 'ConfigMap' },
  { id: 'secrets', label: 'Secrets', singular: 'Secret' },
  { id: 'pvcs', label: 'PVCs', singular: 'PersistentVolumeClaim' },
  { id: 'hpas', label: 'HPAs', singular: 'HorizontalPodAutoscaler' },
  { id: 'nodes', label: 'Nodes', singular: 'Node', clusterScoped: true },
  { id: 'events', label: 'Events', singular: 'Event' },
  { id: 'rollouts', label: 'Argo Rollouts', singular: 'Rollout', requires: 'argo_rollouts' },
  { id: 'applications', label: 'ArgoCD Apps', singular: 'Application', requires: 'argocd' },
];

export function kindDef(id: string): KindDef {
  return KINDS.find((k) => k.id === id) ?? { id: 'pods', label: id, singular: id };
}

export function isKind(id: string | undefined): id is K8sResourceKind {
  return !!id && KINDS.some((k) => k.id === id);
}

/** kubectl-style sizes ("128Mi"), blank when unknown — the shared
 *  formatter's IEC style (lib/metric-format), not a local copy. */
export function formatBytes(n: number | null | undefined): string {
  return sharedFormatBytes(n, { iec: true, empty: '' });
}

export function formatMillicores(m: number | null | undefined): string {
  if (m == null || !Number.isFinite(m)) return '';
  return m >= 1000 ? `${(m / 1000).toFixed(2)} cores` : `${Math.round(m)}m`;
}

export function clusterLabel(c: K8sCluster | null): string {
  return c ? c.name || c.context_name : '';
}
