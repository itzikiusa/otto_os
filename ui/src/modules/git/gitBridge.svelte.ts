// Cross-component bridge for the git module (rune store, same pattern as
// `lib/stores/git.svelte.ts`). Lets a panel that has no prop path to the
// graph ask it to select a commit, and lets any diff header open the
// file-history / blame panels that RepoView renders — without prop drilling
// through GraphView → WipPanel → DiffViewer.
export type FileToolKind = 'history' | 'blame';

export interface FileToolRequest {
  kind: FileToolKind;
  repoId: string;
  path: string;
  /** Revision for blame (default HEAD). */
  rev?: string;
}

export interface FocusRequest {
  repoId: string;
  sha: string;
  /** Monotonic so re-selecting the same sha re-fires the graph's effect. */
  nonce: number;
}

export type RecoveryMode = 'history' | 'rebase' | 'bisect';
class GitBridge {
  recovery = $state<{ repoId: string; mode: RecoveryMode; onto?: string } | null>(null);
  bisectTargets = $state<Record<string, { good?: string; bad?: string }>>({});
  openRecovery(repoId: string, mode: RecoveryMode = 'history', onto?: string): void {
    this.recovery = { repoId, mode, onto };
  }
  selectBisect(repoId: string, side: 'good' | 'bad', sha: string): void {
    this.bisectTargets[repoId] = { ...this.bisectTargets[repoId], [side]: sha };
    this.openRecovery(repoId, 'bisect');
  }

  fileTool = $state<FileToolRequest | null>(null);
  focus = $state<FocusRequest | null>(null);
  private nonce = 0;

  openFileTool(req: FileToolRequest): void {
    this.fileTool = req;
  }
  closeFileTool(): void {
    this.fileTool = null;
  }
  /** Ask the graph of `repoId` to select `sha` (no-op if not loaded there). */
  focusCommit(repoId: string, sha: string): void {
    this.nonce += 1;
    this.focus = { repoId, sha, nonce: this.nonce };
  }
}

export const gitBridge = new GitBridge();
