// PR descriptions and comments: lib/md.ts's escaped `renderMarkdown`, with
// GitHub `:shortcode:`s shown as emoji like github.com does (unless "Show
// emoji in agent replies" is off — lib/stores/agentEmoji.svelte.ts).
import { renderMarkdown } from '../../lib/md';
import { agentEmojiHtml } from '../../lib/agentEmojiRender.svelte';

export const renderPrMarkdown = (md: string): string => agentEmojiHtml(md, renderMarkdown(md));
