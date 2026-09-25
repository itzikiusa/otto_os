// Pure presentation helpers for a run's stage status — shared by the list rows
// and the detail panel. Kept rune-free (a plain .ts) so it's trivially testable
// and importable from any .svelte file.

import type { IconName } from '../../lib/components/Icon.svelte';
import type { RunStatus } from '../../lib/api/types';
import type { StatusInfo, Tone } from '../../lib/status';

/** Tone buckets that map onto the page's status-pill CSS classes. */
export type StatusTone = 'ok' | 'bad' | 'warn' | 'active' | 'dim';

const TERMINAL: ReadonlySet<string> = new Set([
  'completed',
  'failed',
  'rejected',
  'cancelled',
]);

/** True for statuses past which no further action runs. */
export function isTerminal(status: RunStatus | string): boolean {
  return TERMINAL.has(status);
}

/** Color bucket: green completed, red failed/rejected, amber awaiting_approval,
 *  blue for any active mid-stage, dim for queued/cancelled. */
export function statusTone(status: RunStatus | string): StatusTone {
  switch (status) {
    case 'completed':
      return 'ok';
    case 'failed':
    case 'rejected':
      return 'bad';
    case 'awaiting_approval':
      return 'warn';
    case 'queued':
    case 'cancelled':
      return 'dim';
    default:
      // resolving_source / building_context / provisioning / executing /
      // proving / reviewing / drafting_pr — all live work.
      return 'active';
  }
}

/** Humanize a snake_case status/kind into a sentence-case label
 *  ("awaiting_approval" → "Awaiting approval"; content.md: sentence case). */
export function humanize(s: string): string {
  if (!s) return '';
  const t = s.replace(/_/g, ' ').trim().toLowerCase();
  return t ? t[0].toUpperCase() + t.slice(1) : t;
}

const TONE_OF: Record<StatusTone, Tone> = {
  ok: 'success',
  bad: 'danger',
  warn: 'warning',
  active: 'info',
  dim: 'neutral',
};

/** A run status as the shared `StatusBadge` input: the run's own stage word
 *  ("Executing", "Awaiting approval" — more telling than a bare "Running")
 *  with the shared tone vocabulary (patterns.md §1: running is info, never
 *  the success green; the one human gate is the amber warning). */
export function runStatusInfo(status: RunStatus | string): StatusInfo {
  const tone = statusTone(status);
  const stage = STAGES.find((s) => s.statuses.includes(status));
  return {
    key: status,
    label: humanize(status),
    tone: TONE_OF[tone],
    live: tone === 'active' && !isTerminal(status),
    hint: stage ? `${stage.label}: ${stage.hint}` : undefined,
  };
}

// ---------------------------------------------------------------------------
// The pipeline rail — the fixed stage machine made visible. One entry per
// user-meaningful step; `statuses` are the RunStatus values the step covers.
// Mirrors otto-core's RunStatus::next_on_success ordering exactly.
// ---------------------------------------------------------------------------

export interface StageStep {
  key: string;
  label: string;
  /** Icon.svelte name. */
  icon: IconName;
  /** The RunStatus values that mean "this step is running right now". */
  statuses: readonly string[];
  /** One-line tooltip explaining the step. */
  hint: string;
}

export const STAGES: readonly StageStep[] = [
  { key: 'source', label: 'Source', icon: 'ticket', statuses: ['queued', 'resolving_source'], hint: 'Fetch + normalize the source item' },
  { key: 'context', label: 'Context', icon: 'note', statuses: ['building_context'], hint: 'Assemble the task prompt from the source' },
  { key: 'branch', label: 'Branch', icon: 'branch', statuses: ['provisioning'], hint: 'Isolated branch + worktree — your tree is never touched' },
  { key: 'execute', label: 'Execute', icon: 'terminal', statuses: ['executing'], hint: 'The agent (or goal loop) makes the change' },
  { key: 'proof', label: 'Proof', icon: 'gauge', statuses: ['proving'], hint: 'Evidence pack assembled from the produced diff' },
  { key: 'review', label: 'Review', icon: 'eye', statuses: ['reviewing'], hint: 'AI review of the branch → findings' },
  { key: 'approve', label: 'Approval', icon: 'user', statuses: ['awaiting_approval'], hint: 'The one human gate — nothing ships without you' },
  { key: 'pr', label: 'PR draft', icon: 'pr', statuses: ['drafting_pr', 'completed'], hint: 'PR draft generated; opening the real PR stays opt-in' },
] as const;

/** Index of the step a run is currently at. `completed` → the last step;
 *  `rejected` → the approval step (that is where a human said no);
 *  `failed`/`cancelled` → -1 (the rail dims — the timeline carries specifics). */
export function stageIndex(status: RunStatus | string): number {
  if (status === 'completed') return STAGES.length - 1;
  if (status === 'rejected') return STAGES.findIndex((s) => s.key === 'approve');
  const i = STAGES.findIndex((s) => s.statuses.includes(status));
  return i; // -1 for failed / cancelled / unknown
}

// ---------------------------------------------------------------------------
// Source-kind presentation: icon + label + paste template per SourceKind.
// Kinds are told apart by icon and label, never by a hue (no categorical
// rainbow — see docs/design/guidelines/foundations.md §1.3).
// (The `channel` kind is free text — it has no chip; anything unrecognized
// becomes a channel run.)
// ---------------------------------------------------------------------------

export interface SourceMeta {
  kind: string;
  label: string;
  icon: IconName;
  /** Text inserted into the launcher input when the chip is clicked. */
  template: string;
  /** Example shown as the chip tooltip. */
  example: string;
}

export const SOURCE_KINDS: readonly SourceMeta[] = [
  { kind: 'jira', label: 'Jira', icon: 'ticket', template: 'jira:', example: 'PROJ-123 (or jira:PROJ-123)' },
  { kind: 'confluence', label: 'Confluence', icon: 'file', template: 'confluence:', example: 'a page URL or confluence:<page id>' },
  { kind: 'github_pr', label: 'GitHub PR', icon: 'pr', template: 'https://github.com/', example: 'https://github.com/owner/repo/pull/42' },
  { kind: 'github_issue', label: 'GitHub issue', icon: 'comment', template: 'https://github.com/', example: 'https://github.com/owner/repo/issues/9' },
  { kind: 'product_story', label: 'Story', icon: 'layers', template: 'story:', example: 'story:<id> (Product module)' },
  { kind: 'finding', label: 'Finding', icon: 'radar', template: 'finding:', example: 'finding:<id> (review findings)' },
  { kind: 'test', label: 'Failing test', icon: 'function', template: 'test:', example: 'test:<id> (Product test case)' },
  { kind: 'scheduled_report', label: 'Report', icon: 'clock', template: 'report:', example: 'report:<id> (scheduled-task report)' },
] as const;

/** Tint for a SourceKind badge: neutral for every kind (icon + label carry
 *  the kind). Kept as a function so call sites stay put. */
export function sourceColor(_kind: string): string {
  return 'var(--text-dim)';
}

/** Short label for a SourceKind (falls back to humanized snake_case). */
export function sourceLabel(kind: string): string {
  if (kind === 'channel') return 'Chat / free text';
  return SOURCE_KINDS.find((s) => s.kind === kind)?.label ?? humanize(kind);
}
