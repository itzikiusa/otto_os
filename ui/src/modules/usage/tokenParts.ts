// Token buckets (input · cache write · cache read · output) and the labels the
// Usage page's pieces share, so the chart, the "Where it went" rows and the
// sessions list split tokens the same way and in the same colours.
import { kindLabel } from '../../lib/labels';

export interface TokenParts {
  input_tokens: number;
  output_tokens: number;
  cache_read_tokens: number;
  cache_write_tokens: number;
}

const TOKEN_CATS = [
  { label: 'Input', color: 'var(--cat-1)', pick: (o: TokenParts) => o.input_tokens },
  { label: 'Cache write', color: 'var(--cat-2)', pick: (o: TokenParts) => o.cache_write_tokens },
  { label: 'Cache read', color: 'var(--cat-3)', pick: (o: TokenParts) => o.cache_read_tokens },
  { label: 'Output', color: 'var(--cat-4)', pick: (o: TokenParts) => o.output_tokens },
] as const;

export interface Seg {
  label: string;
  color: string;
  v: number;
  pct: number;
}

export function tokenSegs(o: TokenParts): Seg[] {
  const total = o.input_tokens + o.output_tokens + o.cache_read_tokens + o.cache_write_tokens;
  return TOKEN_CATS.map((c) => {
    const v = c.pick(o);
    return { label: c.label, color: c.color, v, pct: total > 0 ? (v / total) * 100 : 0 };
  });
}

/** "Input 1,200 · Cache write 300 · …" — a row's tooltip. */
export function breakdownTitle(o: TokenParts): string {
  return tokenSegs(o)
    .map((s) => `${s.label} ${s.v.toLocaleString()}`)
    .join(' · ');
}

/** The summary carries the same four numbers under total_* names. */
export function summaryParts(s: {
  total_input_tokens: number;
  total_output_tokens: number;
  total_cache_read_tokens: number;
  total_cache_write_tokens: number;
}): TokenParts {
  return {
    input_tokens: s.total_input_tokens,
    output_tokens: s.total_output_tokens,
    cache_read_tokens: s.total_cache_read_tokens,
    cache_write_tokens: s.total_cache_write_tokens,
  };
}

// The by-kind buckets the server emits that read better than the shared kind
// label; every other kind (goal_loop, scheduled_task …) takes lib/labels'.
const FEATURE_LABELS: Record<string, string> = {
  review: 'Code review',
  product: 'Product AI',
  channel: 'Channels',
  agent: 'Ad-hoc agents',
  connection: 'Connections',
  thread: 'Threads',
  external: 'External',
  deleted: 'Deleted sessions',
};

export function featureLabel(k: string): string {
  return FEATURE_LABELS[k] ?? (k ? kindLabel(k) : 'Other');
}

const MONTHS = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec'];

/** "2026-06-16" → "Jun 16" (a bare "06-16" reads as either order). */
export function shortDay(iso: string): string {
  const m = iso.match(/^\d{4}-(\d{2})-(\d{2})/);
  if (!m) return iso;
  return `${MONTHS[parseInt(m[1], 10) - 1] ?? m[1]} ${parseInt(m[2], 10)}`;
}

/** "2026-06-16 14:32:05.123" → "Jun 16, 14:32" (formatted as stored, no
 *  timezone shift; the date matters in a window of up to 180 days). */
export function fmtLastActive(iso: string): string {
  const m = iso.match(/(\d{4})-(\d{2})-(\d{2})[ T](\d{2}:\d{2})/);
  if (!m) return iso;
  return `${MONTHS[parseInt(m[2], 10) - 1] ?? m[2]} ${parseInt(m[3], 10)}, ${m[4]}`;
}
