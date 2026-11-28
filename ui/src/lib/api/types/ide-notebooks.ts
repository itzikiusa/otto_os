// Notebook kernels (Jupyter / built-in): `/ide/notebooks/*` request and
// response bodies (docs/contracts/api.md; crates/otto-ide/src/notebooks.rs).
import type { IdeMlAcceleratorInfo, IdeTargetSpec } from './ide';

export interface IdeNotebookKernelInfo {
  id: string;
  display_name: string;
  language: string;
  /** The kernel's interpreter / command (`argv[0]` for a Jupyter kernel). */
  interpreter: string;
  kind: 'jupyter' | 'builtin';
  version?: string | null;
  active: boolean;
  /** Jupyter only: the kernelspec name written to `metadata.kernelspec.name`. */
  spec_name?: string | null;
  origin?: 'user' | 'system' | 'env' | 'workspace' | null;
  /** Jupyter only: the full launch command, shell-quoted. */
  argv_preview?: string | null;
  /** A workspace kernelspec not accepted yet: ask before its first start. */
  needs_trust?: boolean;
  /** The spec declares debugger support (the running kernel's status is authoritative). */
  debugger?: boolean;
  /** Python envs: venv | uv | conda | poetry | system. */
  env_kind?: string | null;
}

export interface IdeNotebookKernelStatus {
  kernel_id: string;
  state: 'starting' | 'idle' | 'busy' | 'dead';
  exit_code?: number | null;
  debugger?: boolean;
}

/** `GET /ide/notebooks/stream` events (SSE `data:` JSON). */
export type IdeNotebookStreamEvent =
  | ({ kind: 'status' } & (IdeNotebookKernelStatus | { state: 'none' }))
  | { kind: 'output'; run_id: string; index: number; output: Record<string, unknown> }
  | { kind: 'clear_output'; run_id: string }
  | { kind: 'input_request'; run_id: string; prompt: string; password: boolean }
  | { kind: 'input_done' };

export interface IdeNotebookVariable {
  name: string;
  var_type: string;
  shape?: string | null;
  preview: string;
}

/** `POST /ide/notebooks/kernels`. */
export interface IdeNotebookKernelsReq {
  target: IdeTargetSpec;
  notebook_path?: string | null;
}

/** `POST /ide/notebooks/execute`. */
export interface IdeNotebookExecuteReq {
  target: IdeTargetSpec;
  notebook_path: string;
  code: string;
  /** A kernel `id` from `/ide/notebooks/kernels` (preferred over `interpreter`). */
  kernel_id?: string | null;
  interpreter?: string | null;
  /** Jupyter: no limit unless given; the built-in kernel: 120 s. */
  timeout_secs?: number | null;
  /** Tags this run's events on `/ide/notebooks/stream`. */
  run_id?: string | null;
  /** The person accepted starting this workspace kernelspec. */
  trust_kernel?: boolean;
}

/** `POST /ide/notebooks/{interrupt,restart,variables,shutdown}` (restart reads the kernel fields). */
export interface IdeNotebookSessionReq {
  target: IdeTargetSpec;
  notebook_path: string;
  kernel_id?: string | null;
  interpreter?: string | null;
  trust_kernel?: boolean;
}

/** `POST /ide/notebooks/input`: answers the pending `input()` / `getpass()`. */
export interface IdeNotebookInputReq {
  target: IdeTargetSpec;
  notebook_path: string;
  value: string;
}

export interface IdeNotebookOkResp {
  ok: boolean;
  message?: string | null;
}

export interface IdeNotebookKernelsResp {
  kernels: IdeNotebookKernelInfo[];
  accelerators: IdeMlAcceleratorInfo[];
  active_session_id?: string | null;
  active_interpreter?: string | null;
  execution_count?: number;
  kernel_status?: IdeNotebookKernelStatus | null;
}

export interface IdeNotebookExecuteResp {
  session_id: string;
  interpreter: string;
  execution_count: number;
  /** `dead`: the kernel stopped mid-cell. */
  status: 'ok' | 'error' | 'aborted' | 'dead' | string;
  elapsed_ms: number;
  outputs: Record<string, unknown>[];
  variables: IdeNotebookVariable[];
}

export interface IdeNotebookVariablesResp {
  session_id?: string | null;
  execution_count?: number;
  variables: IdeNotebookVariable[];
}
