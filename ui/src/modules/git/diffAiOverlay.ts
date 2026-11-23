// Pure overlay/matching helpers for E-08 (Inline AI Review Overlay in DiffViewer).
// Maps active review draft comments (ReviewComment[]) and persisted findings
// (Finding[]) onto normalized (filePath, line) keys and computes total + per-file
// counts. Free of Svelte/DOM imports so `node --test` can import it directly.

import type { Finding, FindingSeverity, ReviewComment } from '../../lib/api/types';
import type { BadgeTone } from '../../lib/status';

export type AiOverlaySeverity = ReviewComment['severity'] | FindingSeverity;

export type AiOverlayItem =
  | {
      kind: 'comment';
      id: string;
      key: string;
      path: string;
      line: number | null;
      severity: ReviewComment['severity'];
      title: string | null;
      body: string;
      comment: ReviewComment;
    }
  | {
      kind: 'finding';
      id: string;
      key: string;
      path: string;
      line: number | null;
      lineEnd: number | null;
      severity: FindingSeverity;
      title: string;
      body: string;
      finding: Finding;
    };

export interface AiOverlayIndex {
  /** Total active overlay items mapped to a file path. */
  total: number;
  /** Normalized file path → item count (line-anchored + file-level). */
  byFile: ReadonlyMap<string, number>;
  /** `${normalizedPath}\0${line}` → items anchored to that line. */
  byLine: ReadonlyMap<string, readonly AiOverlayItem[]>;
  /** Normalized file path → file-level items with no line number. */
  fileLevel: ReadonlyMap<string, readonly AiOverlayItem[]>;
}

export interface DiffAiOverlayActions {
  onEditComment?: (comment: ReviewComment, body: string) => Promise<boolean>;
  onPostComment?: (comment: ReviewComment) => Promise<boolean>;
  onDismissItem?: (item: AiOverlayItem) => Promise<boolean>;
  onFixItem?: (item: AiOverlayItem) => Promise<boolean>;
}

export interface DiffAiOverlayData extends DiffAiOverlayActions {
  comments: readonly ReviewComment[];
  findings: readonly Finding[];
}

const EMPTY_ITEMS: readonly AiOverlayItem[] = Object.freeze([]);
const ACTIVE_FINDING_STATUSES = new Set<string>(['open', 'accepted']);

/** Normalize a diff/review path so `./src/a.ts`, `/src/a.ts`, `src\\a.ts` match `src/a.ts`. */
export function normalizeOverlayPath(path: string | null | undefined): string {
  if (!path) return '';
  return path.replace(/\\/g, '/').replace(/^(?:\.\/|\/)+/, '').trim();
}

/** Stable map key for `(filePath, line)`. */
export function overlayLineKey(path: string, line: number): string {
  return `${normalizeOverlayPath(path)}\0${line}`;
}

/** True when a draft review comment should appear in the diff AI overlay. */
export function isActiveOverlayComment(c: ReviewComment): boolean {
  if (!normalizeOverlayPath(c.path)) return false;
  return c.state === 'draft' || (c.state === 'approved' && !c.posted);
}

/** True when a persisted finding should appear in the diff AI overlay. */
export function isActiveOverlayFinding(f: Finding): boolean {
  if (!normalizeOverlayPath(f.path)) return false;
  return ACTIVE_FINDING_STATUSES.has(f.status);
}

function normSnippet(text: string): string {
  return text.replace(/\s+/g, ' ').trim().toLowerCase().slice(0, 120);
}

function severityRank(s: AiOverlaySeverity): number {
  switch (s) {
    case 'critical': return 0;
    case 'bug':
    case 'high': return 1;
    case 'warn':
    case 'medium': return 2;
    case 'low': return 3;
    default: return 4;
  }
}

/** Map a review comment or finding severity onto the shared BadgeTone vocabulary. */
export function aiOverlaySeverityTone(severity: AiOverlaySeverity): BadgeTone {
  switch (severity) {
    case 'critical':
    case 'high':
    case 'bug':
      return 'bad';
    case 'medium':
    case 'warn':
      return 'warn';
    case 'low':
      return 'neutral';
    default:
      return 'info';
  }
}

/** Human-readable severity label for the card badge. */
export function aiOverlaySeverityLabel(severity: AiOverlaySeverity): string {
  switch (severity) {
    case 'bug': return 'Bug';
    case 'warn': return 'Warning';
    case 'critical': return 'Critical';
    case 'high': return 'High';
    case 'medium': return 'Medium';
    case 'low': return 'Low';
    default: return 'Info';
  }
}

