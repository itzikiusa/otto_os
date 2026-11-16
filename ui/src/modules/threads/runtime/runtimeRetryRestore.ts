// Attachment hydration and draft restoration helpers for failed/interrupted turns
// and popped queued messages in RuntimeThread.svelte.
import { authedResponse } from '../../../lib/api/client';
import type { MessageAttachment, RuntimeDraftContent } from '../../../lib/api/types';
import { snapshotDraft } from '../../../lib/stores/runtimeDraftPersistence';
import { runtimeDrafts, type ComposerFile } from '../../../lib/stores/runtimeDrafts.svelte';
import { toastError } from '../../../lib/toastError';
import { hydrateRetryAttachments, mergeQueuedAttachments, mergeQueuedEdit } from './runtimeComposer';

async function fetchRuntimeSessionAttachmentBytes(sessionId: string, attachmentId: string): Promise<Uint8Array> {
  const r = await authedResponse(
    `/runtime-sessions/${encodeURIComponent(sessionId)}/attachments/${encodeURIComponent(attachmentId)}`,
  );
  return new Uint8Array(await r.arrayBuffer());
}

export async function restoreFailedTurnToDraft(opts: {
  sessionId: string;
  draft: ReturnType<typeof runtimeDrafts.for>;
  text: string;
  attachments?: readonly MessageAttachment[];
  transformText?: (text: string) => string;
  focusComposer: () => void;
}): Promise<void> {
  try {
    const hydrated = await hydrateRetryAttachments(opts.attachments, (aid) =>
      fetchRuntimeSessionAttachmentBytes(opts.sessionId, aid),
    );
    const cleanText = opts.transformText ? opts.transformText(opts.text) : opts.text;
    if (cleanText) opts.draft.text = mergeQueuedEdit(opts.draft.text, cleanText);
    if (hydrated.length > 0) opts.draft.images = mergeQueuedAttachments(opts.draft.images, hydrated);
    opts.draft.flushSave();
    opts.focusComposer();
  } catch (e) {
    toastError('Couldn’t restore message attachments', e);
  }
}

export async function buildFailedTurnResendDraft(opts: {
  sessionId: string;
  draft: ReturnType<typeof runtimeDrafts.for>;
  text: string;
  attachments?: readonly MessageAttachment[];
  transformText?: (text: string) => string;
}): Promise<{ text: string; sent: RuntimeDraftContent }> {
  const hydrated = await hydrateRetryAttachments(opts.attachments, (aid) =>
    fetchRuntimeSessionAttachmentBytes(opts.sessionId, aid),
  );
  const text = opts.transformText ? opts.transformText(opts.text) : opts.text;
  return {
    text,
    sent: { ...snapshotDraft(opts.draft), text, images: hydrated, context: [], fresh_context: false },
  };
}

export function restorePoppedQueuedMessageToDraft(opts: {
  activeSessionId: string;
  editedSessionId: string;
  text: string;
  restoredAtts: ComposerFile[];
  transformText?: (text: string) => string;
  focusComposer: () => void;
}): void {
  const targetDraft = runtimeDrafts.for(opts.editedSessionId);
  const cleanText =
    opts.editedSessionId === opts.activeSessionId && opts.transformText
      ? opts.transformText(opts.text)
      : opts.text;
  targetDraft.text = mergeQueuedEdit(targetDraft.text, cleanText);
  if (opts.restoredAtts.length > 0) targetDraft.images = mergeQueuedAttachments(targetDraft.images, opts.restoredAtts);
  targetDraft.flushSave();
  if (opts.editedSessionId === opts.activeSessionId) opts.focusComposer();
}
