import type { McpServerDetail } from '../../lib/api/types';

class McpUiState {
  wsId = $state<string | null>(null);
  servers = $state<McpServerDetail[]>([]);
  loaded = $state(false);
  selectedServerId = $state<string | null>(null);
  expandedServerId = $state<string | null>(null);

  /** Returns `true` when the workspace changed (clearing workspace-scoped state). */
  syncWorkspace(nextWsId: string | null): boolean {
    if (this.wsId === nextWsId) return false;
    this.wsId = nextWsId;
    this.servers = [];
    this.loaded = false;
    this.selectedServerId = null;
    this.expandedServerId = null;
    return true;
  }

  selectServer(id: string | null): void {
    this.selectedServerId = id;
  }

  setServers(
    next: McpServerDetail[],
    fallbackSelection?: (items: McpServerDetail[]) => string | null,
  ): void {
    this.servers = next;
    this.loaded = true;
    if (this.selectedServerId && !next.some((s) => s.id === this.selectedServerId)) {
      this.selectedServerId = null;
    }
    if (this.expandedServerId && !next.some((s) => s.id === this.expandedServerId)) {
      this.expandedServerId = null;
    }
    if (!this.selectedServerId && next.length > 0 && fallbackSelection) {
      this.selectedServerId = fallbackSelection(next);
    }
  }

  patchServer(updated: McpServerDetail): void {
    this.servers = this.servers.map((s) => (s.id === updated.id ? updated : s));
  }

  resetForTests(): void {
    this.wsId = null;
    this.servers = [];
    this.loaded = false;
    this.selectedServerId = null;
    this.expandedServerId = null;
  }
}

export const mcpUiState = new McpUiState();
