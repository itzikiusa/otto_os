// Quote part of an answer into a reply (T3 Code #33124, #34181).

/** `text` as a markdown block quote: each line `> …`, blank lines `>`. */
export function markdownQuote(text: string): string {
  const t = text.replace(/\r\n?/g, '\n').trim();
  if (!t) return '';
  return t
    .split('\n')
    .map((l) => (l.trim() ? `> ${l}` : '>'))
    .join('\n');
}

/** `draft` with the quote of `text` appended a blank line apart, the caret
 *  meant to land after it. An empty selection leaves the draft as it is. */
export function withQuote(draft: string, text: string): string {
  const q = markdownQuote(text);
  if (!q) return draft;
  const head = draft.replace(/\s+$/, '');
  return `${head ? `${head}\n\n` : ''}${q}\n\n`;
}
