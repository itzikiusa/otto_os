// Per-kind column layout for the resource table. Fixed columns come from the
// normalized `K8sRow` fields; kind-specific ones read `row.extra[...]`. When
// the daemon returns extra keys this file doesn't know (a newer backend), they
// are appended generically so nothing is silently dropped.

import type { K8sResourceKind, K8sRow } from '../../lib/api/types';
import { formatAge, formatBytes, formatMillicores } from './k8s-util';

export interface Column {
  key: string;
  label: string;
  /** CSS grid track. */
  width: string;
  /** Right-align numeric-ish columns. */
  num?: boolean;
  mono?: boolean;
  /** Render the health-colored status pill. */
  status?: boolean;
  value: (r: K8sRow) => string;
}

const extra = (k: string) => (r: K8sRow) => r.extra?.[k] ?? '';

const NAME: Column = { key: 'name', label: 'Name', width: 'minmax(160px, 2fr)', value: (r) => r.name };
const NS: Column = { key: 'namespace', label: 'Namespace', width: 'minmax(96px, 0.8fr)', value: (r) => r.namespace };
const STATUS: Column = { key: 'status', label: 'Status', width: 'minmax(104px, 0.9fr)', status: true, value: (r) => r.status };
const READY: Column = { key: 'ready', label: 'Ready', width: '58px', num: true, mono: true, value: (r) => r.ready ?? '' };
const RESTARTS: Column = { key: 'restarts', label: 'Restarts', width: '66px', num: true, mono: true, value: (r) => (r.restarts == null ? '' : String(r.restarts)) };
const CPU: Column = { key: 'cpu', label: 'CPU', width: '62px', num: true, mono: true, value: (r) => (r.cpu == null ? '' : formatMillicores(r.cpu)) };
const MEM: Column = { key: 'mem', label: 'MEM', width: '70px', num: true, mono: true, value: (r) => (r.mem == null ? '' : formatBytes(r.mem)) };
const NODE: Column = { key: 'node', label: 'Node', width: 'minmax(96px, 1fr)', value: (r) => r.node ?? '' };
const IP: Column = { key: 'ip', label: 'IP', width: '108px', mono: true, value: (r) => r.ip ?? '' };
const AGE: Column = { key: 'age', label: 'Age', width: '52px', num: true, mono: true, value: (r) => formatAge(r.age_seconds) };

const ex = (key: string, label: string, width = 'minmax(90px, 1fr)', opts: Partial<Column> = {}): Column => ({
  key: `extra.${key}`,
  label,
  width,
  value: extra(key),
  ...opts,
});

/** Kind-specific columns in display order (only rendered when at least one
 *  row carries the key, so an older daemon shows a narrower table rather
 *  than empty columns). */
const KIND_EXTRA: Partial<Record<K8sResourceKind, Column[]>> = {
  deployments: [ex('desired', 'Desired', '70px', { num: true, mono: true }), ex('updated', 'Up-to-date', '84px', { num: true, mono: true }), ex('available', 'Available', '80px', { num: true, mono: true })],
  statefulsets: [ex('desired', 'Desired', '70px', { num: true, mono: true }), ex('updated', 'Updated', '76px', { num: true, mono: true }), ex('available', 'Available', '80px', { num: true, mono: true })],
  daemonsets: [ex('desired', 'Desired', '70px', { num: true, mono: true }), ex('updated', 'Updated', '76px', { num: true, mono: true }), ex('available', 'Available', '80px', { num: true, mono: true })],
  replicasets: [ex('desired', 'Desired', '70px', { num: true, mono: true }), ex('current', 'Current', '70px', { num: true, mono: true })],
  jobs: [ex('completions', 'Completions', '100px', { mono: true }), ex('active', 'Active', '60px', { num: true, mono: true }), ex('duration_seconds', 'Duration', '80px', { num: true, mono: true, value: (r) => (r.extra?.duration_seconds ? formatAge(Number(r.extra.duration_seconds)) : '') })],
  cronjobs: [ex('schedule', 'Schedule', '120px', { mono: true }), ex('suspend', 'Suspend', '70px'), ex('active', 'Active', '64px', { num: true, mono: true }), ex('last_schedule', 'Last schedule', '110px', { mono: true })],
  services: [ex('type', 'Type', '100px'), ex('cluster_ip', 'Cluster IP', '120px', { mono: true }), ex('external_ip', 'External IP', '140px', { mono: true }), ex('ports', 'Ports', 'minmax(120px, 1.2fr)', { mono: true })],
  ingresses: [ex('class', 'Class', '90px'), ex('hosts', 'Hosts', 'minmax(160px, 2fr)', { mono: true }), ex('address', 'Address', 'minmax(120px, 1fr)', { mono: true }), ex('tls', 'TLS', '56px', { value: (r) => (r.extra?.tls === 'true' ? 'yes' : '') })],
  configmaps: [ex('keys', 'Key names', 'minmax(160px, 2fr)', { mono: true })],
  secrets: [ex('key_count', 'Keys', '60px', { num: true, mono: true }), ex('keys', 'Key names', 'minmax(160px, 2fr)', { mono: true })],
  pvcs: [ex('volume', 'Volume', 'minmax(140px, 1.4fr)', { mono: true }), ex('capacity', 'Capacity', '80px', { num: true, mono: true }), ex('access_modes', 'Access', '80px'), ex('storage_class', 'StorageClass', '110px')],
  hpas: [ex('reference', 'Reference', 'minmax(160px, 1.6fr)', { mono: true }), ex('targets', 'Targets', 'minmax(120px, 1fr)', { mono: true }), ex('min', 'Min', '50px', { num: true, mono: true }), ex('max', 'Max', '50px', { num: true, mono: true }), ex('replicas', 'Replicas', '76px', { num: true, mono: true })],
  rollouts: [ex('strategy', 'Strategy', '90px'), ex('phase', 'Phase', '100px'), ex('step', 'Step', '64px', { mono: true }), ex('weight', 'Weight', '64px', { num: true, mono: true }), ex('paused', 'Paused', '64px'), ex('desired', 'Desired', '70px', { num: true, mono: true }), ex('available', 'Available', '80px', { num: true, mono: true })],
  applications: [ex('sync', 'Sync', '100px'), ex('health', 'Health', '100px'), ex('revision', 'Revision', '90px', { mono: true }), ex('repo', 'Repo', 'minmax(160px, 2fr)', { mono: true }), ex('path', 'Path', 'minmax(100px, 1fr)', { mono: true }), ex('dest_ns', 'Dest NS', '110px'), ex('operation', 'Operation', '90px')],
  events: [ex('reason', 'Reason', '130px'), ex('object', 'Object', 'minmax(160px, 1.4fr)', { mono: true }), ex('message', 'Message', 'minmax(240px, 3fr)'), ex('count', 'Count', '60px', { num: true, mono: true }), ex('source', 'Source', '120px', { mono: true })],
  nodes: [ex('roles', 'Roles', '120px'), ex('version', 'Version', '110px', { mono: true }), ex('cpu_capacity', 'CPU cap', '80px', { num: true, mono: true }), ex('mem_capacity', 'MEM cap', '90px', { num: true, mono: true })],
};

