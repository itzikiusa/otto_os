// A new thread's placeholder title from its opening message (no imports:
// unit tests load it). The agent's own name for the conversation replaces it
// when the agent gives one (`title_auto`: Codex `thread/name/updated`, an ACP
// `session_info_update`, Claude Code's `ai-title`); until then this reads
// better than the first 56 characters.

const LEAD_IN =
  /^(?:(?:hey|hi|hello|ok(?:ay)?|so|please|pls)[,!.]?\s+)*(?:(?:can|could|would|will) you(?: please)?\s+|i (?:want|need|would like) you to\s+|i(?:'d| would) like (?:you )?to\s+|help me(?: to)?\s+|let'?s\s+|please\s+)?/i;

/** The first sentence or line, polite lead-ins dropped, cut at a word
 *  boundary to at most `max` characters (an ellipsis when cut), first letter
 *  capitalised. Empty text → `fallback`. */
export function placeholderTitle(text: string, fallback: string, max = 56): string {
  const firstLine = text.split('\n').map((l) => l.trim()).find((l) => l.length > 0) ?? '';
  let t = firstLine.replace(/\s+/g, ' ').replace(LEAD_IN, '').trim();
  const sentence = /^(.{12,}?[.!?])(?:\s|$)/.exec(t);
  if (sentence) t = sentence[1];
  t = t.replace(/[\s.,;:!?]+$/, '');
  if (!t) return fallback;
  if (t.length > max) {
    const cut = t.slice(0, max - 1);
    const space = cut.lastIndexOf(' ');
    t = `${(space > max * 0.5 ? cut.slice(0, space) : cut).replace(/[\s.,;:]+$/, '')}…`;
  }
  return t.charAt(0).toUpperCase() + t.slice(1);
}
