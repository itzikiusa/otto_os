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