const STATUS_LABEL: Partial<Record<K8sResourceKind, string>> = {
  events: 'Type',
  secrets: 'Type',
  configmaps: 'Keys',
};

/** Extras that are already shown by a fixed column (status / ready / the
 *  drawer) — never auto-appended as a generic column. */
const HIDDEN_EXTRA: Partial<Record<K8sResourceKind, string[]>> = {
  pods: ['phase', 'message', 'containers', 'qos'],
  deployments: ['ready', 'reason', 'paused', 'selector'],
  statefulsets: ['ready', 'reason', 'selector'],
  daemonsets: ['ready', 'reason', 'selector'],
  replicasets: ['ready', 'reason', 'selector'],
  rollouts: ['ready', 'message', 'selector'],
  secrets: ['type'],
  configmaps: ['key_count'],
  services: ['selector', 'type'],
  events: ['last_seen', 'message'],
  jobs: ['failed', 'selector'],
};

export function columnsFor(
  kind: K8sResourceKind,
  rows: K8sRow[],
  hasMetrics: boolean,
  allNamespaces: boolean,
): Column[] {
  const cols: Column[] = [NAME];
  const clusterScoped = kind === 'nodes';
  if (allNamespaces && !clusterScoped) cols.push(NS);
  if (kind === 'pods') {
    cols.push(READY, STATUS, RESTARTS);
    if (hasMetrics) cols.push(CPU, MEM);
    cols.push(NODE, IP);
  } else if (kind === 'nodes') {
    cols.push(STATUS);
    if (hasMetrics) cols.push(CPU, MEM);
  } else if (kind === 'events') {
    // An event's `status` is its type (Normal / Warning); Age is "last seen".
    cols.push({ ...STATUS, label: STATUS_LABEL.events ?? 'Type', width: '100px' });
  } else if (kind === 'secrets' || kind === 'configmaps') {
    // Status carries the secret type / "n keys" — a plain column, no health dot.
    cols.push({ ...STATUS, status: false, label: STATUS_LABEL[kind] ?? 'Status', mono: true, width: 'minmax(140px, 1.2fr)' });
  } else if (['deployments', 'statefulsets', 'daemonsets', 'replicasets', 'rollouts'].includes(kind)) {
    cols.push(READY, STATUS);
  } else {
    cols.push(STATUS);
  }
  // Kind-specific extras that at least one row actually carries.
  const present = new Set<string>();
  const sample = rows.length > 200 ? rows.slice(0, 200) : rows;
  for (const r of sample) for (const k of Object.keys(r.extra ?? {})) present.add(k);
  const known = (KIND_EXTRA[kind] ?? []).filter((c) => present.has(c.key.slice(6)));
  cols.push(...known);
  // Unknown extras (newer daemon) — append generically so nothing is lost.
  const knownKeys = new Set((KIND_EXTRA[kind] ?? []).map((c) => c.key.slice(6)));
  const hidden = new Set(HIDDEN_EXTRA[kind] ?? []);
  for (const k of present) {
    if (knownKeys.has(k) || hidden.has(k)) continue;
    if (kind === 'nodes') continue; // folded from K8sNode; nothing unknown there
    cols.push(ex(k, k.replace(/_/g, ' '), 'minmax(90px, 1fr)'));
  }
  cols.push(kind === 'events' ? { ...AGE, label: 'Last seen', width: '80px' } : AGE);
  return cols;
}

export function gridTemplate(cols: Column[]): string {
  return cols.map((c) => c.width).join(' ');
}
