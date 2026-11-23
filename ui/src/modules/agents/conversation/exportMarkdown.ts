// Export a coding-agent conversation transcript as clean, paste-ready Markdown.

import { copyWithToast } from '../../../lib/clipboard';
import { fetchTranscript } from '../../../lib/stores/transcript.svelte';
import { toasts } from '../../../lib/toast.svelte';
import { toastError } from '../../../lib/toastError';
import {
  formatTranscriptMarkdown,
  stripAnsi,
  type ExportTranscriptOptions,
} from './exportMarkdownFormat';

export { formatTranscriptMarkdown, stripAnsi, type ExportTranscriptOptions };

/** Copy already-loaded turns as Markdown (used from ConversationView). */
export async function copyLoadedTranscriptMarkdown(opts: ExportTranscriptOptions): Promise<boolean> {
  const md = formatTranscriptMarkdown(opts);
  if (!md) {
    toasts.info('Nothing to copy yet', 'This session has no transcript turns yet.');
    return false;
  }
  return copyWithToast(md, 'transcript (Markdown)');
}

/** Fetch a session's transcript and copy it as clean Markdown (used from SessionView). */
export async function copySessionTranscriptMarkdown(
  sessionId: string,
  fallback?: { title?: string | null; provider?: string | null; cwd?: string | null },
): Promise<boolean> {
  try {
    const t = await fetchTranscript({ sessionId }, { limit: 200 });
    if (t.unavailable_reason || t.turns.length === 0) {
      toasts.info('No transcript available', 'This session has no readable transcript turns yet.');
      return false;
    }
    return await copyLoadedTranscriptMarkdown({
      title: t.title ?? fallback?.title,
      provider: t.provider ?? fallback?.provider,
      model: t.model,
      cwd: t.cwd ?? fallback?.cwd,
      turns: t.turns,
    });
  } catch (e) {
    toastError('Couldn’t copy the transcript', e);
    return false;
  }
}
