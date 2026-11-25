// Breakpoints an agent set (`otto.ui_ide_debug_*`): ordinary IDE breakpoints
// with an `agent` tag, so the gutter says "Set by ‹agent›" and the agent can
// clear ITS breakpoints — by id — wherever the debugger moved them (delve binds
// a blank line to the next statement; the snapshot sync carries the tag along).
// Pure: the port (ideAgentPort.svelte.ts) writes through `mutatePathBreakpoints`.

import type { IdeBreakpoint, IdeBreakpointAgentTag } from '../store/ideTypes.ts';
import type { IdeAgentBreakpoint, IdeAgentBreakpointSpec } from '../../../lib/uiCommands/idePort.ts';

type Which = 'all' | readonly ({ id: string } | { file: string; line: number })[];

/** `list` (one file's breakpoints) with the agent's `specs` for that file
 *  added; a breakpoint already on that line becomes the agent's. `newId` names
 *  each new one. */
export function withAgentBreakpoints(
  list: readonly IdeBreakpoint[],
  path: string,
  specs: readonly IdeAgentBreakpointSpec[],
  agent: { session_id: string; name: string },
  newId: () => string,
): IdeBreakpoint[] {
  const next = [...list];
  for (const s of specs) {
    const line = Math.max(1, Math.floor(s.line));
    const fields = {
      condition: s.condition ?? null,
      hit_condition: s.hit_condition ?? null,
      log_message: s.log_message ?? null,
    };
    const at = next.findIndex((b) => b.line === line);
    const prev = at >= 0 ? next[at] : null;
    // Setting the same line again keeps the breakpoint (and its id).
    const id = prev?.agent?.session_id === agent.session_id ? prev.agent.id : newId();
    const tag: IdeBreakpointAgentTag = { session_id: agent.session_id, name: agent.name, id, line };
    const bp: IdeBreakpoint = { ...(prev ?? {}), path, line, enabled: true, ...fields, agent: tag };
    if (at >= 0) next[at] = bp;
    else next.push(bp);
  }
  return next;
}

function matches(bp: IdeBreakpoint, path: string, which: Which): boolean {
  if (which === 'all') return true;
  return which.some((w) =>
    'id' in w ? bp.agent?.id === w.id : w.file === path && (bp.line === w.line || bp.agent?.line === w.line));
}

/** `list` without the agent's breakpoints that `which` names; the person's stay. */
export function withoutAgentBreakpoints(list: readonly IdeBreakpoint[], path: string, sessionId: string, which: Which): IdeBreakpoint[] {
  return list.filter((b) => b.agent?.session_id !== sessionId || !matches(b, path, which));
}

/** Files holding one of the agent's breakpoints that `which` names. */
export function agentBreakpointPaths(map: Record<string, readonly IdeBreakpoint[]>, sessionId: string, which: Which): string[] {
  return Object.entries(map)
    .filter(([p, list]) => list.some((b) => b.agent?.session_id === sessionId && matches(b, p, which)))
    .map(([p]) => p);
}

/** The agent's breakpoints, as the port reports them (bound line once verified). */
export function agentBreakpointsOf(map: Record<string, readonly IdeBreakpoint[]>, sessionId: string): IdeAgentBreakpoint[] {
  const out: IdeAgentBreakpoint[] = [];
  for (const [path, list] of Object.entries(map)) {
    for (const b of list) {
      if (b.agent?.session_id !== sessionId) continue;
      out.push({
        id: b.agent.id,
        file: path,
        line: b.line,
        ...(b.condition ? { condition: b.condition } : {}),
        ...(b.hit_condition ? { hit_condition: b.hit_condition } : {}),
        ...(b.log_message ? { log_message: b.log_message } : {}),
        ...(b.verified !== undefined ? { verified: b.verified } : {}),
        ...(b.message ? { message: b.message } : {}),
      });
    }
  }
  return out;
}

/** The gutter tooltip line for a breakpoint the agent set. */
export function agentBreakpointTitle(bp: Pick<IdeBreakpoint, 'agent'>): string | null {
  return bp.agent ? `Set by ${bp.agent.name}` : null;
}
