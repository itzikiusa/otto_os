import type { Finding, FindingSeverity, FindingStatus } from '../../lib/api/types';

// Triage of a long findings board: group (by file, or by severity), sort by
// severity inside a group, and which bulk action applies to which finding
// (mirrors FindingStatus::can_transition in crates/otto-core/src/finding.rs).

export type GroupBy = 'file' | 'severity';
export type BulkAction = 'accept' | 'false_positive' | 'waive';

const SEV_ORDER: FindingSeverity[] = ['critical', 'high', 'medium', 'low', 'info'];
const sevRank = (s: FindingSeverity): number => {
  const i = SEV_ORDER.indexOf(s);
  return i < 0 ? SEV_ORDER.length : i;
};

/** Severity first (critical → info), then file, then line. */
export function bySeverity(a: Finding, b: Finding): number {
  return sevRank(a.severity) - sevRank(b.severity) || (a.path ?? '').localeCompare(b.path ?? '') || (a.line ?? 0) - (b.line ?? 0);
}

export interface FindingGroup {
  key: string;
  label: string;
  items: Finding[];
}

/** Groups in a stable order — files A→Z (general findings last), or
 *  severities critical → info — each sorted by severity then line. */
export function groupFindings(findings: Finding[], by: GroupBy): FindingGroup[] {
  const groups = new Map<string, Finding[]>();
  for (const f of findings) {
    const key = by === 'file' ? (f.path ?? '') : f.severity;
    groups.set(key, [...(groups.get(key) ?? []), f]);
  }
  const keys = [...groups.keys()].sort((a, b) =>
    by === 'severity'
      ? sevRank(a as FindingSeverity) - sevRank(b as FindingSeverity)
      : a === '' ? 1 : b === '' ? -1 : a.localeCompare(b),
  );
  return keys.map((key) => ({
    key,
    label: by === 'file' ? key || 'General' : key.charAt(0).toUpperCase() + key.slice(1),
    items: [...(groups.get(key) ?? [])].sort(bySeverity),
  }));
}

const CAN: Record<BulkAction, FindingStatus[]> = {
  accept: ['open'],
  false_positive: ['open', 'accepted', 'fixed', 'verified'],
  waive: ['open', 'accepted', 'fixed'],
};

/** Whether `action` is a legal transition from the finding's status. */
export function canBulk(f: Finding, action: BulkAction): boolean {
  return CAN[action].includes(f.status);
}

/** The next focus index for a j / k press, clamped to the list. */
export function moveFocus(current: number, key: string, length: number): number {
  if (length === 0) return -1;
  if (key === 'j') return Math.min(length - 1, current + 1);
  if (key === 'k') return Math.max(0, current - 1);
  return current;
}
