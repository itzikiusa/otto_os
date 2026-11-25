// The IDE store lives in the IDE module (modules/ide/ideStore.svelte.ts), and
// lib/ never imports modules/. So the IDE page binds this port while it is
// mounted (modules/ide/agentDebug/ideAgentPort.svelte.ts), and the
// `otto.ui_ide_debug_*` handlers (./ide.ts) await it — which also covers the
// IDE chunk still loading after the bridge opened the IDE. Everything here
// goes through the same store actions as the toolbar and the gutter, so what
// the agent does is what the person sees.

import type { IdeDebugSessionSnapshot, IdeVariable, UiAgentRef } from '../api/types';
import { createPagePort } from './pagePort';

/** The agent behind a command (the ui_command frame's `agent`); `runtime`
 *  `structured` = a Threads thread, else a terminal session. */
export type IdeAgent = UiAgentRef & { runtime?: string; cwd?: string };

/** The agent's run target, resolved exactly like a gutter click. */
export type IdeDebugTarget =
  /** A test function, a table-test case (`TestX/case`) or a testify suite
   *  method (`Method`, or `Suite/Method`). */
  | { kind: 'test'; file: string; name: string }
  /** The `main` of a file (Go main, a Python/Node/shell script, Rust bin). */
  | { kind: 'main'; file: string }
  /** A saved run configuration, by id or name. */
  | { kind: 'config'; name: string };

/** What `launch` runs — built by `resolveTarget`, shown in the start confirm. */
export interface IdeResolvedLaunch {
  /** Project root it runs in (absolute). */
  root: string;
  /** "TestFlowComponent", "cmd/api/main.go", the config's name. */
  label: string;
  language: string;
  /** Human preview of the command line: `go test -tags component -run ^TestFlowComponent$ ./test/flow`. */
  command: string;
  /** Absolute working directory. */
  cwd: string;
  /** Build flags after the project's Go build tags and the agent's `build_tags` were merged. */
  build_flags: string[];
  /** Only the env the AGENT asked for (the project environment is applied by the daemon). */
  env: Record<string, string>;
  /** Opaque: hand it back to `launch`. */
  readonly plan: unknown;
}

/** One breakpoint the agent set (tracked by `id`, so a breakpoint the
 *  debugger moved — delve binds a blank line to the next statement — is still
 *  the agent's). `line` is the bound line once the debugger verified it. */
export interface IdeAgentBreakpoint {
  id: string;
  /** Absolute path. */
  file: string;
  line: number;
  condition?: string | null;
  hit_condition?: string | null;
  log_message?: string | null;
  verified?: boolean;
  message?: string | null;
}

export interface IdeAgentBreakpointSpec {
  file: string;
  line: number;
  condition?: string | null;
  hit_condition?: string | null;
  log_message?: string | null;
}

export type IdeDebugCommand =
  | 'continue' | 'pause' | 'step_over' | 'step_into' | 'step_out' | 'run_to_cursor' | 'select_frame' | 'restart' | 'stop';

export interface IdePort {
  /** The project this IDE shows ('' = none yet). */
  readonly root: string;
  /** The open project's folders (primary first), absolute. */
  readonly projectRoots: readonly string[];
  /** This IDE's debug session right now (the store's live snapshot), or null. */
  readonly debug: IdeDebugSessionSnapshot | null;
  /** The debug session `agent` started here and still drives, or null. */
  agentDebugId(agent: Pick<IdeAgent, 'session_id'>): string | null;

  /** Show `root` in this IDE, and for a Threads session (`agent.runtime ===
   *  'structured'`) dock that thread beside the editor. Nothing happens when
   *  this IDE already shows a project containing `root`. Taking over an IDE
   *  that shows ANOTHER project asks the person first (attributed; names a
   *  live debug session that would stop) and goes through the normal switch
   *  (unsaved edits ask "Discard & switch"). Rejects `cancelled_by_user` when
   *  the person declines. Call it after your own checks, never twice in a row. */
  openProject(agent: IdeAgent, root: string, signal?: AbortSignal): Promise<void>;

