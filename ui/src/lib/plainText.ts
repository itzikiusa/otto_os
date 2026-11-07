// Markdown → plain text for one-line titles (run summaries, report titles).

/** Markdown as one plain line (run summaries are the report's first lines). */
export function plainText(md: string): string {
  return md
    .replace(/```[\s\S]*?```/g, ' ')
    .replace(/!?\[([^\]]*)\]\([^)]*\)/g, '$1')
    .replace(/^\s{0,3}(#{1,6}|>|[-*+]|\d+\.)\s+/gm, '')
    .replace(/[*_`~]+/g, '')
    .replace(/\s+/g, ' ')
    .trim();
}

/** A report's title: its first non-empty line as plain text (a summary is
 *  the report's opening — heading + body — so plainText of all of it glued
 *  the heading onto the first sentence). */
export function firstLine(md: string | null | undefined): string {
  const line = (md ?? '').split('\n').find((l) => plainText(l) !== '') ?? '';
  return plainText(line);
}

/** What a run found: its first line that is not a heading (a report opens
 *  "# Night Brief — Sat 7 Nov", then the finding) — the chat's result card
 *  says the same. Falls back to the first line. */
export function leadLine(md: string | null | undefined): string {
  const lines = (md ?? '').split('\n');
  const lead = lines.find((l) => plainText(l) !== '' && !/^\s{0,3}#{1,6}\s/.test(l));
  return lead !== undefined ? plainText(lead) : firstLine(md);
}
