// The ⋯ menu of a runtime-backed thread (sidebar row, top bar, pane header).
// PTY-only actions (archive, rooms, hand-over) don't apply. The ways out, in
// one vocabulary: "Move to History" tidies the list and leaves the agent
// running; "Close" stops the agent and moves the thread to History, where
// "Reopen" brings it back; "Delete" removes it with its transcript. "Revive"
// restarts a stopped (dormant) agent. Delete asks first unless Settings ›
// "Closing a session tab" is "Always delete" (the same rule as a terminal
// session's Delete). Closing a thread with open child threads asks whether
// they close too (threadClosePlan.ts).
import { confirmer } from '../../../lib/confirm.svelte';
import type { MenuItem } from '../../../lib/contextmenu.svelte';
import { auth } from '../../../lib/stores/auth.svelte';
import { ui } from '../../../lib/stores/ui.svelte';
import { uiControl } from '../../../lib/stores/uiControl.svelte';
import { toastError } from '../../../lib/toastError';
import { closeWithChildrenDialog, openDescendants } from '../threadClosePlan';
import { threads } from '../threadsStore.svelte';
import { phoneHandoff } from '../../../lib/stores/phoneHandoff.svelte';
import { openProjectInIde } from '../ideProjects.svelte';
import { councilMenuItems } from './councilStore.svelte';
import { mcpServersSheet } from './mcpServersSheet.svelte';
import { runtimeThreads } from './runtimeThreads.svelte';

/** The toolbar ⋯ leaves out what the sidebar row keeps (threadsPageMenu.ts). */
export const MOVE_TO_HISTORY = 'Move to History';
export const MOVE_TO_ACTIVE = 'Move back to Active';

export function runtimeThreadMenu(id: string): MenuItem[] {
  const r = runtimeThreads.get(id);
  const cwd = (r?.cwd ?? '').trim().replace(/\/+$/, '');
  const status = r?.status ?? '';
  return [
    { label: threads.isPinned(id) ? 'Unpin thread' : 'Pin thread', icon: 'pin', action: () => threads.togglePin(id) },
    { label: 'Open beside', icon: 'split', action: () => threads.openBeside(id) },
    ...(cwd && cwd !== '~'
      ? [{ label: 'Open in IDE', icon: 'ide' as const, action: () => openProjectInIde(cwd, id) }]
      : []),
    { label: 'Rename…', icon: 'edit', action: () => void renameThread(id) },
    {
      label: 'Terminal drawer',
      icon: 'terminal',
      checked: threads.isTerminalOpen(id),
      hint: '/terminal',
      title: 'A shell in this thread’s folder, under the conversation',
      action: () => threads.toggleTerminal(id),
    },
    { label: 'MCP servers', icon: 'plug', title: 'What this thread’s agent can use over MCP, and why a server is missing', action: () => mcpServersSheet.open(id) },
    ...uiControlItem(id),
    ...councilMenuItems(id),
    ...phoneHandoff.menuItem(`threads/${id}`, 'this thread'),
    { separator: true },
    ...(status !== 'closed'
      ? [{
          label: threads.isSettled(id) ? MOVE_TO_ACTIVE : MOVE_TO_HISTORY,
          icon: 'check',
          title: threads.isSettled(id) ? 'Show it with the active threads again' : 'Tidy it out of the active list; its agent keeps running',
          action: () => threads.toggleSettle(id),
        }] : []),
    ...(status === 'closed'
      ? [{ label: 'Reopen thread', icon: 'refresh', title: 'Bring it back from History; its agent resumes the conversation', action: () => void reopenThread(id) }]
      : status === 'dormant'
        ? [{ label: 'Revive thread', icon: 'refresh', title: 'Restart its stopped agent', action: () => void reopenThread(id) }]
        : []),
    ...(status !== 'closed'
      ? [{ label: 'Close thread…', icon: 'x', danger: true, hint: 'to History', title: 'Stop the agent; the thread moves to History, where you can reopen it', action: () => void closeThread(id) }] : []),
    { label: 'Delete thread…', icon: 'trash', danger: true, hint: 'for good', title: 'Remove the thread and its transcript; this cannot be undone', action: () => void deleteThread(id) },
  ];
}

/** Revive a dormant thread or reopen a closed one (its agent resumes);
 *  `show` focuses it. */
export async function reopenThread(id: string, show = true): Promise<boolean> {
  const closed = runtimeThreads.get(id)?.status === 'closed';
  try {
    await runtimeThreads.reopen(id);
    if (show) threads.selectThread(id);
    return true;
  } catch (e) {
    toastError(closed ? 'Couldn’t reopen thread' : 'Couldn’t revive thread', e);
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
  const kids = openDescendants(Object.values(runtimeThreads.byId), id);
  let ids = [id];
  if (kids.length > 0) {
    const d = closeWithChildrenDialog(title, kids.length);
    const { value } = await confirmer.choose(d.message, { title: d.title, options: d.options });
    if (!value) return;
    // Children first, so none of them reports back into a closing parent.
    if (value === 'all') ids = [...kids.map((k) => k.id).reverse(), id];
  } else {
    const ok = await confirmer.ask(`Stop the agent and close “${title}”? It moves to History, where you can reopen it.`, {
      title: 'Close thread?',
      confirmLabel: 'Close thread',
      danger: true,
    });
    if (!ok) return;
  }
  for (const t of ids) {
    try {
      await runtimeThreads.close(t);
      threads.closePane(t);
    } catch (e) {
      toastError('Couldn’t close thread', e);
      return;
    }
  }
}
