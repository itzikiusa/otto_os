// Display text for the Kafka figures the daemon reports as "unknown" with
// `null`: cluster throughput before two watermark sweeps exist, and a
// consumer group's lag that could not be (or is not yet) computed. Unknown
// must never render as a hard 0 — that read as "no traffic" / "caught up".

/** Overview throughput: `null` rate → "measuring…" while samples are still
 *  arriving, "—" when the metrics request failed. */
export function rateText(rate: number | null | undefined, failed: boolean): string {
  if (rate == null) return failed ? '—' : 'measuring…';
  return Math.round(rate).toLocaleString();
}

/** Overview total: `null` until the first sweep lands. */
export function totalText(total: number | null | undefined): string {
  return total == null ? '—' : total.toLocaleString();
}

/** One group row's lag badge: the text and its tooltip. */
export function lagBadge(lag: number | null | undefined, pending: boolean | undefined, protocolType: string): { text: string; title: string } {
  if (lag != null) return { text: `lag ${lag.toLocaleString()}`, title: 'Total lag across the group’s committed partitions' };
  if (pending) return { text: 'lag …', title: 'Computing lag…' };
  if (protocolType !== 'consumer' && protocolType !== '') {
    return { text: 'lag —', title: `Not a consumer group (${protocolType}) — no committed offsets to measure` };
  }
  return { text: 'lag —', title: 'Lag unavailable: no committed offsets, or the offsets could not be read. Open the group for details.' };
}

/** Delay before re-fetching the group list while lags are still computing
 *  (5 s, 10 s, … capped at 30 s; stops after 8 polls), or `null` when none is. */
export function nextLagPoll(groups: { lag_pending?: boolean }[], attempt: number): number | null {
  if (attempt >= 8 || !groups.some((g) => g.lag_pending)) return null;
  return Math.min(30_000, 5_000 * (attempt + 1));
}
