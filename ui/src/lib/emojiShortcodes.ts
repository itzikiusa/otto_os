// `:shortcode:` emoji — GitHub's alias table (./emoji/gemoji.ts), loaded on
// first use so its ~15 KB never sits in a page chunk. Until it lands every
// lookup misses: callers render the text as written and re-render once
// `loadEmojiTable()` resolves (lib/agentEmojiRender.svelte.ts).

let table: Map<string, string> | null = null;
let order: readonly EmojiSuggestion[] = [];
let loading: Promise<void> | null = null;

export interface EmojiSuggestion {
  shortcode: string;
  emoji: string;
}

export const emojiTableLoaded = (): boolean => table !== null;

export function loadEmojiTable(): Promise<void> {
  loading ??= import('./emoji/gemoji.ts').then(({ GEMOJI }) => {
    const map = new Map<string, string>();
    const list: EmojiSuggestion[] = [];
    for (const entry of GEMOJI.split('|')) {
      const [emoji, ...aliases] = entry.trim().split(' ');
      for (const shortcode of aliases) {
        map.set(shortcode, emoji);
        list.push({ shortcode, emoji });
      }
    }
    table = map;
    order = list;
  });
  return loading;
}

/** The emoji GitHub shows for `:name:` (exact, lower-case like GitHub), or null. */
export function resolveEmojiShortcode(name: string): string | null {
  return table?.get(name) ?? null;
}

// Not after a word, `/` or `\` (URLs, ports, paths, escapes), not before one.
// lib/agentEmojiRender.svelte.ts pre-checks with the same pattern.
const SHORTCODE = /(?<![\\/\w]):([a-z0-9_+-]{1,40}):(?![/\w])/g;

/** Expand known `:shortcode:`s in plain text; unknown names stay literal. */
export function replaceEmojiShortcodes(text: string): string {
  if (!table || !text.includes(':')) return text;
  return text.replace(SHORTCODE, (full, name: string) => resolveEmojiShortcode(name) ?? full);
}

/** Expand `:shortcode:`s in the TEXT of rendered (escaped) HTML — never inside
 *  a tag, `<code>` or `<pre>`. For renderers without a markdown hook (lib/md.ts). */
export function expandEmojiHtml(html: string): string {
  if (!table || !html.includes(':')) return html;
  let code = 0;
  return html.replace(/(<[^>]*>)|([^<]+)/g, (part, tag: string | undefined) => {
    if (tag) {
      if (/^<(code|pre)\b/i.test(tag)) code++;
      else if (/^<\/(code|pre)>/i.test(tag)) code = Math.max(0, code - 1);
      return part;
    }
    return code > 0 ? part : replaceEmojiShortcodes(part);
  });
}

// Shortcodes people reach for, ranked ahead of the table's Unicode order: `:roc` + Enter
// is 🚀, not 🪨 (`:rock:` comes first in the table).
const POPULAR = [
  '+1', '-1', '100', 'bug', 'clap', 'eyes', 'fire', 'heart', 'joy', 'pray', 'rocket', 'sob',
  'smile', 'sparkles', 'tada', 'thinking', 'wave', 'warning', 'white_check_mark', 'x', 'zap',
];

/** Composer completions for `:query` — popular prefix hits, then prefix hits, then substring
 *  hits; one row per emoji. */
export function searchEmojiShortcodes(query: string, limit = 8): EmojiSuggestion[] {
  const q = query.toLowerCase();
  const out: EmojiSuggestion[] = [];
  const seen = new Set<string>();
  const popular = POPULAR.flatMap((shortcode) => {
    const emoji = table?.get(shortcode);
    return emoji ? [{ shortcode, emoji }] : [];
  });
  for (const [list, pass] of [
    [popular, (s: string) => s.startsWith(q)],
    [order, (s: string) => s.startsWith(q)],
    [order, (s: string) => !s.startsWith(q) && s.includes(q)],
  ] as const) {
    for (const e of list) {
      if (out.length >= limit) return out;
      if (pass(e.shortcode) && !seen.has(e.emoji)) {
        seen.add(e.emoji);
        out.push(e);
      }
    }
  }
  return out;
}
