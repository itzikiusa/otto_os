// Svelte context shared down the conversation tree (set by ConversationView).
import type { Conversation } from '../../../lib/stores/transcript.svelte';
import type { DiffResp, TranscriptProvider } from '../../../lib/api/types';

/** What the side panel (PreviewPanel.svelte) shows. */
export type PreviewReq =
  | {
      kind: 'file';
      /** Absolute, or relative to the session cwd. */
      path: string;
      line?: number | null;
      /** Content known from the transcript (a Write's input) — shown without a read. */
      content?: string | null;
      /** This turn's change to the file, for the Diff tab. */
      diff?: DiffResp | null;
    }
  | { kind: 'code'; title: string; text: string; lang: string | null; file?: string | null }
  | { kind: 'diff'; title: string; diff: DiffResp; focus?: string | null };

export interface ConvContext {
  conv: Conversation;
  /** Null in `transcriptPath` (History on-disk) mode — no images / file opens. */
  sessionId: string | null;
  readonly: boolean;
  provider: TranscriptProvider;
  /** Texts of `enqueue` items no later `dequeue`/`remove` cancelled. */
  queuedLive: string[];
  // The rest is the conversation view's (rev 6); other hosts of these pieces
  // (the assistant's ChatView) may leave it out — every use tolerates that.
  /** The session's working directory (relative file references resolve here). */
  cwd?: string | null;
  /** "Show all work": every settled response's fold starts open. */
  expandAll?: boolean;
  /** A render item whose fold must open (search / find-in-page reveal). */
  revealId?: string | null;
  /** Open the side panel (file preview, code, diff). */
  openPreview?: (req: PreviewReq) => void;
  /** Open an external link (system browser; ⌥ → Otto's browser). */
  openUrl?: (url: string, inApp?: boolean) => void;
  /** `#123` → the session repo's pull request / issue URL, when known. */
  issueUrl?: (n: number) => string | null;
  /** Put text into the composer (reuse a prompt). Null when read-only. */
  reusePrompt?: ((text: string) => void) | null;
}

export const CONV_CTX = 'conv';
