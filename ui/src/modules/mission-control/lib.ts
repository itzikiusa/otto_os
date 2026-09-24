// Shared label / colour / formatting helpers for the Mission Control module.

import type { IconName } from '../../lib/components/Icon.svelte';
import type { WorkKind, WorkStatus, RiskLevel, WorkActor, ArtifactKind } from '../../lib/api/types';
import { runStatus, type StatusInfo, type Tone } from '../../lib/status';

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

export const KIND_ICON: Record<WorkKind, IconName> = {
  session: 'terminal',
  swarm: 'grid',
  goal_loop: 'refresh',
  workflow: 'split',
  review: 'eye',
  product_story: 'note',
  pr: 'pr',
  external_trigger: 'bell',
};

/** A work status in the shared run vocabulary (lib/status.ts): pending →
 *  Queued, done → Succeeded, running is info (never the succeeded green).
 *  `blocked` is Mission Control's own: stuck on a dependency/policy — danger. */
export function workStatus(s: WorkStatus): StatusInfo {
  if (s === 'blocked') return { key: 'blocked', label: 'Blocked', tone: 'danger' };
  return runStatus(s);
}

export const STATUS_LABEL: Record<WorkStatus, string> = {
  pending: workStatus('pending').label,
  running: workStatus('running').label,
  waiting: workStatus('waiting').label,
  blocked: workStatus('blocked').label,
  succeeded: workStatus('succeeded').label,
  failed: workStatus('failed').label,
  cancelled: workStatus('cancelled').label,
  done: workStatus('done').label,
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

const TONE_COLOR: Record<Tone, string> = {
  neutral: 'var(--text-dim)',
  info: 'var(--info)',
  success: 'var(--success)',
  warning: 'var(--warning)',
  danger: 'var(--danger)',
};

/** A colour (CSS value) for a normalized status — drives chips, labels and
 *  graph nodes. Tone tokens (text-safe in every theme), never raw hex; the
 *  tone comes from the shared vocabulary via {@link workStatus}. */
export function statusColor(s: WorkStatus): string {
  return TONE_COLOR[workStatus(s).tone];
}

/** A colour for a risk level (the "policy" axis). */
export function riskColor(r: RiskLevel): string {
  switch (r) {
    case 'critical':
      return 'var(--danger)';
    case 'high':
      return 'var(--warning)';
    // medium is the everyday default — neutral, so the chip isn't an amber
    // alarm on every row.
    default:
      return 'var(--text-dim)';
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
