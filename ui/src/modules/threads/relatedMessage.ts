// "Message…" from the Related threads popover (and a sidebar row's "Send
// message from active thread…"): ask for the text in Otto's own prompt
// sheet, then relay it (threadRelay.ts: a runtime thread queues it as
// "From <thread>", a suspended terminal thread asks to be revived first).
import { confirmer } from '../../lib/confirm.svelte';
import { ws } from '../../lib/stores/workspace.svelte';
import { anyThread, relayToThread } from './threadRelay';

/** `target` is a thread id or `'broadcast'` (every active thread). */
export async function messageThread(fromId: string | null, target: string): Promise<boolean> {
  const from = anyThread(fromId);
  const to = target === 'broadcast' ? 'every active thread' : `“${anyThread(target)?.title ?? 'the thread'}”`;
  const text = await confirmer.promptText(`Send a message from “${from?.title ?? 'this thread'}” to ${to}.`, {
    title: target === 'broadcast' ? 'Message all threads' : 'Message thread',
    confirmLabel: 'Send',
    placeholder: 'Hand off context, ask it to check something…',
    validate: (v) => (v.trim() ? null : 'Write a message first'),
  });
  if (!text?.trim()) return false;
  return relayToThread({ fromSessionId: fromId, target, text, workspaceId: from?.workspace_id ?? ws.currentId });
}
