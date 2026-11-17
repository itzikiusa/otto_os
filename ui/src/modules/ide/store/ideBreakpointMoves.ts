// The gutter's breakpoint moves (edits carried breakpoints to new lines) reach the store here,
// once per settled edit, through the one breakpoint writer `mutatePathBreakpoints` — which
// saves them for the project and re-sends them to a live debug session, so the session breaks
// on the code the person set the breakpoint on.

import { applyBreakpointMoves, type IdeBreakpointMove } from '../../../lib/components/cm-ide-bp-model';
import { toasts } from '../../../lib/toast.svelte';
import { mutatePathBreakpoints } from '../shell/ideProjectHelpers';

type BreakpointHost = Parameters<typeof mutatePathBreakpoints>[0];

/** Rewrites one file's breakpoint lines; says so when two breakpoints met on one line. */
export function moveIdeBreakpoints(host: BreakpointHost, path: string, moves: IdeBreakpointMove[]): void {
  if (!moves.length) return;
  mutatePathBreakpoints(host, path, (_p, list) => applyBreakpointMoves(list, moves));
  const merged = moves.filter((m) => m.reason === 'merged');
  if (merged.length) {
    const lines = merged.map((m) => m.into).filter((n): n is number => n != null);
    toasts.info(
      merged.length === 1 ? 'Two breakpoints met on one line' : `${merged.length} breakpoints met others on one line`,
      `Kept one on line ${[...new Set(lines)].join(', ')}.`,
    );
  }
}
