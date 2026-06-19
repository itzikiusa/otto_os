// Open-file signal: a one-shot request to open a file (optionally at a
// line/column) in the right-panel Files tab. The terminal's link provider
// raises a request when a `path:line(:col)` reference is clicked; the primary
// FileTree section listens for it (via the bumped `n` counter) and loads +
// reveals the file. Kept tiny and decoupled so any source can drive it without
// reaching into the FileTree internals.

import { ui } from './ui.svelte';

export interface OpenFileRequest {
  /** Absolute path (already resolved against the session cwd by the caller). */
  path: string;
  /** 1-based line to reveal, if known. */
  line?: number;
  /** 1-based column to place the cursor at, if known. */
  col?: number;
  /** Monotonic counter — bumped on every request so listeners fire exactly once. */
  n: number;
}

class OpenFileStore {
  request: OpenFileRequest | null = $state(null);

  /** Ask the Files panel to open `path` (absolute) at an optional line/col. */
  open(path: string, line?: number, col?: number): void {
    const prev = this.request?.n ?? 0;
    this.request = { path, line, col, n: prev + 1 };
    // Surface the Files tab so the reveal is visible even if it was collapsed
    // or showing another tab.
    ui.openRight('files');
  }
}

export const openFile = new OpenFileStore();