/** Build the lookup index from active draft comments and persisted findings.
 *  When a finding duplicates a draft comment already placed on the same
 *  (path, line) with matching body/title text, the actionable draft comment
 *  wins so the same issue is not rendered twice on one line. */
export function buildAiOverlayIndex(
  comments: readonly ReviewComment[],
  findings: readonly Finding[],
): AiOverlayIndex {
  const byFile = new Map<string, number>();
  const byLine = new Map<string, AiOverlayItem[]>();
  const fileLevel = new Map<string, AiOverlayItem[]>();
  const commentFingerprints = new Map<string, string[]>();
  let total = 0;

  const push = (item: AiOverlayItem): void => {
    total += 1;
    byFile.set(item.path, (byFile.get(item.path) ?? 0) + 1);
    if (item.line !== null && item.line > 0) {
      const k = overlayLineKey(item.path, item.line);
      const list = byLine.get(k);
      if (list) list.push(item);
      else byLine.set(k, [item]);
    } else {
      const list = fileLevel.get(item.path);
      if (list) list.push(item);
      else fileLevel.set(item.path, [item]);
    }
  };

  for (const c of comments) {
    if (!isActiveOverlayComment(c)) continue;
    const path = normalizeOverlayPath(c.path);
    const line = typeof c.line === 'number' && c.line > 0 ? c.line : null;
    push({
      kind: 'comment',
      id: c.id,
      key: `c:${c.id}`,
      path,
      line,
      severity: c.severity,
      title: null,
      body: c.body,
      comment: c,
    });
    const locKey = `${path}\0${line ?? ''}`;
    const bodyNorm = normSnippet(c.body);
    if (bodyNorm) {
      const arr = commentFingerprints.get(locKey);
      if (arr) arr.push(bodyNorm);
      else commentFingerprints.set(locKey, [bodyNorm]);
    }
  }

  for (const f of findings) {
    if (!isActiveOverlayFinding(f)) continue;
    const path = normalizeOverlayPath(f.path);
    const line = typeof f.line === 'number' && f.line > 0 ? f.line : null;
    const locKey = `${path}\0${line ?? ''}`;
    const existing = commentFingerprints.get(locKey);
    if (existing) {
      const titleNorm = normSnippet(f.title);
      const bodyNorm = normSnippet(f.body);
      const duplicate = existing.some(
        (cText) =>
          (bodyNorm !== '' && (cText.includes(bodyNorm) || bodyNorm.includes(cText))) ||
          (titleNorm !== '' && (cText.includes(titleNorm) || titleNorm.includes(cText))),
      );
      if (duplicate) continue;
    }
    push({
      kind: 'finding',
      id: f.id,
      key: `f:${f.id}`,
      path,
      line,
      lineEnd: typeof f.line_end === 'number' && f.line_end > 0 ? f.line_end : null,
      severity: f.severity,
      title: f.title,
      body: f.body,
      finding: f,
    });
  }

  const sortItems = (list: AiOverlayItem[]): void => {
    list.sort((a, b) => severityRank(a.severity) - severityRank(b.severity) || a.key.localeCompare(b.key));
  };
  for (const list of byLine.values()) sortItems(list);
  for (const list of fileLevel.values()) sortItems(list);

  return { total, byFile, byLine, fileLevel };
}

/** Return overlay items attached to a diff row (prefers `newLine`; falls back to `oldLine` when `newLine` is null). */
export function aiOverlayForLine(
  index: AiOverlayIndex,
  filePath: string,
  newLine: number | null,
  oldLine: number | null = null,
): readonly AiOverlayItem[] {
  if (index.total === 0) return EMPTY_ITEMS;
  const line = newLine ?? oldLine;
  if (line === null || line <= 0) return EMPTY_ITEMS;
  return index.byLine.get(overlayLineKey(filePath, line)) ?? EMPTY_ITEMS;
}

/** Total overlay items (line-anchored + file-level) for one file path. */
export function aiOverlayCountForFile(index: AiOverlayIndex, filePath: string): number {
  if (index.total === 0) return 0;
  return index.byFile.get(normalizeOverlayPath(filePath)) ?? 0;
}

/** File-level overlay items (items with no line anchor) for one file path. */
export function aiOverlayForFile(index: AiOverlayIndex, filePath: string): readonly AiOverlayItem[] {
  if (index.total === 0) return EMPTY_ITEMS;
  return index.fileLevel.get(normalizeOverlayPath(filePath)) ?? EMPTY_ITEMS;
}
