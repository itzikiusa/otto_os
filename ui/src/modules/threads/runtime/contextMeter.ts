// How full the agent's context window is (T3 Code's contextWindow.ts, MIT):
// used / window from the thread's `provider_thread.context_usage`, which
// every structured driver already reports on the event stream (Claude's
// result.modelUsage contextWindow, Codex token_count modelContextWindow, ACP
// usage_update, pi's context window). Nothing to show without a window.
// Pure (unit/runtimeContextMeter.test.ts).
import type { ThreadTokenUsage } from '../../../lib/api/types';

export interface ContextMeter {
  /** 0–100, rounded. */
  pct: number;
  used: number;
  max: number;
  tone: 'ok' | 'warning' | 'danger';
  /** "122k of 200k tokens (61 %)". */
  text: string;
}

/** 1234 → "1.2k", 200000 → "200k", 1048576 → "1M". */
export function tokens(n: number): string {
  if (n >= 1_000_000) return `${+(n / 1_000_000).toFixed(n >= 10_000_000 ? 0 : 1)}M`;
  if (n >= 1000) return `${+(n / 1000).toFixed(n >= 10_000 ? 0 : 1)}k`;
  return `${n}`;
}

export function contextMeter(u: ThreadTokenUsage | null | undefined): ContextMeter | null {
  const max = u?.max_tokens ?? 0;
  if (!u || max <= 0) return null;
  const used = Math.max(0, u.used_tokens);
  const pct = Math.min(100, Math.round((used / max) * 100));
  const tone = pct >= 95 ? 'danger' : pct >= 80 ? 'warning' : 'ok';
  return { pct, used, max, tone, text: `${tokens(used)} of ${tokens(max)} tokens (${pct} %)` };
}
