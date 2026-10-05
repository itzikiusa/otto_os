// Shared helpers for the Kubernetes console: the kinds rail, formatting of
// ages / bytes / millicores, and health → Badge tone (the environment pill is the
// shared <EnvBadge>).

import { now } from '../../lib/stores/now.svelte';
import type { BadgeTone } from '../../lib/status';
import type {
  K8sCapabilities,
  K8sCluster,
  K8sContainer,
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

/** A row's age in seconds, ticking: `now - created_at` against the shared
 *  clock (reactive — a 304 keeps the same rows, so the frozen `age_seconds`
 *  would stop the column), else the server's `age_seconds`. */
export function rowAge(r: { age_seconds: number; created_at?: number | null }): number {
  if (r.created_at == null) return r.age_seconds;
  return Math.max(0, Math.floor(now() / 1000) - r.created_at);
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

/** Health → the shared Badge tone (status cells and the drawer header);
 *  `info` is the in-progress state, which the Badge pulses (`live`). */
export function healthTone(h: K8sHealth | null | undefined, status?: string): BadgeTone {
  const c = healthClass(h, status);
  return c === 'health-ok' ? 'ok' : c === 'health-bad' ? 'bad' : c === 'health-warn' ? 'warn' : c === 'health-progressing' ? 'info' : 'neutral';
}

export function clusterLabel(c: K8sCluster | null): string {
  return c ? c.name || c.context_name : '';
}

/** Filename-safe stem for log downloads. */
export function safeName(s: string): string {
  return s.replace(/[^A-Za-z0-9._-]+/g, '_');
}

/** kubectl stderr is a wall of repeated klog lines (`E0925 10:41:59 … memcache.go:265] …`)
 *  ending in one human sentence. Return that sentence for the headline; the
 *  caller keeps the raw text behind a disclosure. */
export function kubectlErrorSummary(raw: string | null | undefined): string {
  const text = (raw ?? '').replace(/^(invalid|forbidden|internal):\s*/i, '').replace(/^kubectl:\s*/i, '').trim();
  if (!text) return 'kubectl failed without a message.';
  const lines = text
    .split(/\n|(?=\b[EWI]\d{4} \d{2}:\d{2}:\d{2}\.\d+ )/)
    .map((l) => l.trim())
    .filter(Boolean);
  const human = lines.filter((l) => !/^[EWI]\d{4} \d{2}:\d{2}:\d{2}/.test(l));
  const pick = (human.length ? human[human.length - 1] : lines[lines.length - 1]) ?? text;
  return pick.length > 240 ? `${pick.slice(0, 237)}…` : pick;
}

/** A pod manifest's containers (init first) with their live status — the
 *  client-side twin of the daemon's `pod_containers`, so the resource drawer
 *  derives them from the `/resource` manifest it already has instead of a
 *  second `kubectl get pod` per open. */
export function podContainers(pod: unknown): K8sContainer[] {
  type Ctr = { name?: string; image?: string };
  type St = { name?: string; ready?: boolean; restartCount?: number; state?: Record<string, { reason?: string }> };
  const p = (pod ?? {}) as {
    spec?: { containers?: Ctr[]; initContainers?: Ctr[] };
    status?: { containerStatuses?: St[]; initContainerStatuses?: St[] };
  };
  const out: K8sContainer[] = [];
  const add = (specs: Ctr[] | undefined, statuses: St[] | undefined, init: boolean): void => {
    for (const c of specs ?? []) {
      const name = c.name ?? '';
      const st = (statuses ?? []).find((x) => x.name === name);
      const first = st?.state ? Object.entries(st.state)[0] : undefined;
      out.push({
        name,
        image: c.image ?? '',
        ready: st?.ready ?? false,
        state: first ? (first[1]?.reason ? `${first[0]}:${first[1].reason}` : first[0]) : 'unknown',
        restarts: st?.restartCount ?? 0,
        init,
      });
    }
  };
  add(p.spec?.initContainers, p.status?.initContainerStatuses, true);
  add(p.spec?.containers, p.status?.containerStatuses, false);
  return out;
}

/** Longest scalar the manifest view renders (SC-19). A ConfigMap can hold a
 *  0.5–1 MB single-line value (Grafana dashboards); CodeMirror would lay it
 *  out as one huge highlighted line. The Copy action still copies it whole. */
export const MANIFEST_SCALAR_MAX = 64 * 1024;

/** A copy of `value` with every string longer than `max` cut to `max` chars
 *  plus a marker, and how many were cut. Unchanged subtrees keep identity. */
export function clipLongScalars(value: unknown, max = MANIFEST_SCALAR_MAX): { value: unknown; clipped: number } {
  let clipped = 0;
  const walk = (v: unknown): unknown => {
    if (typeof v === 'string') {
      if (v.length <= max) return v;
      clipped++;
      return `${v.slice(0, max)}… [${Math.round((v.length - max) / 1024)} KiB more — use Copy for the full value]`;
    }
    if (Array.isArray(v)) {
      let out: unknown[] | null = null;
      for (let i = 0; i < v.length; i++) {
        const next = walk(v[i]);
        if (next !== v[i]) (out ??= v.slice())[i] = next;
      }
      return out ?? v;
    }
    if (v && typeof v === 'object') {
      let out: Record<string, unknown> | null = null;
      for (const [k, x] of Object.entries(v as Record<string, unknown>)) {
        const next = walk(x);
        if (next !== x) (out ??= { ...(v as Record<string, unknown>) })[k] = next;
      }
      return out ?? v;
    }
    return v;
  };
  const out = walk(value);
  return { value: out, clipped };
}
