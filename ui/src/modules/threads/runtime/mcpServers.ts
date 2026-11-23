// What a thread's agent can use over MCP, as rows a person reads: every
// server — Otto's own tools, the workspace's control-plane servers (handed to
// the CLI, or reached through Otto's tools), the CLI's own config — with a
// status WORD and, when it is not connected, why. Nothing is left out
// silently: a server the provider could not take is listed with the reason.
// Pure (unit/threadsMcpServers.test.ts); shown by RuntimeMcpServers.svelte and
// exported for the thread details card.
import type { McpServerStatus } from '../../../lib/api/types.ts';

export type McpTone = 'success' | 'warning' | 'danger' | 'dim';

export interface McpServerRow {
  name: string;
  /** Where it comes from, in words. */
  from: string;
  /** "Connected", "Needs sign-in", "Not supported by Codex" … */
  status: string;
  tone: McpTone;
  reason: string | null;
}

/** A governed (Otto-run) control-plane server of the thread's workspace:
 *  it reaches the agent as tools of Otto's own server. */
export interface GovernedServer {
  name: string;
  health_status: 'unknown' | 'healthy' | 'unhealthy' | 'disabled';
  health_error: string | null;
  /** Its enabled tools (`GET /mcp/gateway/tools`). */
  tools: number;
}

function word(s: McpServerStatus['state'], agent: string): [string, McpTone] {
  switch (s) {
    case 'connected': return ['Connected', 'success'];
    case 'starting': return ['Starting', 'dim'];
    case 'added': return ['Added', 'dim'];
    case 'needs_auth': return ['Needs sign-in', 'warning'];
    case 'failed': return ['Failed', 'danger'];
    case 'disabled': return ['Disabled', 'dim'];
    case 'unsupported': return [`Not supported by ${agent}`, 'warning'];
  }
}

/** Every row, problems first, then Otto's, the workspace's and the CLI's own. */
export function mcpServerRows(servers: readonly McpServerStatus[], governed: readonly GovernedServer[], agent: string): McpServerRow[] {
  const rows: (McpServerRow & { rank: number })[] = [];
  const order = { otto: 0, workspace: 1, cli: 3 } as const;
  for (const s of servers) {
    const [status, tone] = word(s.state, agent);
    const from = s.source === 'otto' ? 'Otto' : s.source === 'workspace' ? 'This workspace' : `${agent}’s own config`;
    const reason = s.reason ?? (s.state === 'added' ? `${agent} doesn’t report whether it connected` : null);
    rows.push({ name: s.name, from, status, tone, reason, rank: order[s.source] });
  }
  // Governed servers reach the agent as tools of Otto's own server.
  const otto = servers.find((s) => s.name === 'otto');
  const ottoUp = !!otto && (otto.state === 'connected' || otto.state === 'added' || otto.state === 'starting');
  for (const g of governed) {
    if (servers.some((s) => s.name === g.name)) continue;
    const via = `Through Otto · ${g.tools} ${g.tools === 1 ? 'tool' : 'tools'}`;
    const row = (status: string, tone: McpTone, reason: string | null) => rows.push({ name: g.name, from: via, status, tone, reason, rank: 2 });
    if (!ottoUp) row('Not available', 'warning', 'It reaches the agent through Otto’s tools, which this thread doesn’t have');
    else if (g.health_status === 'unhealthy') row('Failed', 'danger', g.health_error ?? 'Otto couldn’t reach it');
    else if (g.tools === 0) row('No tools', 'warning', 'None of its tools is turned on (MCP → this server → Tools)');
    else {
      const [status, tone] = word(otto!.state, agent);
      row(status, tone, null);
    }
  }
  const bad = (r: McpServerRow) => (r.tone === 'danger' || r.tone === 'warning' ? 0 : 1);
  return rows
    .sort((a, b) => bad(a) - bad(b) || a.rank - b.rank || a.name.localeCompare(b.name))
    .map(({ rank: _rank, ...r }) => r);
}

/** One line for a details card: "4 MCP servers · 1 needs attention". */
export function mcpSummary(rows: readonly McpServerRow[]): string {
  if (rows.length === 0) return 'No MCP servers';
  const off = rows.filter((r) => r.tone === 'danger' || r.tone === 'warning').length;
  const n = `${rows.length} MCP ${rows.length === 1 ? 'server' : 'servers'}`;
  return off > 0 ? `${n} · ${off} ${off === 1 ? 'needs' : 'need'} attention` : n;
}

/** The workspace's enabled servers Otto hands the CLI differ from what this
 *  thread's agent was started with: a server turned on or off since. (Otto's
 *  own tools and governed servers update in place where the CLI re-lists.) */
export function mcpChanged(spawn: readonly McpServerStatus[], enabledNative: readonly string[]): boolean {
  if (spawn.length === 0) return false;
  const had = new Set(spawn.filter((s) => s.source === 'workspace' && s.name !== 'Workspace servers').map((s) => s.name));
  return had.size !== enabledNative.length || enabledNative.some((n) => !had.has(n));
}
