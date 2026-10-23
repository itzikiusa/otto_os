// The Insights page's empty-state copy, driven by the scheduled-run config
// (error-handling-6-03): "off" only when the config says off, and a cadence
// whose scheduled runs keep failing is named with its last error.
import type { InsightsConfig } from '../../lib/api/types';

const CADENCES = ['daily', 'weekly', 'monthly'] as const;

export interface ScheduleCopy {
  /** The sentence appended to the empty state's body. */
  text: string;
  /** Show "Turn on scheduled reports" (only when nothing is on). */
  offerTurnOn: boolean;
  /** A scheduled cadence failed: offer Run now prominently. */
  failing: boolean;
}

export function scheduleCopy(cfg: InsightsConfig | null): ScheduleCopy {
  if (!cfg) return { text: '', offerTurnOn: true, failing: false };
  const on = CADENCES.filter((c) => cfg[c]);
  if (on.length === 0) {
    return { text: 'Scheduled reports are off until you turn them on.', offerTurnOn: true, failing: false };
  }
  for (const c of on) {
    const s = cfg.schedule_status?.[c];
    if (!s) continue;
    const runs = s.attempts === 1 ? 'the last run' : `the last ${s.attempts} runs`;
    const why = s.last_error ? `: ${s.last_error}` : '.';
    return {
      text: `Scheduled ${c} reports are on, but ${runs} didn’t produce a report${why}`,
      offerTurnOn: false,
      failing: true,
    };
  }
  return { text: `Scheduled ${on.join(', ')} reports are on; the next one appears here once it runs.`, offerTurnOn: false, failing: false };
}
