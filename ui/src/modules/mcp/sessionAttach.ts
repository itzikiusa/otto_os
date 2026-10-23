// Which state the MCP home's "Attach to sessions" row renders. The row never
// shows a guessed tick: until the daemon reports the workspace's setting it is
// a loading placeholder, and a failed read is an inline error with Retry
// (ux-design-6-05, ux-design-6w1-04). Used by OttoServerHome.svelte.

export type AttachView = 'no-workspace' | 'error' | 'loading' | 'ready';

export function attachView(
  wsId: string | null | undefined,
  attach: unknown,
  attachError: string | null,
): AttachView {
  if (!wsId) return 'no-workspace';
  if (attachError) return 'error';
  return attach == null ? 'loading' : 'ready';
}
