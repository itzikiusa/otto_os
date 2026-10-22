// Per-session tokens + cost (review A5) — the PURE half (no runes) so
// node:test can cover the formatting. Tokens lead, cost follows: the token
// count is what the user controls; cost is a list-price estimate.
import { formatCount, formatUsd } from './metric-format.ts';

/** Mirrors `SessionTotals` (types.ts / otto-usage). */
export interface SessionTotalsLike {
  total_tokens: number;
  cost_usd: number;
}

/** 950 → "950", 12_300 → "12.3k", 2_100_000 → "2.1M" — the shared
 *  `formatCount`, so the sidebar chip and the conversation view read the same
 *  token count the same way; only the "no usage" floor is local. */
export function formatTokens(n: number): string {
  return !Number.isFinite(n) || n < 0 ? '0' : formatCount(n);
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
