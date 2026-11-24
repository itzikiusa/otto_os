// The MCP Control Plane's landing in words (pure; unit/mcpHome.test.ts): what
// waits on a person (approval rows) and, when nothing does, four sentences —
// servers connected · tools agents may use · what asks first · recent denials —
// each with one link. No new request: it reads the approvals list, the server
// list, the Otto tool catalog and the auto-approve rules the page loads.

export interface ApprovalLike {
  kind: string;
  tool: string | null;
  title: string;
  args_redacted_json: string;
  status: string;
  decided_at: string | null;
  expires_at: string | null;
}
export interface ToolLike {
  mutating: boolean;
  enabled: boolean;
}
export interface ServerLike {
  health_status: string;
}
export interface Sentence {
  text: string;
  link: string;
  /** Route (router.go) the link opens. */
  to: string;
  testid: string;
}

const DAY = 86_400_000;
const n = (count: number, one: string, many = `${one}s`) => `${count.toLocaleString('en-US')} ${count === 1 ? one : many}`;

/** Pending requests first — newest first, as the list arrives. */
export const pendingOf = <T extends ApprovalLike>(list: readonly T[]): T[] => list.filter((a) => a.status === 'pending');

/** Denied in the last `days` days. */
export function recentDenials(list: readonly ApprovalLike[], now: number, days = 7): number {
  return list.filter((a) => a.status === 'denied' && a.decided_at && now - Date.parse(a.decided_at) <= days * DAY).length;
}

/** What a request will do, in words, from its redacted arguments: the tool,
 *  then up to three short top-level values ("otto.git_push · repo otto ·
 *  branch fx/x"). Objects, arrays and long strings are left out. */
export function argsLine(a: Pick<ApprovalLike, 'tool' | 'args_redacted_json'>): string {
  const parts: string[] = [];
  if (a.tool) parts.push(a.tool);
  try {
    const args = JSON.parse(a.args_redacted_json) as Record<string, unknown>;
    for (const [k, v] of Object.entries(args ?? {})) {
      if (parts.length >= 4) break;
      if (typeof v === 'string' && v && v.length <= 40) parts.push(`${k} ${v}`);
      else if (typeof v === 'number' || typeof v === 'boolean') parts.push(`${k} ${v}`);
    }
  } catch {
    /* unparseable args: the tool alone */
  }
  return parts.join(' · ');
}

/** The four sentences under "Nothing waits on you". `null` inputs are
 *  still loading or failed (the page shows one notice line for that). */
export function summarySentences(input: {
  servers: readonly ServerLike[] | null;
  tools: readonly ToolLike[] | null;
  rules: number | null;
  approvalsOn: boolean;
  approvals: readonly ApprovalLike[];
  now: number;
}): Sentence[] {
  const out: Sentence[] = [];
  const { servers, tools, rules } = input;
  if (servers) {
    if (servers.length === 0) {
      out.push({ text: 'No external MCP servers yet.', link: 'Add a server', to: 'mcp/servers', testid: 'mcp-sum-servers' });
    } else {
      const ok = servers.filter((s) => s.health_status === 'healthy').length;
      const bad = servers.filter((s) => s.health_status === 'unhealthy').length;
      const rest = servers.length - ok - bad;
      const parts = [ok && `${ok} connected`, bad && `${bad} can’t connect`, rest && `${rest} not checked`].filter(Boolean);
      out.push({ text: `${n(servers.length, 'external server')}: ${parts.join(', ')}.`, link: 'Open servers', to: 'mcp/servers', testid: 'mcp-sum-servers' });
    }
  }
  if (tools) {
    const on = tools.filter((t) => t.enabled);
    const changing = on.filter((t) => t.mutating).length;
    out.push({
      text: `Agents may use ${on.length} of Otto’s ${n(tools.length, 'tool')}; ${changing} of them can change things.`,
      link: 'Review tools',
      to: 'mcp/otto',
      testid: 'mcp-sum-tools',
    });
  }
  if (rules !== null) {
    out.push({
      text: !input.approvalsOn
        ? 'Approval prompts are turned off: calls that change things run without asking.'
        : rules === 0
          ? 'Every call that can change things asks a person first.'
          : `Calls that can change things ask a person first; ${n(rules, 'auto-approve rule')} let some run without asking.`,
      link: rules === 0 ? 'Add a rule' : 'Review rules',
      to: 'mcp/otto',
      testid: 'mcp-sum-rules',
    });
  }
  const denied = recentDenials(input.approvals, input.now);
  out.push({
    text: denied === 0 ? 'No calls were denied in the last 7 days.' : `${n(denied, 'call')} denied in the last 7 days.`,
    link: 'Open activity',
    to: 'mcp/activity',
    testid: 'mcp-sum-denials',
  });
  return out;
}
