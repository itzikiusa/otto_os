// Debug-session pieces of `IdeDebugSessionSnapshot` (`ide.ts` re-exports them).

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
