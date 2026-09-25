// Pure session-scope rules shared by the workspace store and the ⌘I / Ask Otto
// engine — no runes, so node:test can import them (unit/sessionScope.test.ts).

/** Id of the daemon's hidden, system-owned **scratch** workspace — the home of
 *  workspace-less sessions. Mirrors `SCRATCH_WORKSPACE_ID` in
 *  `crates/otto-core/src/domain.rs`. Never in `workspaces` (hidden from
 *  `GET /workspaces`); read via `GET /workspaces/scratch`. Also the tabs/panes
 *  persistence key when no workspace is selected. */
export const SCRATCH_WORKSPACE_ID = 'scratch';

/** The sessions a workspace-scoped command ("close all shell sessions") may
 *  act on, from the store's list. The store loads EVERY scratch ("No
 *  workspace") session beside the current workspace's own, for the sidebar
 *  group — but those are not this workspace's, so a provider-wide close here
 *  must not archive them (the API host, which lists only
 *  `/workspaces/{id}/sessions`, never saw them). Two exceptions, the same as
 *  the tiled grid's: one the user opened as a tab here is on screen and
 *  theirs to close, and with no workspace selected they are all there is. */
export function workspaceCommandScope<S extends { id: string; workspace_id: string }>(
  sessions: readonly S[],
  currentId: string | null,
  openTabs: readonly string[],
): S[] {
  if (currentId === null) return sessions.slice();
  return sessions.filter((s) => s.workspace_id !== SCRATCH_WORKSPACE_ID || openTabs.includes(s.id));
}
