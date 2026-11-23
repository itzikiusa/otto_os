// "MCP servers" from a thread's ⋯ menu: which thread's list the sheet
// (RuntimeMcpServers.svelte, mounted by ThreadCanvas) shows.
class McpServersSheet {
  id = $state<string | null>(null);
  open(id: string): void {
    this.id = id;
  }
  close(): void {
    this.id = null;
  }
}

export const mcpServersSheet = new McpServersSheet();
