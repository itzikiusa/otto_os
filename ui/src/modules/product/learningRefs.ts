// Helpers for LearningsView tags and reference chips.
// `refHref` enforces http(s) schemes (stripping control/whitespace chars first)
// so AI-suggested or imported learning refs can never produce javascript:/data: links.

export interface LearningRef {
  type: string;
  ref: string;
  label?: string;
}

export function parseLearningTags(tags: string): string[] {
  return tags
    ? tags
        .split(',')
        .map((t) => t.trim())
        .filter(Boolean)
    : [];
}

export function parseLearningRefs(refsJson: string): LearningRef[] {
  if (!refsJson) return [];
  try {
    const parsed = JSON.parse(refsJson);
    if (Array.isArray(parsed)) return parsed as LearningRef[];
  } catch {
    // ignore malformed
  }
  return [];
}

export function learningRefHref(r: LearningRef): string | null {
  if (r.type !== 'url' && r.type !== 'jira' && r.type !== 'confluence') return null;
  const raw = (r.ref ?? '').trim();
  // eslint-disable-next-line no-control-regex
  const probe = raw.replace(/[\u0000-\u0020\u007f-\u00a0\s]/g, '').toLowerCase();
  return /^https?:\/\//.test(probe) ? raw : null;
}

export function learningRefClass(type: string): string {
  switch (type) {
    case 'jira': return 'ref-jira';
    case 'confluence': return 'ref-confluence';
    case 'url': return 'ref-url';
    default: return 'ref-other';
  }
}
