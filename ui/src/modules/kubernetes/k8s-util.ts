// Shared helpers for the Kubernetes console: the kinds rail, formatting of
// ages / bytes / millicores, health → CSS class, and the environment pill.

import type {
  Environment,
  K8sCapabilities,
  K8sCluster,
  K8sHealth,
  K8sResourceKind,
} from '../../lib/api/types';

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

/** Kinds visible for a cluster given its capability probe (unknown ⇒ hide
 *  the CRD-backed ones until the probe lands). */
export function visibleKinds(caps: K8sCapabilities | null): KindDef[] {
  return KINDS.filter((k) => !k.requires || (caps?.[k.requires] ?? false));
}

/** `kubectl`-style compact age: 45s · 12m · 3h · 5d · 2y. */
export function formatAge(seconds: number | null | undefined): string {
  if (seconds == null || !Number.isFinite(seconds)) return '';
  const s = Math.max(0, Math.floor(seconds));
  if (s < 60) return `${s}s`;
  const m = Math.floor(s / 60);
  if (m < 60) return `${m}m`;
  const h = Math.floor(m / 60);
  if (h < 48) return `${h}h`;
  const d = Math.floor(h / 24);
  if (d < 365) return `${d}d`;
  return `${Math.floor(d / 365)}y`;
}

export function formatBytes(n: number | null | undefined): string {
  if (n == null || !Number.isFinite(n)) return '';
  if (n < 1024) return `${n} B`;
  const units = ['Ki', 'Mi', 'Gi', 'Ti'];
  let v = n / 1024;
  let i = 0;
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024;
    i++;
  }
  return `${v >= 100 ? v.toFixed(0) : v.toFixed(1)}${units[i]}`;
}

export function formatMillicores(m: number | null | undefined): string {
  if (m == null || !Number.isFinite(m)) return '';
  return m >= 1000 ? `${(m / 1000).toFixed(2)} cores` : `${Math.round(m)}m`;
}

/** Parse a kubectl quantity ("250m", "1.5", "128Mi", "2Gi") into millicores
 *  or bytes — used for the metrics bars when the daemon returns raw strings. */
export function parseCpu(q: string | null | undefined): number | null {
  if (!q) return null;
  const m = /^([\d.]+)(m|n|u)?$/.exec(q.trim());
  if (!m) return null;
  const v = parseFloat(m[1]);
  if (m[2] === 'm') return v;
  if (m[2] === 'u') return v / 1000;
  if (m[2] === 'n') return v / 1_000_000;
  return v * 1000;
}
export function parseMem(q: string | null | undefined): number | null {
  if (!q) return null;
  const m = /^([\d.]+)(Ki|Mi|Gi|Ti|K|M|G|T)?$/i.exec(q.trim());
  if (!m) return null;
  const v = parseFloat(m[1]);
  const mult: Record<string, number> = {
    '': 1,
    ki: 1024,
    mi: 1024 ** 2,
    gi: 1024 ** 3,
    ti: 1024 ** 4,
    k: 1e3,
    m: 1e6,
    g: 1e9,
    t: 1e12,
  };
  return v * (mult[(m[2] ?? '').toLowerCase()] ?? 1);
}

export function healthClass(h: K8sHealth | null | undefined, status?: string): string {
  if (h) return `health-${h}`;
  const s = (status ?? '').toLowerCase();
  if (/running|succeeded|ready|active|bound|healthy|synced|complete/.test(s)) return 'health-ok';
  if (/crash|error|fail|backoff|evicted|degraded|notready|unknown/.test(s)) return 'health-bad';
  if (/pending|creating|init|progress|terminating|waiting/.test(s)) return 'health-progressing';
  return '';
}

/** Short environment tag for the card/top-bar pill (mirrors the DB hub). */
export function envBadge(env: Environment | undefined): string {
  if (env === 'prod') return 'PROD';
  if (env === 'staging') return 'STG';
  return 'DEV';
}

export function clusterLabel(c: K8sCluster | null): string {
  return c ? c.name || c.context_name : '';
}

/** Filename-safe stem for log downloads. */
export function safeName(s: string): string {
  return s.replace(/[^A-Za-z0-9._-]+/g, '_');
}
