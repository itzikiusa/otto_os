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

export interface QuotePlacement {
  top: number;
  inlineStart: number;
}

/** Place the fixed "Quote" pill above a selection's physical viewport rect,
 *  clamped into the viewport. Converts physical `left` into logical
 *  `inlineStart` (`inset-inline-start`) so RTL layouts do not mirror the pill
 *  away from the selected text. */
export function placeSelectionQuote(
  rect: { top: number; left: number; width: number },
  viewport: { width: number; height: number },
  rtl: boolean,
  pillWidth = 84,
): QuotePlacement {
  const top = Math.min(Math.max(8, rect.top - 34), Math.max(8, viewport.height - 40));
  const left = Math.min(
    Math.max(8, rect.left + rect.width / 2 - pillWidth / 2),
    Math.max(8, viewport.width - pillWidth - 8),
  );
  const inlineStart = rtl ? Math.max(8, viewport.width - (left + pillWidth)) : left;
  return { top, inlineStart };
}
