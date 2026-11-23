// Threads' source for the ⌘K switcher (lib/paletteThreads.svelte.ts):
// every live main thread with the sidebar's state, unread mark and project,
// ordered by threadSwitcher.ts; Enter opens in the focused pane, ⌘↵ beside it
// when a split is open, and "New thread: <query>" starts the New thread stage
// with the query as its draft. Installed by ThreadsPage while it is mounted
// (loaded on demand, so the page itself does not carry it).
import { paletteThreads, type PaletteThreadSource } from '../../lib/paletteThreads.svelte';
import { ws } from '../../lib/stores/workspace.svelte';
import { runtimeThreads } from './runtime/runtimeThreads.svelte';
import { runtimeViews } from './runtime/runtimeView.svelte';
import { threadOrigin } from './threadOrigins';
import { threadNav, threadRowState } from './threadNav.svelte';
import { switcherRows, type SwitcherThread } from './threadSwitcher';
import { isThreadSession, projectNameFromCwd } from './threadsHelpers';
import { threads } from './threadsStore.svelte';

function asks(id: string): SwitcherThread['asks'] {
  const pending = runtimeViews.projections[id]?.requests.filter((r) => r.status === 'pending') ?? [];
  if (pending.length === 0) return null;
  return pending.some((r) => r.kind !== 'question') ? 'approval' : 'question';
}

export const threadSwitcherSource: PaletteThreadSource = {
  label: 'Threads',
  rows() {
    const list = [...ws.sessions.filter((s) => !s.archived && isThreadSession(s)), ...runtimeThreads.rows()].filter(
      (s) => threadOrigin(s) === 'main',
    );
    const items: SwitcherThread[] = list.map((s) => ({
      id: s.id,
      title: s.title,
      project: s.cwd ? projectNameFromCwd(s.cwd, '') : '',
      state: threadRowState(s),
      unread: ws.unread[s.id] === true || runtimeThreads.unread[s.id] === true,
      asks: asks(s.id),
      lastActive: s.last_active_at,
    }));
    return switcherRows(items, threadNav.order, Date.now());
  },
  open(id, beside) {
    if (beside && threads.layoutMode !== 'single' && threads.paneIds.length > 1) threads.openBeside(id);
    else threads.selectThread(id);
  },
  newThread(draft) {
    threads.startNewThread({ workspaceId: ws.currentId, draft });
  },
};

/** Install the source; returns the uninstall. */
export function installThreadSwitcher(): () => void {
  return paletteThreads.install(threadSwitcherSource);
}
