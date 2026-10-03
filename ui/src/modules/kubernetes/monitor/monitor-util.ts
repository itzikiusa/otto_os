// Shared helpers for the Kubernetes Monitor views: window options, health
// badge colouring, restart-class colours, number formatting, and the tiny
// HTML allowlist used when rendering a watchdog report.

import type { K8sMonitorHealth, K8sMonitorStatus, K8sRestartClass } from '../../../lib/api/types';

export const WINDOWS = ['1h', '6h', '24h', '7d'] as const;
export type Window = (typeof WINDOWS)[number];

export function isWindow(s: string | undefined): s is Window {
  return !!s && (WINDOWS as readonly string[]).includes(s);
}

/** Badge label + CSS modifier for an overview row's health. */
export function healthLabel(h: K8sMonitorHealth): { label: string; cls: string } {
  switch (h) {
    case 'healthy':
      return { label: 'Healthy', cls: 'ok' };
    case 'degraded':
      return { label: 'Degraded', cls: 'warn' };
    case 'incident':
      return { label: 'Incident', cls: 'bad' };
    case 'off':
      return { label: 'Monitoring off', cls: 'off' };
    default:
      return { label: 'No data yet', cls: 'off' };
  }
}

/** Colour token for a restart class (stacked bars, chips, timeline dots). */
export function classColor(c: K8sRestartClass | '' | string): string {
  switch (c) {
    case 'oom':
      return 'var(--status-exited)';
    case 'crash':
      return 'color-mix(in srgb, var(--status-exited) 60%, orange)';
    case 'probe':
      return 'orange';
    case 'planned':
      return 'var(--accent)';
    case 'completed':
      return 'var(--status-working)';
    default:
      return 'var(--text-dim)';
  }
}

export function classLabel(c: string): string {
  switch (c) {
    case 'oom':
      return 'OOM';
    case 'crash':
      return 'Crash';
    case 'probe':
      return 'Liveness';
    case 'planned':
      return 'Planned';
    case 'completed':
      return 'Completed';
    case 'unknown':
      return 'Unknown';
    case 'version':
      return 'New version';
    default:
      return c || '—';
  }
}

export function fmtPct(v: number | null | undefined, digits = 1): string {
  if (v === null || v === undefined || !Number.isFinite(v)) return '—';
  return `${v.toFixed(digits)}%`;
}

export function fmtRate(v: number | null | undefined): string {
  if (v === null || v === undefined || !Number.isFinite(v)) return '—';
  if (v >= 100) return `${Math.round(v)}/s`;
  if (v >= 10) return `${v.toFixed(1)}/s`;
  return `${v.toFixed(2)}/s`;
}

export function fmtMs(v: number | null | undefined): string {
  if (!v || !Number.isFinite(v)) return '—';
  if (v >= 1000) return `${(v / 1000).toFixed(2)}s`;
  return `${Math.round(v)}ms`;
}

export function fmtAgo(iso: string | null | undefined): string {
  if (!iso) return 'never';
  const t = Date.parse(iso);
  if (Number.isNaN(t)) return iso;
  const s = Math.max(0, Math.round((Date.now() - t) / 1000));
  if (s < 60) return `${s}s ago`;
  if (s < 3600) return `${Math.round(s / 60)}m ago`;
  if (s < 86400) return `${Math.round(s / 3600)}h ago`;
  return `${Math.round(s / 86400)}d ago`;
}

/** `forbidden: cluster RBAC: Error from server (Forbidden): …` → the kubectl line. */
export function rbacMessage(ms: string | undefined): string | null {
  if (!ms || !ms.startsWith('forbidden:')) return null;
  return ms.slice('forbidden:'.length).replace(/^\s*cluster RBAC:\s*/, '').trim();
}

/** One-line collector status for cards + headers. */
export function collectorLine(status: K8sMonitorStatus | null | undefined, enabled: boolean): string {
  if (!enabled) return 'Monitoring is off';
  if (!status) return 'Waiting for the first cycle…';
  if (!status.last_ok_at && status.last_error) return `Error: ${status.last_error}`;
  const parts = [`last cycle ${fmtAgo(status.last_cycle_at)}`];
  if (status.pods_seen) parts.push(`${status.pods_scraped}/${status.pods_seen} pods scraped`);
  if (status.cycle_ms) parts.push(`${(status.cycle_ms / 1000).toFixed(1)}s`);
  if (status.transport_used) parts.push(status.transport_used === 'proxy' ? 'via proxy' : 'via port-forward');
  if (status.last_error) parts.push(status.last_error);
  return parts.join(' · ');
}

