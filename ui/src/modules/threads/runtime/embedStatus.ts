// What an embedded thread tells its host through `onstatus`
// (lib/runtimeThreadPort.ts): enough for a folded header's dot. Pure —
// unit-tested in unit/threadsEmbedStatus.test.ts.
import type { RuntimeSessionStatus } from '../../../lib/api/types';
import type { RuntimeEmbedStatus } from '../../../lib/runtimeThreadPort';

export interface EmbedStatusInput {
  /** The session row's status (null while the row is not loaded). */
  status: RuntimeSessionStatus | null | undefined;
  /** Background work the agent started that is still running. */
  backgroundTasks: number;
  /** The conversation's latest turn is pending or running. */
  turnRunning: boolean;
  /** Approvals and questions waiting for an answer. */
  pendingRequests: number;
}

export function embedStatus(s: EmbedStatusInput): RuntimeEmbedStatus {
  if (s.status === 'closed') return 'closed';
  // Waiting on the person outranks working: the dot must ask for them.
  if (s.pendingRequests > 0 || s.status === 'awaiting_permission') return 'needs-you';
  if (s.turnRunning || s.status === 'working' || s.status === 'starting' || s.backgroundTasks > 0) return 'working';
  return 'idle';
}
