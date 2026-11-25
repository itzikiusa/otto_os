// DB Watch for the agent-debug `ide_debug_db` tool and each Stop's `db`
// (./ide.ts). The IDE's DB Watch store (modules/ide/dbwatch,
// `ideDbWatchState.svelte.ts`) binds itself here while the IDE page is mounted
// (agentDebug/ideAgentPort.svelte.ts), so the agent's watch IS the panel's
// watch — the person sees the same rows (raw), the agent reads them masked
// (`forAgent`). lib/ never imports modules/.

import type { IdeDbWatch, IdeDbWatchItemSpec, IdeDbWatchReadResp, IdeDbWatchTargetRef } from '../api/types/ide-db-watch';
import { createPagePort } from './pagePort';

export interface IdeDbWatchAgentOps {
  /** The watch of the open project, or null. */
  readonly watch: IdeDbWatch | null;
  /** Why the last start / read failed (a people sentence), or null. */
  readonly error: string | null;
  /** Start (or replace) the project's watch; a production connection asks
   *  the person (typed confirm, naming `asker`) first, never skippable from
   *  here. Null = declined, or failed (then `error` says why). */
  start(
    req: { target: IdeDbWatchTargetRef; items: IdeDbWatchItemSpec[]; bind?: { debug_session_id: string } | null },
    opts?: { asker?: string },
  ): Promise<IdeDbWatch | null>;
  stop(): Promise<unknown>;
  /** Rows changed since the previous stop / the start, MASKED with `forAgent`. */
  diffSince(compare: 'previous_stop' | 'start', opts: { forAgent: true }): Promise<IdeDbWatchReadResp | null>;
}

export const ideDbWatchPort = createPagePort<IdeDbWatchAgentOps>('the IDE DB Watch');
