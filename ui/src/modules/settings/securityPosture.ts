// Pure view-model helpers for the Settings -> Trust & safety Security Posture
// card (`GET /api/v1/security-posture`). Kept free of Svelte/DOM imports so
// `node --test` covers sorting, grouping, status mapping, and summary copy
// directly (`unit/securityPosture.test.ts`).

import type {
  SecurityPostureCheck,
  SecurityPostureCheckStatus,
  SecurityPostureOverall,
} from '../../lib/api/types';
import type { BadgeTone, StatusInfo, Tone } from '../../lib/status';
import { plural } from '../../lib/plural.ts';

/** Severity sort rank for individual checks: fail -> warn -> pass. */
export const CHECK_STATUS_RANK: Record<SecurityPostureCheckStatus, number> = {
  fail: 0,
  warn: 1,
  pass: 2,
};

/** Canonical display order and labels for known backend check categories. */
export const CATEGORY_ORDER = [
  'secrets',
  'filesystem',
  'network',
  'access',
  'plugins',
  'database',
] as const;

const CATEGORY_LABELS: Record<string, string> = {
  secrets: 'Secrets',
  filesystem: 'Filesystem',
  network: 'Network',
  access: 'Access',
  plugins: 'Plugins',
  database: 'Database',
};

export interface PostureCounts {
  fail: number;
  warn: number;
  pass: number;
  total: number;
}

export interface PostureCategoryGroup {
  category: string;
  label: string;
  checks: SecurityPostureCheck[];
  worstStatus: SecurityPostureCheckStatus;
  counts: PostureCounts;
}

export interface CheckStatusDisplay {
  key: SecurityPostureCheckStatus;
  label: string;
  tone: Tone;
  badgeTone: BadgeTone;
}

/** Map a check status to its shared semantic tone and label. */
export function checkStatusInfo(status: SecurityPostureCheckStatus): CheckStatusDisplay {
  switch (status) {
    case 'fail':
      return { key: 'fail', label: 'Fail', tone: 'danger', badgeTone: 'bad' };
    case 'warn':
      return { key: 'warn', label: 'Warning', tone: 'warning', badgeTone: 'warn' };
    case 'pass':
      return { key: 'pass', label: 'Pass', tone: 'success', badgeTone: 'ok' };
  }
}

/** Derive an overall verdict from checks when the daemon omits `overall`. */
export function deriveOverallPosture(
  overall: SecurityPostureOverall | undefined,
  checks: readonly SecurityPostureCheck[] | undefined,
): SecurityPostureOverall {
  if (overall === 'critical' || overall === 'warn' || overall === 'healthy') {
    return overall;
  }
  const list = checks ?? [];
  if (list.some((c) => c.status === 'fail')) return 'critical';
  if (list.some((c) => c.status === 'warn')) return 'warn';
  return 'healthy';
}

/** Map an overall posture verdict to StatusInfo for `<StatusBadge>`. */
export function overallPostureInfo(
  overall: SecurityPostureOverall | undefined,
  checks: readonly SecurityPostureCheck[] | undefined,
): StatusInfo {
  const key = deriveOverallPosture(overall, checks);
  switch (key) {
    case 'critical':
      return {
        key: 'critical',
        label: 'Critical',
        tone: 'danger',
        hint: 'One or more security checks failed and need remediation',
      };
    case 'warn':
      return {
        key: 'warn',
        label: 'Needs attention',
        tone: 'warning',
        hint: 'One or more security checks reported warnings',
      };
    case 'healthy':
      return {
        key: 'healthy',
        label: 'Healthy',
        tone: 'success',
        hint: 'All security checks passed',
      };
  }
}

/** Human-readable sentence-case label for a check category. */
export function categoryLabel(category: string): string {
  const key = category.trim().toLowerCase();
  if (!key) return 'General';
  if (CATEGORY_LABELS[key]) return CATEGORY_LABELS[key];
  const spaced = key.replace(/[_-]+/g, ' ').trim();
  return spaced ? spaced[0].toUpperCase() + spaced.slice(1) : 'General';
}

function categoryOrderRank(category: string): number {
  const idx = CATEGORY_ORDER.indexOf(category as (typeof CATEGORY_ORDER)[number]);
  return idx === -1 ? CATEGORY_ORDER.length : idx;
}

/** Return a new array of checks sorted fail -> warn -> pass, then title/id. */
export function sortChecks(checks: readonly SecurityPostureCheck[]): SecurityPostureCheck[] {
  return [...checks].sort((a, b) => {
    const rankDiff = CHECK_STATUS_RANK[a.status] - CHECK_STATUS_RANK[b.status];
    if (rankDiff !== 0) return rankDiff;
    const titleDiff = a.title.localeCompare(b.title);
    if (titleDiff !== 0) return titleDiff;
    return a.id.localeCompare(b.id);
  });
}

/** Count fail, warn, pass, and total checks. */
export function summarizeChecks(checks: readonly SecurityPostureCheck[] | undefined): PostureCounts {
  let fail = 0;
  let warn = 0;
  let pass = 0;
  for (const c of checks ?? []) {
    if (c.status === 'fail') fail += 1;
    else if (c.status === 'warn') warn += 1;
    else pass += 1;
  }
  return { fail, warn, pass, total: fail + warn + pass };
}

/** Group checks by category, sorting checks within each group and groups by worst severity first. */
export function groupChecksByCategory(checks: readonly SecurityPostureCheck[] | undefined): PostureCategoryGroup[] {
  const buckets = new Map<string, SecurityPostureCheck[]>();
  for (const c of checks ?? []) {
    const key = c.category.trim().toLowerCase() || 'general';
    const list = buckets.get(key);
    if (list) list.push(c);
    else buckets.set(key, [c]);
  }

  const groups: PostureCategoryGroup[] = [];
  for (const [category, raw] of buckets) {
    const sorted = sortChecks(raw);
    const counts = summarizeChecks(sorted);
    const worstStatus: SecurityPostureCheckStatus =
      counts.fail > 0 ? 'fail' : counts.warn > 0 ? 'warn' : 'pass';
    groups.push({
      category,
      label: categoryLabel(category),
      checks: sorted,
      worstStatus,
      counts,
    });
  }

  return groups.sort((a, b) => {
    const sevDiff = CHECK_STATUS_RANK[a.worstStatus] - CHECK_STATUS_RANK[b.worstStatus];
    if (sevDiff !== 0) return sevDiff;
    const orderDiff = categoryOrderRank(a.category) - categoryOrderRank(b.category);
    if (orderDiff !== 0) return orderDiff;
    return a.label.localeCompare(b.label);
  });
}

/** Whether a check provides a non-empty remediation instruction. */
export function hasRemediation(check: SecurityPostureCheck): boolean {
  return typeof check.remediation === 'string' && check.remediation.trim().length > 0;
}

/** Compact summary sentence for the posture header. */
export function postureSummaryLine(counts: PostureCounts): string {
  if (counts.total === 0) return 'No security checks reported';
  if (counts.fail === 0 && counts.warn === 0) {
    return counts.total === 1 ? '1 check passed' : `All ${plural(counts.total, 'check')} passed`;
  }
  const parts: string[] = [];
  if (counts.fail > 0) parts.push(`${counts.fail} failed`);
  if (counts.warn > 0) parts.push(plural(counts.warn, 'warning'));
  if (counts.pass > 0) parts.push(`${counts.pass} passed`);
  return parts.join(' · ');
}