  /** Opens `target.file` and resolves the target exactly as a gutter click
   *  would (test incl. table cases and testify suite methods, main, run
   *  config; build tags merged). No side effect beyond opening the file.
   *  Rejects `not_found` (no such test/main/config, naming what exists) or
   *  `invalid_args`. */
  resolveTarget(target: IdeDebugTarget, opts?: { build_tags?: string[]; args?: string[]; env?: Record<string, string> }): Promise<IdeResolvedLaunch>;

  /** Start the resolved launch through the store's own path: the prod gate
   *  (typed confirm, never remembered) and the one-live-session guard ("Stop
   *  and debug again"), then the Debug tab opens. Resolves the new session's
   *  id; rejects `cancelled_by_user` (a gate declined) or `failed` (the
   *  daemon refused / the build failed: the message is the IDE's start error).
   *  The session is recorded as driven by `agent` (driving bar, Proceed,
   *  "stopped driving"). Call it only after your own start confirm. */
  launch(agent: IdeAgent, plan: IdeResolvedLaunch): Promise<{ debug_id: string }>;

  /** A debugger action on the CURRENT session (the store's `debugCommand`);
   *  resolves the snapshot after it. `run_to_cursor` takes `path` + `line`. */
  command(action: IdeDebugCommand, extra?: { thread_id?: number; frame_id?: number; path?: string; line?: number }): Promise<IdeDebugSessionSnapshot | null>;

  /** Every snapshot the store applies (SSE stream + command responses), in
   *  order. Returns the unsubscribe. Wait for a stop with a `stop_seq` greater
   *  than the one you last returned, or `state` terminated/error. */
  onSnapshot(fn: (snap: IdeDebugSessionSnapshot) => void): () => void;

  /** Add the agent's breakpoints in the gutter (red dots, tooltip "Set by
   *  ‹agent›"); an existing breakpoint on that line is taken over. Live
   *  sessions get them at once. Resolves the agent's breakpoints after the
   *  daemon answered (verified / bound line / message). */
  setAgentBreakpoints(agent: IdeAgent, bps: readonly IdeAgentBreakpointSpec[]): Promise<IdeAgentBreakpoint[]>;
  /** Remove the agent's breakpoints: by id, by file+line (its original or bound
   *  line), or all of them (`'all'`). Never touches the person's. Resolves the
   *  count removed. */
  clearAgentBreakpoints(agent: Pick<IdeAgent, 'session_id'>, which: 'all' | readonly ({ id: string } | { file: string; line: number })[]): Promise<number>;
  /** The agent's breakpoints now. */
  agentBreakpoints(agent: Pick<IdeAgent, 'session_id'>): IdeAgentBreakpoint[];

  /** Children of a variable (DAP `variables`, read-only); paging `start`/`count`. */
  variables(ref: number, opts?: { start?: number; count?: number }): Promise<IdeVariable[]>;
  /** DAP `evaluate` in the REPL context of `frameId` (default: the selected
   *  frame); the person sees it in the Debug console. Runs code in the
   *  debuggee: confirm it first. */
  evaluate(expression: string, frameId?: number): Promise<{ result: string; type?: string | null; variables_reference: number }>;
  /** Lines `from`..`to` (1-based, inclusive) of a file — as the editor has
   *  it when it is open (unsaved edits included), else from disk. */
  readLines(file: string, from: number, to: number): Promise<string[]>;

  /** One line in the Debug console, e.g. `POST localhost:8080/orders (Claude)`. */
  consoleNote(text: string): void;
  /** The agent's one-line narration for the driving bar; it stays while the
   *  debuggee is paused. */
  say(agent: IdeAgent, text: string): void;
}

export const idePort = createPagePort<IdePort>('the IDE');

/** Called by the IDE page while it is mounted; returns the unbind. */
export function registerIdePort(store: IdePort): () => void {
  return idePort.bind(store);
}
