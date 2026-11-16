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
import { openProjectInIde } from './ideProjects.svelte';
import { messageThread } from './relatedMessage';
import { relatedPopover } from './relatedPopover.svelte';
import { RELATED_LABEL } from './threadsPageMenu';

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

/** ⋯ › Related threads…: linked threads, workers, side chat and rooms, with
 *  Message and Start a room (the old "Send message to peer…" and "Session
 *  room…" items) — the header chip's popover. */
export function relatedThreadsItem(id: string): MenuItem {
  return { label: RELATED_LABEL, icon: 'people', action: () => relatedPopover.open(id) };
}

export function buildThreadMenu(s: Session): MenuItem[] {
  const id = s.id;
  const parentId = delegatedParentId(s);
  const shell = isPlainShellSession(s);
  const cwd = (s.cwd ?? '').trim().replace(/\/+$/, '');
  return [
    ...(cwd && cwd !== '~'
      ? [{ label: 'Open in IDE', icon: 'ide' as const, action: () => openProjectInIde(cwd, id) }]
      : []),
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
    relatedThreadsItem(id),
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

export function buildSidebarLegacyThreadMenu(
  s: Session,
  opts: {
    onreopen: (s: Session, beside: boolean) => void;
    onstartroom: (sessionId: string) => void;
    onselect?: () => void;
  },
): MenuItem[] {
  const pinned = threads.isPinned(s.id);
  const settled = threads.isSettled(s.id);
  const cwd = (s.cwd ?? '').trim().replace(/\/+$/, '');
  const canSendFromFocused =
    Boolean(threads.focusedSessionId && threads.focusedSessionId !== s.id && !s.archived);
  return [
    {
      label: pinned ? 'Unpin thread' : 'Pin thread',
      icon: 'star',
      action: () => threads.togglePin(s.id),
    },
    {
      label: 'Open beside in Split',
      icon: 'split',
      action: () => {
        if (s.archived) opts.onreopen(s, true);
        else {
          threads.openBeside(s.id);
          opts.onselect?.();
        }
      },
    },
    ...(cwd && cwd !== '~'
      ? [{ label: 'Open in IDE', icon: 'ide' as const, action: () => openProjectInIde(cwd, s.id) }]
      : []),
    ...(canSendFromFocused
      ? [
          {
            label: 'Send message from active thread…',
            icon: 'send' as const,
            action: () => void messageThread(threads.focusedSessionId, s.id),
          },
        ]
      : []),
    ...(!s.archived ? legacyReviveItem(s) : []),
    ...(!s.archived
      ? [
          {
            label: 'Open Room…',
            icon: 'people' as const,
            action: () => opts.onstartroom(s.id),
          },
          {
            label: settled ? 'Move back to Active' : 'Settle to History',
            icon: 'check' as const,
            action: () => threads.toggleSettle(s.id),
          },
        ]
      : [
          {
            label: 'Revive thread',
            icon: 'refresh' as const,
            action: () => opts.onreopen(s, false),
          },
        ]),
    { separator: true },
    {
      label: 'Rename…',
      icon: 'edit',
      action: () => void promptRenameThread(s),
    },
    ...(!s.archived
      ? [
          {
            label: 'Archive',
            icon: 'archive' as const,
            action: () => {
              void ws.requestArchive(s.id).then((ok) => {
                if (ok && threads.paneIds.includes(s.id)) threads.closePane(s.id);
              });
            },
          },
        ]
      : []),
    {
      label: 'Delete…',
      icon: 'trash',
      danger: true,
      action: () => {
        void ws.requestDeleteSession(s.id).then(() => {
          if (!ws.getSession(s.id) && threads.paneIds.includes(s.id)) {
            threads.closePane(s.id);
          }
        });
      },
    },
  ];
}
