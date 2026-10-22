// MCP profiles — the pure logic behind the profile editor (MCP → Profiles),
// the per-session "MCP tools" picker and the per-profile connection configs.
// Mirrors `otto_mcp::profiles` (canonical names, group membership, the union
// filter); the daemon stays authoritative — this only drives the UI.
import type { McpProfile, McpProfileReq, McpToolGroup } from './api/types';
import { plural } from './plural.ts';

/** `otto.create_pr` / `otto_create_pr` / `create_pr` → `create_pr`. */
export function canonical(name: string): string {
  const n = name.startsWith('otto.') ? name.slice(5) : name;
  return n.startsWith('otto_') ? n.slice(5) : n;
}

/** The group a tool belongs to (exact names first, then prefix families). */
export function groupOf(groups: McpToolGroup[], tool: string): McpToolGroup | undefined {
  const key = canonical(tool);
  return (
    groups.find((g) => g.tools.includes(key)) ?? groups.find((g) => g.prefixes.some((p) => key.startsWith(p)))
  );
}

type ProfileTools = Pick<McpProfile, 'all_tools' | 'groups' | 'include_tools' | 'exclude_tools'>;

/** Whether one profile lets `tool` through — `ProfileDef::allows`. */
export function profileAllows(groups: McpToolGroup[], p: ProfileTools, tool: string): boolean {
  const key = canonical(tool);
  const has = (list: string[]) => list.some((t) => canonical(t) === key);
  if (has(p.exclude_tools)) return false;
  if (p.all_tools || has(p.include_tools)) return true;
  const g = groupOf(groups, key);
  return g !== undefined && p.groups.includes(g.id);
}

/** The listed tools a profile shows (prefix-only families not counted). */
export function profileTools(groups: McpToolGroup[], p: ProfileTools): string[] {
  const all = [...new Set(groups.flatMap((g) => g.tools).concat(p.include_tools.map(canonical)))];
  return all.filter((t) => profileAllows(groups, p, t));
}

/** One line under a profile's name: "All tools" / "Database, Confluence · 12 tools". */
export function profileSummary(groups: McpToolGroup[], p: McpProfile): string {
  if (p.all_tools) {
    return p.exclude_tools.length ? `All tools except ${p.exclude_tools.length}` : 'All tools';
  }
  const labels = p.groups.map((id) => groups.find((g) => g.id === id)?.label ?? id);
  const extra = p.include_tools.length ? [`+${plural(p.include_tools.length, 'tool')}`] : [];
  const head = [...labels, ...extra].join(', ') || 'No groups';
  return `${head} · ${plural(profileTools(groups, p).length, 'tool')}`;
}

// ── The session picker ──────────────────────────────────────────────────────

/** A session's stored profile ids (`meta.mcp_profiles`); [] ⇒ the default. */
export function sessionProfileIds(meta: Record<string, unknown> | null | undefined): string[] {
  const v = meta?.mcp_profiles;
  return Array.isArray(v) ? v.filter((x): x is string => typeof x === 'string' && x.trim() !== '') : [];
}

/** Multi-select toggle. Never empty: clearing the last one falls back to the
 *  default (the union of nothing would be meaningless). */
export function toggleProfile(ids: string[], id: string, defaultId: string): string[] {
  const next = ids.includes(id) ? ids.filter((x) => x !== id) : [...ids, id];
  return next.length ? next : [defaultId];
}

/** The ids a session should store: [] when the pick is exactly the default
 *  (so it follows a later default change, like sessions created before
 *  profiles existed), else the pick with unknown ids dropped. */
export function storedProfileIds(ids: string[], profiles: McpProfile[], defaultId: string): string[] {
  const known = ids.filter((id) => profiles.some((p) => p.id === id));
  return known.length === 0 || (known.length === 1 && known[0] === defaultId) ? [] : known;
}

/** What a session runs under, spelled out: "Database + Issues", or
 *  "Full (default)" when it stores nothing. */
export function selectionLabel(ids: string[], profiles: McpProfile[], defaultId: string): string {
  const named = ids.map((id) => profiles.find((p) => p.id === id)?.name).filter((n): n is string => !!n);
  if (named.length) return named.join(' + ');
  const d = profiles.find((p) => p.id === defaultId);
  return d ? `${d.name} (default)` : 'Default';
}

/** A note when the pick is redundant: Full already contains every tool. */
export function selectionHint(ids: string[], profiles: McpProfile[]): string {
  const full = profiles.find((p) => ids.includes(p.id) && p.all_tools && p.exclude_tools.length === 0);
  return full && ids.length > 1 ? `${full.name} already includes every tool.` : '';
}

// ── The editor ──────────────────────────────────────────────────────────────

