// Composer emoji, Slack / Claude Code style (lib/components/EmojiAssist.svelte
// drives it from a textarea's keys):
//   • an emoticon (`:)`, `<3`, …) becomes its emoji when a SPACE is typed after
//     it; Backspace then deletes the space, and a Backspace on the fresh emoji
//     restores the typed text, which stays until another space follows it;
//   • `:` + 2 letters opens the `:shortcode` completion list.
// Never inside inline code or a code fence, never glued to a word or a URL.

/** The small, common set Slack converts (its own picks for each). */
export const EMOTICONS: Readonly<Record<string, string>> = {
  ':)': '🙂',
  ':-)': '🙂',
  ':(': '😞',
  ':-(': '😞',
  ';)': '😉',
  ';-)': '😉',
  ':D': '😄',
  ':-D': '😄',
  ':P': '😛',
  ':-P': '😛',
  ':p': '😛',
  ':o': '😮',
  ':O': '😮',
  ":'(": '😢',
  ':|': '😐',
  '<3': '❤️',
};
const BY_LENGTH = Object.keys(EMOTICONS).sort((a, b) => b.length - a.length);

/** Is `pos` inside a code fence or an inline code span (an odd number of
 *  backticks earlier on its line)? */
export function inCode(text: string, pos: number): boolean {
  const before = text.slice(0, pos);
  const lines = before.split('\n');
  const current = lines.pop() ?? '';
  let fence = false;
  for (const l of lines) if (/^\s*(```|~~~)/.test(l)) fence = !fence;
  if (fence || /^\s*(```|~~~)/.test(current)) return true;
  return (current.match(/`/g)?.length ?? 0) % 2 === 1;
}

/** The emoticon ending at `caret`, standing on its own (line start or after
 *  whitespace) and outside code. */
export function emoticonBefore(text: string, caret: number): { start: number; original: string; emoji: string } | null {
  for (const original of BY_LENGTH) {
    const start = caret - original.length;
    if (start < 0 || text.slice(start, caret) !== original) continue;
    if (start > 0 && !/\s/.test(text[start - 1])) return null;
    return inCode(text, start) ? null : { start, original, emoji: EMOTICONS[original] };
  }
  return null;
}

/** Replace `[from, to)` with `insert`. */
export interface EmojiEdit {
  from: number;
  to: number;
  insert: string;
}

export const applyEmojiEdit = (text: string, e: EmojiEdit): string => text.slice(0, e.from) + e.insert + text.slice(e.to);

const MODIFIER_KEYS = new Set(['Shift', 'Meta', 'Alt', 'Control', 'CapsLock', 'Fn']);

/** Emoticon conversion: feed it every keydown; an edit means "do this instead
 *  of the key's default". */
export class EmoticonConverter {
  /** The last conversion, while a Backspace may still undo it. */
  private last: { at: number; emoji: string; original: string } | null = null;

  key(key: string, text: string, selStart: number, selEnd: number): EmojiEdit | null {
    const collapsed = selStart === selEnd;
    if (key === ' ') {
      this.last = null;
      const m = collapsed ? emoticonBefore(text, selStart) : null;
      if (!m) return null;
      this.last = { at: m.start, emoji: m.emoji, original: m.original };
      return { from: m.start, to: selStart, insert: `${m.emoji} ` };
    }
    if (key === 'Backspace' && this.last && collapsed) {
      const { at, emoji, original } = this.last;
      const end = at + emoji.length;
      if (selStart === end && text.slice(at, end) === emoji) {
        this.last = null;
        return { from: at, to: end, insert: original };
      }
      // The first Backspace deletes the space after the emoji as usual.
      if (selStart === end + 1 && text.slice(at, end + 1) === `${emoji} `) return null;
    }
    if (!MODIFIER_KEYS.has(key)) this.last = null;
    return null;
  }
}

/** The `:query` being typed at `caret` (`:` at a word start + 2 or more
 *  name characters), outside code. */
export function shortcodeQuery(text: string, caret: number): { start: number; query: string } | null {
  const m = /(?:^|[\s(]):([a-z0-9_+-]{2,40})$/i.exec(text.slice(Math.max(0, caret - 48), caret));
  if (!m) return null;
  const start = caret - m[1].length - 1;
  return inCode(text, start) ? null : { start, query: m[1].toLowerCase() };
}
