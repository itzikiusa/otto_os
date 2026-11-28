// Unity Editor attach (`POST /ide/debug/unity/instances`), mirrors
// crates/otto-ide/src/types/unity_attach.rs.

import type { IdeTargetSpec } from './ide';

export interface IdeUnityInstancesReq {
  target: IdeTargetSpec;
  /** The Unity project folder when it is not the target root; inside the target. */
  cwd?: string | null;
}

/** A running Unity Editor that has the project open. */
export interface IdeUnityEditor {
  pid: number;
  /** From `Library/EditorInstance.json` (`6000.0.23f1`). */
  version: string;
  /** The Editor's Mono soft-debugger port on loopback (`56000 + pid % 1000`). */
  port: number;
  project_root: string;
}

export interface IdeUnityInstancesResp {
  /** Empty when no Editor has the project open. */
  editors: IdeUnityEditor[];
}
