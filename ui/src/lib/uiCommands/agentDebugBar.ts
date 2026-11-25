// The driving bar while an agent drives a debug session in this document —
// pure, so unit/agentDebugBar.test.ts pins it. The state is uiControl's
// `agentDebug` (the IDE port reports launches, stops and narration).

import type { UiAgentRef } from '../api/types';

interface Driving { agent: Pick<UiAgentRef, 'session_id'>; running: number }
interface AgentDebug {
  agent: Pick<UiAgentRef, 'session_id'>;
  say: string;
  pausedSeq: number | null;
  proceededSeq: number | null;
  released: boolean;
}

/** The bar stays (no one-minute linger) while the driving agent's debuggee lives. */
export function drivesDebug(d: Driving | null, a: AgentDebug | null): boolean {
  return !!d && !!a && !a.released && a.agent.session_id === d.agent.session_id;
}

/** What the bar adds for the debug session: the agent's narration (kept while
 *  paused, so the person can read it) and whether Proceed shows — the
 *  debuggee is paused, nothing runs for the agent, and Proceed wasn't sent
 *  for this stop yet. */
export function debugBarView(d: Driving | null, a: AgentDebug | null): { say: string | null; proceed: boolean; paused: boolean } {
  if (!drivesDebug(d, a)) return { say: null, proceed: false, paused: false };
  const paused = a!.pausedSeq !== null;
  return {
    say: a!.say || null,
    paused,
    proceed: paused && d!.running === 0 && a!.proceededSeq !== a!.pausedSeq,
  };
}
