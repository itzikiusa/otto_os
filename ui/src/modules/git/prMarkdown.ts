// PR descriptions and comments: lib/md.ts's sanitized GFM renderer
// (`renderMarkdownGfm` — tables, task lists, `_italic_`, links, code blocks),
// with GitHub `:shortcode:`s shown as emoji like github.com does (unless "Show
// emoji in agent replies" is off — lib/stores/agentEmoji.svelte.ts).
import { renderMarkdownGfm } from '../../lib/md';
import { agentEmojiHtml } from '../../lib/agentEmojiRender.svelte';

export const renderPrMarkdown = (md: string): string => agentEmojiHtml(md, renderMarkdownGfm(md));
