// Threads' source for the ⌘K switcher (lib/paletteThreads.svelte.ts):
// every live main thread, ordered and worded by the sidebar's inbox logic
// (threadInbox.ts, threadSwitcher.ts) — Approval / Question from its compact
// request read; Enter opens in the focused pane, ⌘↵ beside it when a split is
// open, and "New thread: <query>" starts the New thread stage with the query
// as its draft. Installed by ThreadsPage while it is mounted (loaded on
// demand, so the page itself does not carry it).
import { paletteThreads, type PaletteThreadSource } from '../../lib/paletteThreads.svelte';
import { now, rel } from '../../lib/stores/now.svelte';
import type { Session } from '../../lib/api/types';
import { ws } from '../../lib/stores/workspace.svelte';
import { isRuntimeThread, runtimeThreads } from './runtime/runtimeThreads.svelte';
import { inboxClass, statusWord, type InboxClass } from './threadInbox';
import { inboxLive } from './threadInbox.svelte';
import { threadOrigin } from './threadOrigins';
import { threadNav, threadRowState } from './threadNav.svelte';
import { switcherOrder, wordTone } from './threadSwitcher';
import { isThreadSession, projectNameFromCwd } from './threadsHelpers';
import { threads } from './threadsStore.svelte';

/** The class the sidebar gives a row (ThreadSidebar's `classOf`). */
function classOf(s: Session): InboxClass {
  const st = threadRowState(s);
  const focused = threads.focusedSessionId === s.id && !threads.newDraftMode;
  const unread = !focused && st.key !== 'needs-you' && (ws.unread[s.id] === true || runtimeThreads.unread[s.id] === true);
  return inboxClass(st.key, unread);
}

function live(): { s: Session; cls: InboxClass; id: string; lastActive: string }[] {
  const list = [...ws.sessions.filter((s) => !s.archived && isThreadSession(s)), ...runtimeThreads.rows()].filter((s) => threadOrigin(s) === 'main');
  return list.map((s) => ({ s, cls: classOf(s), id: s.id, lastActive: s.last_active_at }));
}

export const threadSwitcherSource: PaletteThreadSource = {
  label: 'Threads',
  rows() {
    return switcherOrder(live(), threadNav.order).map(({ s, cls }) => {
      const word =
        statusWord(cls, {
          kinds: inboxLive.kindsOf(s.id),
          workingSince: cls === 'working' ? (inboxLive.since[s.id] ?? null) : null,
          now: cls === 'working' ? now() : 0,
          limited: cls === 'failed' && threadRowState(s).label === 'Limited',
        }) ?? rel(s.last_active_at);
      return { id: s.id, title: s.title, project: s.cwd ? projectNameFromCwd(s.cwd, '') : '', word, tone: wordTone(word) };
    });
  },
  open(id, beside) {
    if (beside && threads.layoutMode !== 'single' && threads.paneIds.length > 1) threads.openBeside(id);
    else threads.selectThread(id);
  },
  newThread(draft) {
    threads.startNewThread({ workspaceId: ws.currentId, draft });
  },
  warm() {
    for (const { s, cls } of live()) if (isRuntimeThread(s) && (cls === 'needs' || cls === 'working')) inboxLive.ensure(s.id);
  },
};

/** Install the source; returns the uninstall. */
export function installThreadSwitcher(): () => void {
  const off = paletteThreads.install(threadSwitcherSource);
  const unlisten = inboxLive.mount();
  return () => {
    off();
    unlisten();
  };
}

