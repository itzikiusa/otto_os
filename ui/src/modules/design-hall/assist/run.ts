// Sending an ask: one assist turn or a variants run, and remembering what the
// person asked (the daemon doesn't keep the request text) so the thread reads
// as a conversation. Shared by the Otto panel and the lobby hand-off.

import { ApiError } from '../../../lib/api/client';
import type { DesignAssistTurn, DesignVariantRun } from '../../../lib/api/types';
import { asks } from './asks.svelte';
import { startAssist, startVariants } from './api';
import type { AssistRequest } from './model';

export type Sent = { kind: 'turn'; turn: DesignAssistTurn } | { kind: 'run'; run: DesignVariantRun };

export async function sendAsk(artifactId: string, req: AssistRequest, selectionLabel: string | null = null): Promise<Sent> {
  const at = new Date().toISOString();
  if (req.kind === 'variants') {
    const run = await startVariants(artifactId, req.body);
    asks.add({ key: run.run_id, artifactId, prompt: req.prompt, intent: req.intent, selectionLabel, at });
    return { kind: 'run', run };
  }
  const turn = await startAssist(artifactId, req.body);
  asks.add({ key: turn.turn_id, artifactId, prompt: req.prompt, intent: req.intent, selectionLabel, at });
  return { kind: 'turn', turn };
}

/** A person-readable reason an ask could not start. */
export function askError(e: unknown): string {
  if (e instanceof ApiError && e.status === 409) return 'Otto is already working on this design. Wait for that turn to finish.';
  if (e instanceof ApiError && e.status === 400) return e.message || 'This design can’t be edited by an agent (binary formats aren’t supported).';
  if (e instanceof ApiError && e.status === 403) return 'You need edit access to this design’s workspace to ask Otto.';
  return e instanceof Error ? e.message : String(e);
}
