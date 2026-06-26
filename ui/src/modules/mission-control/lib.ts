// Shared label / colour / formatting helpers for the Mission Control module.

import type { WorkKind, WorkStatus, RiskLevel, WorkActor, ArtifactKind } from '../../lib/api/types';

export const KIND_LABEL: Record<WorkKind, string> = {
  session: 'Session',
  swarm: 'Swarm Project',
  goal_loop: 'Goal Loop',
  workflow: 'Workflow Run',
  review: 'PR Review',
  product_story: 'Product Story',
  pr: 'Pull Request',
  external_trigger: 'External Trigger',
};

export const KIND_ICON: Record<WorkKind, string> = {
  session: 'terminal',
  swarm: 'grid',
  goal_loop: 'refresh',
  workflow: 'split',
  review: 'eye',
  product_story: 'note',
  pr: 'pr',
  external_trigger: 'bell',
};

export const STATUS_LABEL: Record<WorkStatus, string> = {
  pending: 'Pending',
  running: 'Running',
  waiting: 'Waiting',
  blocked: 'Blocked',
  succeeded: 'Succeeded',
  failed: 'Failed',
  cancelled: 'Cancelled',
  done: 'Done',
};

export const RISK_LABEL: Record<RiskLevel, string> = {
  low: 'Low',
  medium: 'Medium',
  high: 'High',
  critical: 'Critical',
};

export const ACTOR_LABEL: Record<WorkActor, string> = {
  user: 'User',
  agent: 'Agent',
  system: 'System',
  integration: 'Integration',
};

export const ARTIFACT_LABEL: Record<ArtifactKind, string> = {
  diff: 'Diff',
  commit: 'Commit',
  pr: 'Pull Request',
  test_run: 'Test Run',
  report: 'Report',
  file: 'File',
  link: 'Link',
  finding: 'Finding',
  session: 'Session',
};

export const WORK_KINDS: WorkKind[] = [
  'session',
  'swarm',
  'goal_loop',
  'workflow',
  'review',
  'product_story',
  'pr',
  'external_trigger',
];
export const WORK_STATUSES: WorkStatus[] = [
  'pending',
  'running',
  'waiting',
  'blocked',
  'succeeded',
  'failed',
  'cancelled',
  'done',
];
export const RISK_LEVELS: RiskLevel[] = ['low', 'medium', 'high', 'critical'];

/** A colour (CSS value) for a normalized status — drives chips and graph nodes. */
export function statusColor(s: WorkStatus): string {
  switch (s) {
    case 'running':
      return 'var(--status-working, #28c840)';
    case 'succeeded':
    case 'done':
      return '#2ea043';
    case 'waiting':
    case 'pending':
      return 'var(--status-warn, #e0a000)';
    case 'blocked':
    case 'failed':
      return 'var(--status-exited, #ff5f57)';
    case 'cancelled':
      return 'var(--text-dim, #98989f)';
    default:
      return 'var(--text-dim, #98989f)';
  }
}

/** A colour for a risk level (the "policy" axis). */
export function riskColor(r: RiskLevel): string {
  switch (r) {
    case 'critical':
      return '#ff5f57';
    case 'high':
      return '#ff8c00';
    case 'medium':
      return 'var(--status-warn, #e0a000)';
    default:
      return 'var(--text-dim, #98989f)';
  }
}

export function fmtCost(n: number | null | undefined): string {
  const v = typeof n === 'number' ? n : 0;
  return `$${v.toFixed(2)}`;
}

/** Compact relative time ("3m", "2h", "5d") from an ISO timestamp. */
export function relTime(iso: string | null | undefined): string {
  if (!iso) return '—';
  const t = Date.parse(iso);
  if (Number.isNaN(t)) return '—';
  const secs = Math.max(0, Math.floor((Date.now() - t) / 1000));
  if (secs < 60) return `${secs}s`;
  const mins = Math.floor(secs / 60);
  if (mins < 60) return `${mins}m`;
  const hrs = Math.floor(mins / 60);
  if (hrs < 24) return `${hrs}h`;
  const days = Math.floor(hrs / 24);
  return `${days}d`;
}
