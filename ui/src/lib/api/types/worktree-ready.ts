// `POST /repos/{id}/worktrees` `prepared` and `…/worktrees/setup`
// (docs/contracts/api.md; otto-git worktree_ready.rs). Part of lib/api/types.
import type { WorktreeInfo } from './git';

/** What a new worktree got from the main checkout's `.worktreeinclude`. */
export interface WorktreePrepared {
  copied: number;
  /** Cloned copy-on-write (APFS). */
  cloned: string[];
  /** Symlinked to the main checkout: this volume cannot clone. */
  linked: string[];
  /** Left out, with why ("big.bin: over the 100 MB copy limit"). */
  skipped: string[];
  /** The repo's setup command's name, when one is set. */
  setup?: string;
}

/** `POST /repos/{id}/worktrees`. */
export interface WorktreeCreated extends WorktreeInfo {
  prepared?: WorktreePrepared;
}

/** `POST /repos/{id}/worktrees/setup {path}`. */
export interface WorktreeSetupResult {
  name: string;
  ok: boolean;
  exit_code: number | null;
  /** The end of what it printed. */
  output: string;
}

/** Global setting `worktree_setup`: each repository's setup command, by its main checkout's path. */
export type WorktreeSetupSetting = Record<string, { name: string; command: string }>;
