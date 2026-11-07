// The one per-thread action menu — shared by a split pane's ⋯ and the page's
// top-bar ⋯ (single view has no pane header). Everything goes through the
// threads store so either caller drives the same pane state.
import { canSwitchSessionSurface, openSessionSurface } from '../../lib/sessionSurface';
import type { Session } from '../../lib/api/types';
import { confirmer } from '../../lib/confirm.svelte';
import type { MenuItem } from '../../lib/contextmenu.svelte';
import { transcript } from '../../lib/stores/transcript.svelte';
import { ws } from '../../lib/stores/workspace.svelte';
import { toastError } from '../../lib/toastError';
import { delegatedParentId, isPlainShellSession } from './threadsHelpers';
import { threads } from './threadsStore.svelte';
import { phoneHandoff } from '../../lib/stores/phoneHandoff.svelte';

export async function promptRenameThread(s: Session): Promise<void> {
  const next = await confirmer.promptText('Enter a title for this thread.', {
    title: 'Rename thread',
    confirmLabel: 'Rename',
    initial: s.title,
    placeholder: 'Thread title',
  });
  if (!next || next === s.title) return;
  try {
    await ws.renameSession(s.id, next);
  } catch (e) {
    toastError('Couldn’t rename thread', e);
  }
}

/** Resume the same legacy agent session; never restart an already-live runner. */
export async function reviveLegacyThread(s: Session, show = true): Promise<boolean> {
  try {
    if (s.archived) await ws.unarchiveSession(s.id);
    if (!isPlainShellSession(s)) await ws.resumeSession(s.id);
    threads.unsettle(s.id);
    if (show) threads.selectThread(s.id);
    return true;
  } catch (e) {
    toastError('Couldn’t revive thread', e);
    return false;
  }
}

export function legacyReviveItem(s: Session): MenuItem[] {
  return !isPlainShellSession(s) && ['exited', 'reconnectable'].includes(ws.statusMap[s.id] ?? s.status)
    ? [{ label: 'Revive thread', icon: 'refresh', action: () => void reviveLegacyThread(s) }] : [];
}

export function buildThreadMenu(s: Session, opts: { onstartroom: (id: string) => void }): MenuItem[] {
  const id = s.id;
  const parentId = delegatedParentId(s);
  const shell = isPlainShellSession(s);
  return [
    ...(ws.myRole !== 'viewer' && canSwitchSessionSurface(s) ? [{ label: 'Open in Terminal', icon: 'terminal', title: 'Continue with the same agent in Agents', action: () => void openSessionSurface(s, 'agents') }] : []),
    { label: 'Rename thread…', icon: 'edit', action: () => void promptRenameThread(s) },
    { label: threads.isPinned(id) ? 'Unpin thread' : 'Pin thread', icon: 'star', action: () => threads.togglePin(id) },
    ...(!shell
      ? [
          { label: 'Search conversation…', icon: 'search', hint: '⌘F', action: () => threads.requestSearch(id) },
          {
            label: 'Show system notes',
            icon: transcript.showSystem ? 'eye' : 'eyeOff',
            checked: transcript.showSystem,
            action: () => transcript.setShowSystem(!transcript.showSystem),
          },
          {
            label: threads.isTerminalOpen(id) ? 'Hide terminal drawer' : 'Open terminal drawer',
            icon: 'terminal',
            action: () => threads.toggleTerminal(id),
          },
        ]
      : []),
    ...(!shell ? [{ label: 'Side chat…', icon: 'comment', hint: '/btw', action: () => threads.openSideChat(id) }] : []),
    { label: 'Send message to peer…', icon: 'send', action: () => threads.togglePeerBar(id) },
    { label: 'Session room…', icon: 'people', action: () => opts.onstartroom(id) },
    ...phoneHandoff.menuItem(`threads/${id}`, 'this thread'),
    ...(parentId
      ? [{ label: 'Open with parent in Split', icon: 'split', action: () => threads.openPairInSplit(parentId, id) }]
      : []),
    ...legacyReviveItem(s),
    { separator: true },
    {
      label: threads.isSettled(id) ? 'Move back to Active' : 'Settle to History',
      icon: 'check',
      action: () => threads.toggleSettle(id),
    },
    {
      label: 'Archive',
      icon: 'archive',
      action: () => {
        void ws.requestArchive(id).then((ok) => {
          if (ok) threads.closePane(id);
        });
      },
    },
  ];
}
