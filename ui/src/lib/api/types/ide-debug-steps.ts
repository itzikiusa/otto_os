// Debug commands, their requests and the Debug All steps of `IdeDebugSessionSnapshot` (`ide.ts` re-exports them).

export interface IdeDebugCommandReq {
  action: IdeDebugAction;
  thread_id?: number | null;
  frame_id?: number | null;
  path?: string | null;
  line?: number | null;
}

export interface IdeDebugVariablesReq {
  variables_reference: number;
}

export interface IdeDebugEvaluateReq {
  expression: string;
  frame_id?: number | null;
  context?: string | null;
}

export interface IdeDebugSetVariableReq {
  variables_reference: number;
  name: string;
  value: string;
}

export type IdeDebugStepState = 'pending' | 'running' | 'passed' | 'failed' | 'no_tests' | 'error' | 'skipped';

/** One step of a Debug All run. */
export interface IdeDebugStep {
  /** `Api.Tests (net10.0)`, `lib (src/root.zig)`. */
  label: string;
  state: IdeDebugStepState;
  exit_code?: number | null;
  /** `42 passed`, `1 of 17 failed`, `no tests`, or why it couldn't start. */
  summary?: string | null;
  /** Up to 5 failed test names. */
  failed_tests?: string[];
  /** What a single-test Debug of a failed test launches (test project or suite source). */
  program: string;
  /** .NET target framework of the step. */
  test_framework?: string | null;
  started_ms?: number | null;
  ended_ms?: number | null;
}

export type IdeDebugAction =
  | 'continue'
  | 'pause'
  | 'step_over'
  | 'step_into'
  | 'step_out'
  | 'run_to_cursor'
  | 'select_frame'
  | 'restart'
  | 'stop'
  /** Debug All: end the current step and start the next. */
  | 'skip_step';
