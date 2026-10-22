// Per-session tokens + cost (review A5) — the PURE half (no runes) so
// node:test can cover the formatting. Tokens lead, cost follows: the token
// count is what the user controls; cost is a list-price estimate.
import { formatUsd } from './metric-format.ts';

/** Mirrors `SessionTotals` (types.ts / otto-usage). */
export interface SessionTotalsLike {
  total_tokens: number;
  cost_usd: number;
}

/** 950 → "950", 12_300 → "12.3K", 2_100_000 → "2.1M". */
export function formatTokens(n: number): string {
  if (!Number.isFinite(n) || n < 0) return '0';
  if (n < 1000) return String(Math.round(n));
  const units: [number, string][] = [
    [1e9, 'B'],
    [1e6, 'M'],
    [1e3, 'K'],
  ];
  for (const [v, u] of units) {
    if (n >= v) {
      const x = n / v;
      return `${x >= 100 ? Math.round(x) : x.toFixed(1).replace(/\.0$/, '')}${u}`;
    }
  }
  return String(n);
}

/** $0.004 → "<$0.01", 1.239 → "$1.24", 1234 → "$1,234". */
export function formatCost(usd: number): string {
  // A negative or missing total is "no spend" on this chip.
  return !Number.isFinite(usd) || usd <= 0 ? '$0' : formatUsd(usd);
}

/** "2.1M tokens · $1.24" — tokens first, cost second; null for no usage. */
export function sessionUsageLabel(t: SessionTotalsLike | null | undefined): string | null {
  if (!t || !(t.total_tokens > 0)) return null;
  return `${formatTokens(t.total_tokens)} tokens · ${formatCost(t.cost_usd)}`;
}
