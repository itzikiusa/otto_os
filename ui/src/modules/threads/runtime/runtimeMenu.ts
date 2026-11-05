// The ⋯ menu of a runtime-backed thread (sidebar row, top bar, pane header).
// PTY-only actions (archive, rooms, hand-over) don't apply; a closed thread
// lives in History, can still be renamed, and can be reopened. Delete asks
// first unless Settings › "Closing a session tab" is "Always delete" (the
// same rule as a terminal session's Delete).
import { confirmer } from '../../../lib/confirm.svelte';
import type { MenuItem } from '../../../lib/contextmenu.svelte';
import { auth } from '../../../lib/stores/auth.svelte';
import { ui } from '../../../lib/stores/ui.svelte';
import { uiControl } from '../../../lib/stores/uiControl.svelte';
import { toastError } from '../../../lib/toastError';
import { threads } from '../threadsStore.svelte';
import { phoneHandoff } from '../../../lib/stores/phoneHandoff.svelte';
import { runtimeThreads } from './runtimeThreads.svelte';

export function runtimeThreadMenu(id: string): MenuItem[] {
  return [
    { label: threads.isPinned(id) ? 'Unpin thread' : 'Pin thread', icon: 'pin', action: () => threads.togglePin(id) },
    { label: 'Open beside', icon: 'split', action: () => threads.openBeside(id) },
    { label: 'Rename…', icon: 'edit', action: () => void renameThread(id) },
    {
      label: threads.isTerminalOpen(id) ? 'Hide terminal drawer' : 'Open terminal drawer',
      icon: 'terminal',
      hint: '/terminal',
      action: () => threads.toggleTerminal(id),
    },
    ...uiControlItem(id),
    ...phoneHandoff.menuItem(`threads/${id}`, 'this thread'),
    { separator: true },
    runtimeThreads.get(id)?.status === 'closed'
      ? { label: 'Reopen thread', icon: 'refresh', action: () => void reopenThread(id) }
      : { label: 'Close thread…', icon: 'x', danger: true, action: () => void closeThread(id) },
    { label: 'Delete thread…', icon: 'trash', danger: true, action: () => void deleteThread(id) },
  ];
}

/** Reopen a closed thread (its agent resumes); `show` focuses it. */
export async function reopenThread(id: string, show = true): Promise<boolean> {
  try {
    await runtimeThreads.reopen(id);
    if (show) threads.selectThread(id);
    return true;
  } catch (e) {
    toastError('Couldn’t reopen thread', e);
    return false;
  }
}

/** Delete a thread and its history — confirmed unless the person chose
 *  "Always delete" for closing tabs (`ui.closeTabPref`). */
export async function deleteThread(id: string): Promise<void> {
  const r = runtimeThreads.get(id);
  if (ui.closeTabPref !== 'delete') {
    const name = r?.title?.trim();
    const working = r?.status === 'working' || (r?.background_tasks?.length ?? 0) > 0;
    const ok = await confirmer.ask(
      `Delete ${name ? `“${name}”` : 'this thread'} and its entire history? This cannot be undone.${working ? ' Its agent is working right now and will stop.' : ''}`,
      { title: 'Delete thread', confirmLabel: 'Delete', danger: true },
    );
    if (!ok) return;
  }
  try {
    await runtimeThreads.remove(id);
    if (threads.paneIds.includes(id)) threads.closePane(id);
  } catch (e) {
    toastError('Couldn’t delete thread', e);
  }
}

/** "Allow UI control" (checked while on): lets the thread's agent open and
 *  drive Otto modules (a DB connection, k8s, a note…) in the side pane. Only
 *  the owner may turn it on; anyone who sees it on may turn it off. */
function uiControlItem(id: string): MenuItem[] {
  const r = runtimeThreads.get(id);
  if (!r) return [];
  const on = uiControl.granted(id);
  const canGrant = r.status !== 'closed' && !auth.isImpersonating && r.created_by === auth.me?.id;
  if (!canGrant && !on) return [];
  return [
    {
      label: 'Allow UI control',
      icon: 'cursor',
      checked: on,
      disabled: uiControl.busy[id] === true,
      title: on
        ? 'UI control is on — the agent can open and drive Otto beside this thread. Click to turn it off.'
        : 'Let the agent open and drive Otto (connections, queries, k8s, notes…) beside this thread, where you can see it',
      action: () => void uiControl.setGrant(id, !on),
    },
  ];
}

async function renameThread(id: string): Promise<void> {
  const title = await confirmer.promptText('Thread title', {
    title: 'Rename thread',
    confirmLabel: 'Rename',
    initial: runtimeThreads.get(id)?.title ?? '',
  });
  if (!title) return;
  try {
    await runtimeThreads.rename(id, title);
  } catch (e) {
    toastError('Couldn’t rename thread', e);
  }
}

async function closeThread(id: string): Promise<void> {
  const title = runtimeThreads.get(id)?.title ?? 'this thread';
  const ok = await confirmer.ask(`Stop the agent and close “${title}”? It moves to History; you can reopen it later.`, {
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
