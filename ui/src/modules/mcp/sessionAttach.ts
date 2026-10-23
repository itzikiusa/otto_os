// Which state the MCP home's "Attach to sessions" row and its "What my
// sessions see" panel render. Neither ever shows a guessed value: until the
// daemon reports the workspace's setting (or its gateway tools) the view is a
// loading placeholder, and a failed read is an inline error with Retry
// (ux-design-6-05, ux-design-6w1-04, ux-design-6w2-01). Used by
// OttoServerHome.svelte.

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

/** The "Built-in tools: …" line of the sessions panel, driven by the same
 *  state as the attach row above it — so a loading or failed read never reads
 *  as "attached". */
export function builtInToolsLine(view: AttachView, attach: { attached: boolean } | null): string {
  switch (view) {
    case 'ready':
      return `Built-in tools: ${attach?.attached ? 'attached' : 'not attached'}`;
    case 'loading':
      return 'Built-in tools: checking…';
    case 'error':
      return 'Built-in tools: unknown (see above)';
    default:
      return 'Built-in tools: select a workspace';
  }
}

export type GatewayView = 'no-workspace' | 'error' | 'loading' | 'ready';

/** The gateway-tools list: a first load (nothing to show yet) is a loading
 *  placeholder, not a "0" count; a refresh keeps the list it already has. */
export function gatewayView(
  wsId: string | null | undefined,
  loading: boolean,
  error: string | null,
  count: number,
): GatewayView {
  if (!wsId) return 'no-workspace';
  if (error) return 'error';
  return loading && count === 0 ? 'loading' : 'ready';
}
