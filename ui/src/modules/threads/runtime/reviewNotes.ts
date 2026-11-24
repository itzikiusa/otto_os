// Review notes on a diff (after Orca's diff comments, MIT): a comment on a
// line or a range of lines in the Changes panel, gathered per thread and sent
// to its agent as ONE message — each note with its file, old / new line
// numbers, the quoted lines and the comment. Pure (unit/runtimeReviewNotes.test.ts);
// the per-thread store is reviewNotes.svelte.ts.
import type { DiffLine, FileDiff } from '../../../lib/api/types';

export interface ReviewNote {
  id: string;
  repoId: string;
  /** The Changes target: `working`, `range:<upstream>...HEAD` or `commit:<sha>`. */
  target: string;
  path: string;
  /** Old / new line numbers the note covers ("12" or "12–14"; '' when none). */
  oldLines: string;
  newLines: string;
  /** The lines as the diff shows them (`+` / `-` / space, then the text). */
  quote: string;
  comment: string;
  state: 'draft' | 'sent' | 'resolved';
  /** The noted lines are no longer in the diff (the agent changed them). */
  outdated?: boolean;
  /** While `sent`: the composer's context chip that carries it (not yet a message). */
  chip?: string;
}

const span = (nums: (number | null)[]): string => {
  const n = nums.filter((x): x is number => x !== null);
  if (!n.length) return '';
  const lo = Math.min(...n);
  const hi = Math.max(...n);
  return lo === hi ? `${lo}` : `${lo}–${hi}`;
};

const sign = (l: DiffLine): string => (l.origin === 'add' ? '+' : l.origin === 'del' ? '-' : ' ');

/** The lines a note covers, as its quote and line spans. */
export function noteAnchor(lines: readonly DiffLine[]): Pick<ReviewNote, 'oldLines' | 'newLines' | 'quote'> {
  return {
    oldLines: span(lines.map((l) => l.old_line)),
    newLines: span(lines.map((l) => l.new_line)),
    quote: lines.map((l) => `${sign(l)}${l.content.replace(/\n$/, '')}`).join('\n'),
  };
}

/** "new 12–14 · old 10" — the note's place, in words. */
export function noteWhere(n: Pick<ReviewNote, 'oldLines' | 'newLines'>): string {
  return [n.newLines && `new ${n.newLines}`, n.oldLines && `old ${n.oldLines}`].filter(Boolean).join(' · ');
}

/** The one message the agent gets for `notes`. */
/** The header a sent review carries in the message (composerContext.ts
 *  `notes:` context): "--- Review notes on the diff: Review · 2 notes ---". */
export const REVIEW_HEADER = '--- Review notes on the diff: ';

/** One note as a sent message carries it (the inverse of {@link reviewMessage}). */
export interface SentReviewNote {
  path: string;
  where: string;
  quote: string[];
  comment: string;
}

/** A sent review in a user message: the person's own words before it and its
 *  notes — or null when the message carries none. Plain text to every agent;
 *  only the timeline reads it back (a compact "Review · N notes" block). */
export function sentReview(text: string): { prose: string; label: string; notes: SentReviewNote[] } | null {
  const at = text.indexOf(REVIEW_HEADER);
  if (at < 0) return null;
  const nl = text.indexOf('\n', at);
  const head = text.slice(at + REVIEW_HEADER.length, nl < 0 ? undefined : nl).replace(/ ---\s*$/, '');
  // The block runs to the next context section, if any.
  const rest = nl < 0 ? '' : text.slice(nl + 1);
  const next = rest.search(/\n--- [^\n]+ ---\n/);
  const body = next < 0 ? rest : rest.slice(0, next);
  const notes: SentReviewNote[] = [];
  for (const chunk of body.split(/\n\s*\n/)) {
    const lines = chunk.split('\n');
    const m = /^\d+\. (.+?) \((.*)\)$/.exec(lines[0]?.trim() ?? '');
    if (!m) continue;
    const quote = lines.filter((l) => l.startsWith('   > ')).map((l) => l.slice(5));
    const comment = lines.slice(1).filter((l) => !l.startsWith('   > ')).map((l) => l.trim()).join('\n').trim();
    notes.push({ path: m[1], where: m[2], quote, comment });
  }
  if (!notes.length) return null;
  return { prose: text.slice(0, at).trim(), label: head, notes };
}

export function reviewMessage(notes: readonly ReviewNote[]): string {
  return notes
    .map((n, i) => [`${i + 1}. ${n.path} (${noteWhere(n) || 'file'})`, ...n.quote.split('\n').map((l) => `   > ${l}`), `   ${n.comment.trim()}`].join('\n'))
    .join('\n\n');
}

/** The note's lines are gone from this diff of its file: the agent (or you)
 *  changed them since. A note whose lines are still there is current. */
export function isOutdated(n: Pick<ReviewNote, 'quote'>, file: Pick<FileDiff, 'hunks'>): boolean {
  const want = n.quote.split('\n');
  for (const h of file.hunks) {
    const have = h.lines.map((l) => `${sign(l)}${l.content.replace(/\n$/, '')}`);
    for (let i = 0; i + want.length <= have.length; i++) {
      if (want.every((w, k) => have[i + k] === w)) return false;
    }
  }
  return true;
}

/** Whether line `ln` is the last line a note covers (it shows under it). */
export function endsAt(n: Pick<ReviewNote, 'oldLines' | 'newLines'>, ln: Pick<DiffLine, 'old_line' | 'new_line'>): boolean {
  const last = (s: string): number | null => (s ? Number(s.split('–').at(-1)) : null);
  const nl = last(n.newLines);
  if (nl !== null) return ln.new_line === nl;
  const ol = last(n.oldLines);
  return ol !== null && ln.old_line === ol && ln.new_line === null;
}