/** Extract `Verdict: X` from a watchdog report (last occurrence wins). */
export function verdictOf(md: string): 'HEALTHY' | 'DEGRADED' | 'INCIDENT' | null {
  const m = [...md.matchAll(/Verdict:\s*\**\s*(HEALTHY|DEGRADED|INCIDENT)/gi)];
  if (!m.length) return null;
  return m[m.length - 1][1].toUpperCase() as 'HEALTHY' | 'DEGRADED' | 'INCIDENT';
}

/** Server page caps (`otto-k8s` monitor `fleet.rs`: table 2000, events 1000). */
export const FLEET_TABLE_MAX = 2000;
export const FLEET_EVENTS_MAX = 1000;

/** The `limit` for a fleet table/events load, or `null` to skip it (r3-02-04).
 *  A LIVE refresh (after every collection cycle) re-reads as many rows as the
 *  user has paged in — it used to reset "Load more" back to one page every few
 *  seconds — and is skipped while a "Load more" is in flight (it would abort
 *  it) or when the loaded rows are past the server cap (a head-only refresh
 *  would drop the rest). Every other load is one page. */
export function liveRefreshLimit(
  liveRefresh: boolean,
  loaded: number,
  page: number,
  serverMax: number,
  appendInFlight: boolean,
): number | null {
  if (!liveRefresh) return page;
  if (appendInFlight || loaded > serverMax) return null;
  return Math.max(page, loaded);
}

// ── perf K8s (review K6): stable identities across live refreshes ────────────

/** A fleet table row's identity: cluster / namespace / workload (/ pod in pod
 *  grouping — empty in workload grouping, so the key reads the same there). */
export function fleetRowKey(r: { cluster_id: string; namespace: string; workload: string; pod: string }): string {
  return r.pod ? `${r.cluster_id}/${r.namespace}/${r.workload}/${r.pod}` : `${r.cluster_id}/${r.namespace}/${r.workload}`;
}

/** A fleet event's identity — events carry no id, so the fields that tell two
 *  apart: when, where, what. Pair with {@link uniqueKeys} (two identical
 *  events in one second must still get distinct keys). */
export function fleetEventKey(e: { ts: string; cluster_id: string; namespace: string; pod: string; container?: string | null; reason: string; kind: string; class?: string }): string {
  return `${e.ts}|${e.cluster_id}|${e.namespace}|${e.pod}|${e.container ?? ''}|${e.kind}|${e.class ?? ''}|${e.reason}`;
}

/** `keys` with duplicates suffixed (`k`, `k#2`, `k#3`…) so a keyed `{#each}`
 *  never sees the same key twice; a unique key list comes back unchanged. */
export function uniqueKeys(keys: readonly string[]): string[] {
  const seen = new Map<string, number>();
  return keys.map((k) => {
    const n = (seen.get(k) ?? 0) + 1;
    seen.set(k, n);
    return n === 1 ? k : `${k}#${n}`;
  });
}

/** `next` with every item that is UNCHANGED since `prev` (same key, same JSON)
 *  swapped for the `prev` object, so a keyed `{#each}` skips re-rendering it.
 *  When nothing changed at all (same keys, same order, same content) `prev`
 *  itself comes back — assigning it to `$state.raw` is then a no-op. */
export function reuseByKey<T>(prev: readonly T[], next: readonly T[], key: (item: T) => string): T[] {
  if (!prev.length) return next as T[];
  const old = new Map<string, { item: T; json: string }>();
  for (const p of prev) old.set(key(p), { item: p, json: JSON.stringify(p) });
  let same = prev.length === next.length;
  const out = next.map((n, i) => {
    const hit = old.get(key(n));
    if (hit && hit.json === JSON.stringify(n)) {
      if (prev[i] !== hit.item) same = false;
      return hit.item;
    }
    same = false;
    return n;
  });
  return same ? (prev as T[]) : out;
}
