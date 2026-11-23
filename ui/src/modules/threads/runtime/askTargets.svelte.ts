// The agents a thread's composer will ask next (chips above its text), per
// thread, and the send: one message to 1–4 new threads linked to this one
// (POST /runtime-sessions/{id}/children). A full group, a wider access mode
// or a refused provider answers 400 / 409 and shows in the composer's error
// line (designed, with what to do), never as a toast.
import { ApiError } from '../../../lib/api/client';
import type { AskChildTarget, RuntimeAccessMode } from '../../../lib/api/types';
import type { RuntimeComposerDraft } from '../../../lib/stores/runtimeDrafts.svelte';
import { toastError } from '../../../lib/toastError';
import { addTarget } from './askAgents';
import { runtimeThreads } from './runtimeThreads.svelte';

class AskTargets {
  byThread = $state<Record<string, AskChildTarget[]>>({});
  /** "Answer to me only" (off by default: the answer goes to the thread's agent). */
  toMeOnly = $state<Record<string, boolean>>({});

  for(id: string): AskChildTarget[] {
    return this.byThread[id] ?? [];
  }

  add(id: string, t: AskChildTarget): void {
    this.byThread = { ...this.byThread, [id]: addTarget(this.for(id), t) };
  }

  remove(id: string, i: number): void {
    this.byThread = { ...this.byThread, [id]: this.for(id).filter((_, j) => j !== i) };
  }

  clear(id: string): void {
    const { [id]: _gone, ...rest } = this.byThread;
    this.byThread = rest;
    const { [id]: _off, ...keep } = this.toMeOnly;
    this.toMeOnly = keep;
  }
}

export const askTargets = new AskTargets();

/** Send the composer's message to the picked agents instead of this
 *  thread's own agent. The draft clears only once they started. */
export async function askFromComposer(parentId: string, draft: RuntimeComposerDraft, text: string, access: RuntimeAccessMode | null): Promise<void> {
  const targets = askTargets.for(parentId);
  if (targets.length === 0 || draft.sending) return;
  const nonImages = draft.images.filter((f) => !f.mime.startsWith('image/'));
  if (nonImages.length > 0) {
    draft.startError =
      'PDF attachments can only be sent directly to this thread — remove the PDF or clear the @agent chips first.';
    return;
  }
  const pics = draft.images.filter((f) => f.mime.startsWith('image/'));
  draft.sending = true;
  try {
    await runtimeThreads.askChildren(parentId, {
      prompt: text,
      targets,
      ...(askTargets.toMeOnly[parentId] ? { to_me_only: true } : {}),
      // Omitted = this thread's own mode; a wider pick is refused (400) in words.
      ...(access ? { access_mode: access } : {}),
      ...(pics.length
        ? { images: pics.map(({ name, mime, data_base64 }) => ({ ...(name ? { name } : {}), mime, data_base64 })) }
        : {}),
    });
    draft.text = '';
    draft.images = [];
    draft.context = [];
    draft.startError = null;
    draft.flushSave();
    askTargets.clear(parentId);
  } catch (e) {
    if (e instanceof ApiError && (e.status === 400 || e.status === 409)) draft.startError = e.message;
    else toastError('Couldn’t delegate', e);
  } finally {
    draft.sending = false;
  }
}