export interface ProfileDraft {
  name: string;
  description: string;
  all_tools: boolean;
  groups: string[];
  include_tools: string[];
  exclude_tools: string[];
}

export function emptyDraft(): ProfileDraft {
  return { name: '', description: '', all_tools: false, groups: [], include_tools: [], exclude_tools: [] };
}

export function draftOf(p: McpProfile): ProfileDraft {
  return {
    name: p.name,
    description: p.description,
    all_tools: p.all_tools,
    groups: [...p.groups],
    include_tools: [...p.include_tools],
    exclude_tools: [...p.exclude_tools],
  };
}

/** "Database copy", then "Database copy 2", … — unique among `profiles`. */
export function duplicateName(name: string, profiles: McpProfile[]): string {
  const taken = new Set(profiles.map((p) => p.name.toLowerCase()));
  const base = `${name} copy`;
  if (!taken.has(base.toLowerCase())) return base;
  for (let i = 2; ; i++) if (!taken.has(`${base} ${i}`.toLowerCase())) return `${base} ${i}`;
}

/** The inline error for a draft, or '' when it can be saved. */
export function validateDraft(d: ProfileDraft, profiles: McpProfile[], editingId: string | null): string {
  const name = d.name.trim();
  if (!name) return 'Give the profile a name.';
  if ([...name].length > 60) return 'Keep the name under 60 characters.';
  if (profiles.some((p) => p.id !== editingId && p.name.toLowerCase() === name.toLowerCase())) {
    return `A profile named “${name}” already exists.`;
  }
  if (!d.all_tools && d.groups.length === 0 && d.include_tools.length === 0) {
    return 'Pick at least one tool group or tool.';
  }
  return '';
}

/** Toggle a group in or out. */
export function toggleGroup(d: ProfileDraft, id: string): ProfileDraft {
  return { ...d, groups: d.groups.includes(id) ? d.groups.filter((g) => g !== id) : [...d.groups, id] };
}

/** Flip one tool against what its groups give it: a tool the groups include
 *  becomes an exclude (or drops one), a tool they leave out becomes an
 *  include (or drops one). */
export function toggleTool(groups: McpToolGroup[], d: ProfileDraft, tool: string): ProfileDraft {
  const key = canonical(tool);
  const strip = (list: string[]) => list.filter((t) => canonical(t) !== key);
  const base = { ...d, include_tools: strip(d.include_tools), exclude_tools: strip(d.exclude_tools) };
  const byGroups = profileAllows(groups, base, key);
  const now = profileAllows(groups, d, key);
  if (now === byGroups) {
    return byGroups ? { ...base, exclude_tools: [...base.exclude_tools, key] } : { ...base, include_tools: [...base.include_tools, key] };
  }
  return base;
}

export function draftToReq(d: ProfileDraft): McpProfileReq {
  return {
    name: d.name.trim(),
    description: d.description.trim(),
    all_tools: d.all_tools,
    groups: d.groups,
    include_tools: d.include_tools,
    exclude_tools: d.exclude_tools,
  };
}

// ── External connections ────────────────────────────────────────────────────

/** The copyable configs attaching an outside Claude Code / Codex as one
 *  profile ("Otto · Database"). The Full profile keeps the plain all-tools
 *  connection (no profile parameter) so existing installs read the same. */
export function connectionConfigs(p: Pick<McpProfile, 'id' | 'name' | 'all_tools' | 'exclude_tools' | 'locked'>, httpBase: string) {
  const plain = p.locked && p.all_tools && p.exclude_tools.length === 0;
  const key = plain ? 'otto' : `otto-${p.id.replace(/[^a-zA-Z0-9_-]/g, '').toLowerCase()}`;
  const args = plain ? ['mcp-server'] : ['mcp-server', '--profile', p.id];
  const httpUrl = `${httpBase.replace(/\/+$/, '')}/api/v1/mcp/http${plain ? '' : `?profile=${encodeURIComponent(p.id)}`}`;
  const stdioJson = JSON.stringify(
    { mcpServers: { [key]: { command: 'ottod', args, env: { OTTO_API_TOKEN: 'YOUR_OTTO_MCP_TOKEN' } } } },
    null,
    2,
  );
  const claudeHttp = `claude mcp add --transport http ${key} "${httpUrl}" --header "Authorization: Bearer YOUR_OTTO_MCP_TOKEN"`;
  const codexToml = [
    `[mcp_servers.${key.replace(/-/g, '_')}]`,
    `command = "ottod"`,
    `args = [${args.map((a) => JSON.stringify(a)).join(', ')}]`,
    `env = { OTTO_API_TOKEN = "YOUR_OTTO_MCP_TOKEN" }`,
  ].join('\n');
  return { key, label: plain ? 'Otto' : `Otto · ${p.name}`, stdioJson, httpUrl, claudeHttp, codexToml };
}
