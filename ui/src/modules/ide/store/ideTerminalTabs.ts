// Pure helper for pruning stale IDE terminal tabs (local and SSH) whose
// backing sessions were deleted or archived outside the IDE panel.

export interface IdeTerminalTabEntry {
  id: string;
  label: string;
  kind: 'local' | 'ssh';
  targetKey: string;
  cwd: string;
}

export interface IdeTerminalSessionView {
  id: string;
  archived?: boolean;
}

/** Keep a local tab while its session is present and unarchived; keep an SSH
 *  tab while its session is present and unarchived OR still tracked as live
 *  (not killed / not exited) in `statusMap` (e.g. across workspaces or right
 *  after `POST /connections/:id/open` before `session_created` arrives). */
export function pruneIdeTerminalTabs<T extends Pick<IdeTerminalTabEntry, 'id' | 'kind'>>(
  tabs: readonly T[],
  sessions: readonly IdeTerminalSessionView[],
  otherWsSessions: readonly IdeTerminalSessionView[],
  archivedSessions: readonly Pick<IdeTerminalSessionView, 'id'>[],
  statusMap: Readonly<Record<string, string | undefined>>,
): T[] {
  const liveIds = new Set<string>();
  for (let i = 0; i < sessions.length; i++) {
    if (!sessions[i].archived) liveIds.add(sessions[i].id);
  }
  for (let i = 0; i < otherWsSessions.length; i++) {
    if (!otherWsSessions[i].archived) liveIds.add(otherWsSessions[i].id);
  }
  const archivedIds = new Set<string>();
  for (let i = 0; i < archivedSessions.length; i++) {
    archivedIds.add(archivedSessions[i].id);
  }
  return tabs.filter((t) => {
    if (archivedIds.has(t.id)) return false;
    if (liveIds.has(t.id)) return true;
    if (t.kind === 'ssh') {
      const status = statusMap[t.id];
      return status !== undefined && status !== 'exited';
    }
    return false;
  });
}
