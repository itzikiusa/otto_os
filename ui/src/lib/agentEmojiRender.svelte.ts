// Whether agent text renders its `:shortcode:`s as emoji right now
// (lib/stores/agentEmoji.svelte.ts). The alias table (and the expander) load
// on the first text that needs them; readers re-render when they land. Only
// agent output and GitHub PR markdown ask — never the composer or the
// person's own messages.
import { agentEmoji } from './stores/agentEmoji.svelte';

type EmojiLib = typeof import('./emojiShortcodes');
let lib = $state.raw<EmojiLib | null>(null);
let loading = false;

// The same shape lib/emojiShortcodes.ts expands (a cheap pre-check).
const MAYBE = /(?<![\\/\w]):[a-z0-9_+-]{1,40}:(?![/\w])/;

export function agentEmojiFor(text: string): boolean {
  if (!agentEmoji.on || !text.includes(':') || !MAYBE.test(text)) return false;
  if (!lib && !loading) {
    loading = true;
    void import('./emojiShortcodes').then(async (m) => {
      await m.loadEmojiTable();
      lib = m;
    });
  }
  return lib !== null;
}

/** `html` (escaped markdown output of `md`) with its shortcodes expanded when on. */
export const agentEmojiHtml = (md: string, html: string): string => (agentEmojiFor(md) && lib ? lib.expandEmojiHtml(html) : html);
