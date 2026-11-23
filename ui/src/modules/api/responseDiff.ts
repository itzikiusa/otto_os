// Pure response-diff helpers for API Client ("Pin baseline" & "Diff vs previous").
// Formats JSON with canonical alphabetical key ordering so structural API
// regressions stand out immediately without noisy key-order churn.

export interface ResponseSnapshot {
  status: number;
  status_text: string;
  duration_ms: number;
  size_bytes: number;
  content_type: string | null;
  body: string;
  too_large?: boolean;
  label: string;
  capturedAt: number;
}

export const DIFF_MAX_CHARS = 256 * 1024;

/** Recursively sort object keys alphabetically while preserving array order. */
export function canonicalizeJson(value: unknown): unknown {
  if (Array.isArray(value)) return value.map(canonicalizeJson);
  if (value !== null && typeof value === 'object') {
    const src = value as Record<string, unknown>;
    const out: Record<string, unknown> = {};
    for (const k of Object.keys(src).sort()) out[k] = canonicalizeJson(src[k]);
    return out;
  }
  return value;
}

/** Format a response body for line/word/split diffing. When `canonicalJson` is
 *  enabled and the body parses as JSON (within `maxChars`), keys are sorted
 *  recursively and pretty-printed with 2-space indentation. */
export function formatForDiff(
  body: string,
  contentType: string | null,
  canonicalJson = true,
  maxChars = DIFF_MAX_CHARS,
): { text: string; isJson: boolean; truncated: boolean } {
  if (body.length > maxChars) {
    return {
      text: `${body.slice(0, maxChars)}\n… (truncated at ${Math.round(maxChars / 1024)} KB for diff)`,
      isJson: false,
      truncated: true,
    };
  }
  const t = body.trim();
  const maybeJson =
    (contentType != null && /\bjson\b/i.test(contentType)) ||
    (t.startsWith('{') && t.endsWith('}')) ||
    (t.startsWith('[') && t.endsWith(']'));
  if (maybeJson && t.length > 0) {
    try {
      const parsed: unknown = JSON.parse(t);
      const formatted = JSON.stringify(canonicalJson ? canonicalizeJson(parsed) : parsed, null, 2);
      return { text: formatted, isJson: true, truncated: false };
    } catch {
      /* fall through to raw text */
    }
  }
  return { text: body, isJson: false, truncated: false };
}

export interface ResponseDelta {
  statusChanged: boolean;
  statusFrom: number;
  statusTo: number;
  durationDeltaMs: number;
  sizeDeltaBytes: number;
  identical: boolean;
}

/** Compute compact status, latency, size, and formatted-body equality deltas. */
export function summarizeResponseDelta(
  before: ResponseSnapshot,
  after: ResponseSnapshot,
  canonicalJson = true,
): ResponseDelta {
  const b = formatForDiff(before.body, before.content_type, canonicalJson);
  const a = formatForDiff(after.body, after.content_type, canonicalJson);
  return {
    statusChanged: before.status !== after.status,
    statusFrom: before.status,
    statusTo: after.status,
    durationDeltaMs: after.duration_ms - before.duration_ms,
    sizeDeltaBytes: after.size_bytes - before.size_bytes,
    identical: !before.too_large && !after.too_large && b.text === a.text,
  };
}

/** Create a comparable snapshot from an API response object. */
export function snapshotFromResponse(
  r: {
    status: number;
    status_text: string;
    duration_ms: number;
    size_bytes: number;
    content_type: string | null;
    body: string;
    too_large?: boolean;
  },
  label: string,
  capturedAt = Date.now(),
): ResponseSnapshot {
  return {
    status: r.status,
    status_text: r.status_text,
    duration_ms: r.duration_ms,
    size_bytes: r.size_bytes,
    content_type: r.content_type,
    body: r.body,
    too_large: r.too_large,
    label,
    capturedAt,
  };
}

export interface TabDiffState {
  latest: ResponseSnapshot | null;
  previous: ResponseSnapshot | null;
  pinned: ResponseSnapshot | null;
}

/** Bounded per-tab tracker for previous and pinned baseline responses. */
export class ResponseDiffHistory {
  private readonly map = new Map<string, TabDiffState>();
  private readonly maxTabs: number;

  constructor(maxTabs = 24) {
    this.maxTabs = maxTabs;
  }

  private ensure(tabKey: string): TabDiffState {
    let entry = this.map.get(tabKey);
    if (!entry) {
      entry = { latest: null, previous: null, pinned: null };
      this.map.set(tabKey, entry);
      if (this.map.size > this.maxTabs) {
        const oldest = this.map.keys().next().value;
        if (oldest !== undefined) this.map.delete(oldest);
      }
    }
    return entry;
  }

  /** Record a response observation for `tabKey`. If `ref` is a new response
   *  object (distinct from the previous observation), shifts prior `latest`
   *  into `previous`. Returns the updated state. */
  observe(tabKey: string, snap: ResponseSnapshot): TabDiffState {
    const st = this.ensure(tabKey);
    if (st.latest && st.latest !== snap) {
      st.previous = { ...st.latest, label: 'Previous' };
    }
    st.latest = snap;
    return st;
  }

  pin(tabKey: string, snap: ResponseSnapshot): TabDiffState {
    const st = this.ensure(tabKey);
    st.pinned = { ...snap, label: 'Pinned baseline' };
    return st;
  }

  unpin(tabKey: string): TabDiffState {
    const st = this.ensure(tabKey);
    st.pinned = null;
    return st;
  }

  get(tabKey: string): TabDiffState {
    return this.map.get(tabKey) ?? { latest: null, previous: null, pinned: null };
  }
}
