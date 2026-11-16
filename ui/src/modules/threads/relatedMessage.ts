// "Message…" from the Related threads popover (and a sidebar row's "Send
// message from active thread…", a council answer's "Send to a thread"): ask
// for the text in Otto's own prompt sheet, then relay it (threadRelay.ts: a
// runtime thread queues it as "From <thread>", a suspended terminal thread
// asks to be revived first, `'broadcast'` reaches every open thread of the
// sender's project).
import { confirmer } from '../../lib/confirm.svelte';
import { ws } from '../../lib/stores/workspace.svelte';
import { anyThread, broadcastTargets, relayToThread } from './threadRelay';

/** `target` is a thread id or `'broadcast'` (every open thread in the
 *  sender's project); `initial` prefills the text (a quoted answer). */
export async function messageThread(fromId: string | null, target: string, initial = ''): Promise<boolean> {
  const from = anyThread(fromId);
  const all = target === 'broadcast';
  const n = all ? broadcastTargets(fromId).length : 0;
  const to = all ? `every open thread in this project (${n})` : `“${anyThread(target)?.title ?? 'the thread'}”`;
  const text = await confirmer.promptText(`Send a message from “${from?.title ?? 'this thread'}” to ${to}.`, {
    title: all ? 'Message all open threads in this project' : 'Message thread',
    confirmLabel: all ? `Send to ${n}` : 'Send',
    initial,
    placeholder: 'Hand off context, ask it to check something…',
    validate: (v) => (v.trim() ? null : 'Write a message first'),
  });
  if (!text?.trim()) return false;
  return relayToThread({ fromSessionId: fromId, target, text, workspaceId: from?.workspace_id ?? ws.currentId });
}
