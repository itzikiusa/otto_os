// Conversation view — the agent session rendered as a Claude/Codex-app-style
// chat rebuilt from the provider transcript (docs/design/conversation-view.md §5.2).
export { default as ConversationView } from './ConversationView.svelte';
export type { ConversationViewProps } from './ConversationView.svelte';
// The turn renderer, for hosts that show a session's chat without owning the
// session (session rooms read the transcript with their room capability and
// set `ConvContext.imageUrl`).
export { default as TurnItem } from './TurnItem.svelte';
export { CONV_CTX, type ConvContext } from './context';
export { groupTurns, type RenderItem } from './format';
