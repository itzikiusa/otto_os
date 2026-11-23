// One calm failure state for a turn that failed (or a thread that could not
// start): a plain-language cause line, the provider's raw text kept for
// "Details", and which timeline error row it replaces so the same failure is
// not said twice. Pure (unit/threadsAttentionDock.test.ts).
import type { ProviderFailure, TurnItem } from '../../../lib/api/types.ts';

export type FailureCause = 'signed_out' | 'usage_limit' | 'cli_missing' | 'cli_unsupported' | 'connection' | 'provider';

export interface FailureView {
  cause: FailureCause;
  /** "Claude Code isn’t signed in" — what the person reads first. */
  title: string;
  /** What to do about it, in one short sentence, or '' when the actions say it. */
  hint: string;
  /** The provider's own words, behind "Details" ('' when there are none). */
  detail: string;
}

/** Classify a provider failure (class + message) into a cause people know. */
export function failureCause(f: Pick<ProviderFailure, 'class' | 'message'> & { code?: string | null } | null): FailureCause {
  const cls = f?.class ?? '';
  const msg = f?.message ?? '';
  // Codex states it (`codexErrorInfo: "usageLimitExceeded"` → `code`); the rest say it in words.
  if (cls === 'usage_limit' || /usage.?limit/i.test(f?.code ?? '') || /usage.?limit|rate.?limit|quota|limit (?:reached|exceeded)|resource_exhausted|\b429\b|too many requests/i.test(msg)) {
    return 'usage_limit';
  }
  if (/not (?:logged|signed) in|log ?in required|please run \/login|\/login|unauthori[sz]ed|\b401\b|invalid api key|authentication|expired (?:token|credentials|session)|oauth/i.test(msg)) {
    return 'signed_out';
  }
  if (/command not found|no such file or directory|ENOENT|executable (?:not found|unavailable)|not installed|cli is disabled|fail(?:ed)? to spawn|cannot find .*(?:cli|binary)/i.test(msg)) {
    return 'cli_missing';
  }
  if (
    cls === 'handshake_error' || cls === 'handshake_timeout' || cls === 'protocol_error' ||
    /unsupported control request|unsupported (?:protocol|version|method)|unknown (?:option|flag|method)|unrecognized (?:option|argument)|upgrade|too old/i.test(msg)
  ) {
    return 'cli_unsupported';
  }
  if (cls === 'transport_error' || /connection (?:reset|refused|closed)|broken pipe|exited unexpectedly|process exited|EOF/i.test(msg)) {
    return 'connection';
  }
  return 'provider';
}

/** The failure block's words for `agent` ("Claude Code", "Codex"). */
export function failureView(f: Pick<ProviderFailure, 'class' | 'message'> & { code?: string | null } | null, agent: string): FailureView {
  const cause = failureCause(f);
  const detail = (f?.message ?? '').trim();
  switch (cause) {
    case 'signed_out':
      return { cause, title: `${agent} isn’t signed in`, hint: `Sign in to ${agent} in a terminal, then send again.`, detail };
    case 'usage_limit':
      return { cause, title: `${agent} reached its usage limit`, hint: '', detail }; // the composer line has the next step
    case 'cli_missing':
      return { cause, title: `${agent} isn’t installed or can’t start`, hint: `Install ${agent}, then send again.`, detail };
    case 'cli_unsupported':
      return { cause, title: `This version of ${agent} isn’t supported`, hint: `Update ${agent}, then send again.`, detail };
    case 'connection':
      return { cause, title: `${agent} stopped unexpectedly`, hint: '', detail };
    default:
      return { cause, title: `${agent} couldn’t finish this turn`, hint: '', detail };
  }
}

/** The error row a failed turn's block stands in for: the turn's last
 *  top-level `error` item. The timeline hides it while the block shows. */
export function turnErrorItem(items: readonly TurnItem[], turnId: string | null | undefined): Extract<TurnItem, { type: 'error' }> | null {
  if (!turnId) return null;
  for (let i = items.length - 1; i >= 0; i--) {
    const it = items[i];
    if (it.turn_id === turnId && !it.parent_item_id && it.type === 'error') return it;
  }
  return null;
}
