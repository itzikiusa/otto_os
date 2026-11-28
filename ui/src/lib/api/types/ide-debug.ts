// Debug-session pieces of `IdeDebugSessionSnapshot` and the launch (`ide.ts` re-exports them).

export interface IdeDebugLaunch {
  program: string;
  args?: string[];
  build_flags?: string[];
  cwd?: string | null;
  test_name?: string | null;
  /** The line of the test's `test(` call (Playwright selects `file:line`). */
  test_line?: number | null;
  /** Kotest: debug the spec `program` names (empty) or its container / test at this path. */
  test_path?: string[] | null;
  is_test?: boolean;
  attach_port?: number | null;
  attach_pid?: number | null;
  attach_process_name?: string | null;
  core_file?: string | null;
  env?: Record<string, string>;
  env_file?: string | null;
  inherit_shell_env?: boolean;
  sdk_path?: string | null;
  maven_profiles?: string[];
  spring_profiles?: string[];
  skip_tests?: boolean;
  offline?: boolean;
  /** .NET test debugging: the one target framework to run (`dotnet test -f`). */
  test_framework?: string | null;
}
export type IdeDebugLaunchSpec = IdeDebugLaunch;

export interface IdeDebugOutput {
  category: 'stdout' | 'stderr' | 'console' | string;
  text: string;
  /** Monotonic per-session position (1-based, never reused); absent on older daemons. */
  seq?: number;
}

/** One process of a js-debug session: a Playwright worker, a Karma Chrome page. */
export interface IdeDebugTarget {
  id: string;
  name: string;
  /** Stopped: the one in focus, or one waiting for the focus to resume. */
  paused: boolean;
  /** Why it could not be attached. */
  error?: string | null;
}
