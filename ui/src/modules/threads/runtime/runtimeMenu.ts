// The ⋯ menu of a runtime-backed thread (sidebar row, top bar, pane header).
// PTY-only actions (rename, archive, rooms, terminal, hand-over) don't apply.
import { confirmer } from '../../../lib/confirm.svelte';
import type { MenuItem } from '../../../lib/contextmenu.svelte';
import { toastError } from '../../../lib/toastError';
import { threads } from '../threadsStore.svelte';
import { runtimeThreads } from './runtimeThreads.svelte';

export function runtimeThreadMenu(id: string): MenuItem[] {
  return [
    { label: threads.isPinned(id) ? 'Unpin thread' : 'Pin thread', icon: 'pin', action: () => threads.togglePin(id) },
    { label: 'Open beside', icon: 'split', action: () => threads.openBeside(id) },
    { separator: true },
    {
      label: 'Close thread…',
      icon: 'x',
      danger: true,
      action: () => void closeThread(id),
    },
  ];
}

async function closeThread(id: string): Promise<void> {
  const title = runtimeThreads.get(id)?.title ?? 'this thread';
  const ok = await confirmer.ask(`Stop the agent and close “${title}”? It can’t take new messages afterwards.`, {
    title: 'Close thread?',
    confirmLabel: 'Close thread',
    danger: true,
  });
  if (!ok) return;
  try {
    await runtimeThreads.close(id);
    threads.closePane(id);
  } catch (e) {
    toastError('Couldn’t close thread', e);
  }
}
