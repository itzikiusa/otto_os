// Svelte context shared down the conversation tree (set by ConversationView).
import type { Conversation } from '../../../lib/stores/transcript.svelte';
import type { TranscriptProvider } from '../../../lib/api/types';

export interface ConvContext {
  conv: Conversation;
  /** Null in `transcriptPath` (History on-disk) mode — no images / file opens. */
  sessionId: string | null;
  readonly: boolean;
  provider: TranscriptProvider;
  /** Texts of `enqueue` items no later `dequeue`/`remove` cancelled. */
  queuedLive: string[];
}

export const CONV_CTX = 'conv';
