// DB Watch for the agent-debug `ide_debug_db` tool and each Stop's `db`
// (./ide.ts). The IDE's DB Watch store (modules/ide/dbwatch,
// `ideDbWatchState.svelte.ts`) binds this while the IDE is mounted, so the
// agent's watch IS the panel's watch — the person sees the same rows (raw),
// the agent reads them masked (`forAgent`). lib/ never imports modules/.
// The shapes are the parts of `IdeDbWatch` / `IdeDbWatchReadResp`
// (api/types/ide-db-watch.ts) the handler reads.

import type { IdeDbEngine } from '../api/types';
import { createPagePort } from './pagePort';

export type AgentDbTargetRef =
  | { kind: 'saved'; connection_id: string }
  | { kind: 'ephemeral'; ephemeral_id: string }
  | { kind: 'auto'; engine: IdeDbEngine };

export interface AgentDbItemSpec {
  object: string;
  filter?: { column: string; value: string | number | boolean | null } | null;
}

export interface AgentDbWatch {
  id: string;
  target: { label: string; local: boolean; engine: IdeDbEngine; environment?: string | null };
  bind?: { debug_session_id?: string; run_id?: string } | null;
  /** A checkpoint is being taken now. */
  busy: boolean;
}

export interface AgentDbItemDiff {
  object: string;
  window_note: string;
  columns: string[];
  added_count: number;
  modified_count: number;
  removed_count: number;
  diffs: {
    key: string;
    kind: 'added' | 'modified' | 'removed';
    row: unknown[];
    changes: { column: string; before: unknown; after: unknown; masked?: boolean }[];
  }[];
  more_count: number;
  error?: string | null;
}

export interface AgentDbRead {
  from_seq: number | null;
  to_seq: number | null;
  items: AgentDbItemDiff[];
}

export interface IdeDbWatchAgentOps {
  /** The watch of the open project, or null. */
  readonly watch: AgentDbWatch | null;
  /** Start (or replace) the project's watch; a production connection asks
   *  the person (typed confirm) first, never skippable from here. */
  start(req: { target: AgentDbTargetRef; items: AgentDbItemSpec[]; bind?: { debug_session_id: string } | null }): Promise<unknown>;
  stop(): Promise<unknown>;
  /** Rows changed since the previous stop / the start, MASKED with `forAgent`. */
  diffSince(compare: 'previous_stop' | 'start', opts: { forAgent: true }): Promise<AgentDbRead | null>;
}

export const ideDbWatchPort = createPagePort<IdeDbWatchAgentOps>('the IDE DB Watch');
