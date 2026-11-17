// Token usage of runtime turns, in words ("12.4k tokens"). Tokens, not money:
// most people run on a subscription, so a token count is the honest measure.
// `turn_token_usage` arrives on `provider_turn.updated` (projection.ts).
// Pure, for any surface that shows a turn or a thread total
// (unit/threadsAttentionDock.test.ts).
import type { ProviderTurn, TurnTokenUsage } from '../../../lib/api/types.ts';

/** Tokens one turn used: what it read plus what it wrote (reasoning is part
 *  of the output). Null when the provider reported nothing. */
export function turnTokens(u: TurnTokenUsage | null | undefined): number | null {
  if (!u) return null;
  const n = (Number(u.input_tokens) || 0) + (Number(u.output_tokens) || 0);
  return n > 0 ? n : null;
}

/** "840 tokens", "12.4k tokens", "1.2M tokens"; '' for nothing. */
export function formatTokens(n: number | null | undefined): string {
  if (!n || !Number.isFinite(n) || n <= 0) return '';
  if (n < 1000) return `${Math.round(n)} ${n === 1 ? 'token' : 'tokens'}`;
  if (n < 1_000_000) {
    const k = n / 1000;
    return `${k < 100 ? trim1(k) : Math.round(k)}k tokens`;
  }
  const m = n / 1_000_000;
  return `${m < 100 ? trim1(m) : Math.round(m)}M tokens`;
}

function trim1(x: number): string {
  return (Math.round(x * 10) / 10).toFixed(1).replace(/\.0$/, '');
}

/** One turn's usage in words, '' while it runs or when unknown. */
export function turnUsageText(turn: Pick<ProviderTurn, 'status' | 'turn_token_usage'> | null | undefined): string {
  if (!turn || turn.status === 'running' || turn.status === 'pending') return '';
  return formatTokens(turnTokens(turn.turn_token_usage));
}

/** Every reported turn of a thread added up (a thread / child / seat total). */
export function threadTokenTotal(turns: readonly Pick<ProviderTurn, 'turn_token_usage'>[]): number {
  let total = 0;
  for (const t of turns) total += turnTokens(t.turn_token_usage) ?? 0;
  return total;
}
