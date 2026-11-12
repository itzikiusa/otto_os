// Banners for the top of the Usage page when usage is not being recorded:
// the transcript tailer retrying / stopped, the usage database down
// (`outage_since`) and the ingestion journal filling (`journal_fill`). Pure
// (type-only import), so `node --test` can load it.
import type { UsageStatus } from '../../lib/api/usage.svelte';

/** A journal fuller than this gets its own line (the bell warns too). */
export const JOURNAL_WARN_FILL = 0.5;

export interface UsageHealthLine {
  key: 'tailer' | 'outage' | 'journal';
  text: string;
}

type HealthFields = Pick<UsageStatus, 'tailer' | 'outage_since' | 'journal_fill'>;

/** "14:05" today, "12 Nov 14:05" on another day; the raw value if unparsable. */
export function outageTime(iso: string, now: Date = new Date()): string {
  const t = new Date(iso);
  if (Number.isNaN(t.getTime())) return iso;
  const time = t.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
  return t.toDateString() === now.toDateString()
    ? time
    : `${t.toLocaleDateString([], { day: 'numeric', month: 'short' })} ${time}`;
}

/** What is wrong, one line each (empty = healthy). */
export function usageHealthLines(s: HealthFields | null | undefined, now: Date = new Date()): UsageHealthLine[] {
  if (!s) return [];
  const out: UsageHealthLine[] = [];
  if (s.outage_since) {
    out.push({
      key: 'outage',
      text: `Usage has not been recorded since ${outageTime(s.outage_since, now)}: the usage database is down. Otto is retrying in the background.`,
    });
  }
  if (s.tailer && s.tailer.state !== 'running') {
    const cause = s.tailer.cause ?? 'the usage journal is unavailable';
    out.push({
      key: 'tailer',
      text: `Usage isn’t being recorded${s.tailer.state === 'retrying' ? ' — Otto is retrying' : ''}: ${cause}${s.tailer.state === 'stopped' ? '. Restart Otto after fixing it.' : '.'}`,
    });
  }
  if (typeof s.journal_fill === 'number' && s.journal_fill > JOURNAL_WARN_FILL) {
    const pct = Math.round(s.journal_fill * 100);
    out.push({
      key: 'journal',
      text: `The usage journal is ${pct}% full. At 100% Otto stops reading new transcripts, and usage the agent CLIs prune meanwhile is lost.`,
    });
  }
  return out;
}
