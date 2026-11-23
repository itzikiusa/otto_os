// ↑ / ↓ in the composer walk the prompts the person sent in this thread
// (T3 Code's composerPromptHistory.ts, MIT; Orca's
// native-chat-sent-prompt-history.ts): ↑ in an EMPTY composer recalls the
// last one, ↑ again older, ↓ newer and past the newest back to the empty
// draft. Any edit leaves history (the text no longer matches what was
// recalled). A multi-line recall moves the caret between its lines before it
// moves on (↑ only from the first line, ↓ only from the last). Text only:
// attachments and context are never re-attached. The source is the thread's
// own user messages (no storage). Pure (unit/runtimePromptHistory.test.ts).
import type { TurnItem } from '../../../lib/api/types';
import { parseUserMessageWithBadges } from './mentionMatcher.ts';
import { messageAttribution } from './timeline.ts';

export interface HistoryState {
  /** Position in `prompts` (0 = newest) while browsing, else null. */
  at: number | null;
  /** The text last recalled (an edit ends browsing). */
  shown: string;
}

/** What the person typed in each of their messages, newest first: framed
 *  messages from other threads left out, context envelopes and chips cut,
 *  a repeat right after itself once. */
export function sentPrompts(items: readonly TurnItem[]): string[] {
  const out: string[] = [];
  for (let i = items.length - 1; i >= 0; i--) {
    const it = items[i];
    if (it.type !== 'user_message' || it.parent_item_id || messageAttribution(it.text)) continue;
    const parsed = parseUserMessageWithBadges(it.text);
    const text = (parsed.badges.length ? parsed.prose : it.text).split(/\n\n--- [^\n]* ---\n/)[0].trim();
    if (text && out[out.length - 1] !== text) out.push(text);
  }
  return out;
}

/** The text ↑ / ↓ puts in the composer, or null when the key is not history's. */
export function recallPrompt(
  e: { key: string; shiftKey: boolean; metaKey: boolean; ctrlKey: boolean; altKey: boolean; isComposing?: boolean },
  box: { value: string; selectionStart: number; selectionEnd: number },
  prompts: readonly string[],
  st: HistoryState,
): string | null {
  if ((e.key !== 'ArrowUp' && e.key !== 'ArrowDown') || e.shiftKey || e.metaKey || e.ctrlKey || e.altKey || e.isComposing) return null;
  if (st.at !== null && box.value !== st.shown) st.at = null;
  const up = e.key === 'ArrowUp';
  if (st.at === null) {
    if (!up || box.value !== '' || prompts.length === 0) return null;
  } else {
    if (box.selectionStart !== box.selectionEnd) return null;
    const v = box.value;
    if (up ? v.lastIndexOf('\n', box.selectionStart - 1) >= 0 : v.indexOf('\n', box.selectionEnd) >= 0) return null;
  }
  const next = st.at === null ? 0 : st.at + (up ? 1 : -1);
  if (next >= prompts.length) return null; // the oldest: stay
  if (next < 0) {
    st.at = null;
    st.shown = '';
    return '';
  }
  st.at = next;
  st.shown = prompts[next];
  return st.shown;
}
