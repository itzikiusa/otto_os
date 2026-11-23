// The workspace's enabled native MCP servers (`GET /workspaces/{id}/mcp-servers`),
// shared across open threads for 15 s so several panes cost one read.
const TTL_MS = 15_000;
const cache = new Map<string, { at: number; names: Promise<string[]> }>();

export function nativeServerNames(ws: string, get: (path: string) => Promise<unknown>, now = Date.now()): Promise<string[]> {
  const hit = cache.get(ws);
  if (hit && now - hit.at < TTL_MS) return hit.names;
  const names = get(`/workspaces/${encodeURIComponent(ws)}/mcp-servers`)
    .then((rows) => (Array.isArray(rows) ? rows : []).filter((r) => r?.enabled).map((r) => String(r.name)))
    .catch(() => {
      cache.delete(ws);
      return [] as string[];
    });
  cache.set(ws, { at: now, names });
  return names;
}
