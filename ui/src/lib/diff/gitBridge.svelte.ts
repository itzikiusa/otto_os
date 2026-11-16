// Cross-component bridge for shared diff / blame / history panels (rune store).
// Lets a diff header or blame commit button open file-history / blame panels
// or focus a commit in the graph without prop-drilling through containers.
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

  stashCount = $state<Record<string, number>>({});
  fileTool = $state<FileToolRequest | null>(null);
  focus = $state<FocusRequest | null>(null);
  graphRefresh = $state<{ repoId: string; nonce: number } | null>(null);
  private nonce = 0;

  openFileTool(req: FileToolRequest): void {
    this.fileTool = req;
  }
  closeFileTool(): void {
    this.fileTool = null;
  }
  refreshGraph(repoId: string): void {
    this.nonce += 1;
    this.graphRefresh = { repoId, nonce: this.nonce };
  }
  focusCommit(repoId: string, sha: string): void {
    this.nonce += 1;
    this.focus = { repoId, sha, nonce: this.nonce };
  }
}

export const gitBridge = new GitBridge();
