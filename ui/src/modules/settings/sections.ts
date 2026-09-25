// Single source of truth for the Settings sections, shared by the Settings
// nav (grouped list + filter), each section's PageHeader title, and the ⌘K
// "Settings: <section>" commands (App.svelte). Mirrors lib/sidebar.ts: pure
// data + helpers so the unit tests can exercise them without Svelte/auth
// state — callers supply the access predicate. The section components
// themselves are mapped in Settings.svelte (this file must stay importable
// from plain Node).

import type { Capability, Feature } from '../../lib/api/types';

/** Nav groups, in display order (macOS source-list style). */
export type SettingsGroupId = 'general' | 'integrations' | 'agents' | 'system' | 'people';

export const SETTINGS_GROUPS: { id: SettingsGroupId; label: string }[] = [
  { id: 'general', label: 'General' },
  { id: 'integrations', label: 'Integrations' },
  { id: 'agents', label: 'Agents' },
  { id: 'system', label: 'System' },
  { id: 'people', label: 'People' },
];

/** Who may open a section: a feature at a capability, or the root account. */
export type SettingsGate = { feature: Feature; level: Capability } | 'root';

/** The caller's access, as `auth` exposes it (kept structural for tests). */
export interface SettingsAccess {
  can(feature: Feature, level: Capability): boolean;
  isRoot: boolean;
}

const ADMIN: SettingsGate = { feature: 'settings', level: 'admin' };
const USERS_ADMIN: SettingsGate = { feature: 'users', level: 'admin' };

/**
 * Every section, in nav order. `id` is the route (`#/settings/<id>`) and must
 * never change (links and E2E use it); `label` is the ONE name shown in the
 * nav, the page title and ⌘K — sentence case, proper nouns (Jira, MCP) kept.
 * `gate` omitted = any signed-in member.
 */
export const SETTINGS_SECTIONS = [
  // ── General ──
  { id: 'appearance', label: 'Appearance', group: 'general', keywords: 'theme scheme dark light accent colour color backdrop wallpaper transparency direction rtl right-to-left terminal font size copy select floating bar isolate device close tab archive delete database vertical columns sidebar customize hide reorder' },
  { id: 'session-names', label: 'Session names', group: 'general', keywords: 'theme auto name rename agent session' },
  { id: 'notifications', label: 'Notifications', group: 'general', keywords: 'alerts credential expiry warnings native macos session finished waiting banner' },
  { id: 'snipping', label: 'Snipping', group: 'general', keywords: 'snip screenshot capture annotate clipboard shortcut' },
  { id: 'browser', label: 'Browser', group: 'general', keywords: 'live tabs engine lightpanda web view reader', gate: { feature: 'browser', level: 'view' } },
  { id: 'tokens', label: 'Personal access tokens', group: 'general', keywords: 'api token pat key secret cli script ci' },
  // ── Integrations ──
  { id: 'git-accounts', label: 'Git accounts', group: 'integrations', keywords: 'github gitlab bitbucket token pr push https credentials' },
  { id: 'jira', label: 'Jira accounts', group: 'integrations', keywords: 'atlassian confluence issues tickets token' },
  { id: 'channels', label: 'Channels', group: 'integrations', keywords: 'slack telegram webhook bridge inbound bot' },
  { id: 'mcp-servers', label: 'MCP servers', group: 'integrations', keywords: 'model context protocol tools workspace' },
  { id: 'language-servers', label: 'Language servers', group: 'integrations', keywords: 'lsp completion gopls rust-analyzer typescript install path' },
  { id: 'sharing', label: 'Sharing', group: 'integrations', keywords: 'email sender share guest one-time code otp gmail smtp email remote' },
  // ── Agents ──
  { id: 'assistant', label: 'Assistant', group: 'agents', keywords: 'otto assistant routing claude codex subscription limit' },
  { id: 'providers', label: 'Providers', group: 'agents', keywords: 'agent cli claude codex agy gemini custom model update', gate: ADMIN },
  { id: 'context-soul', label: 'Workspace context', group: 'agents', keywords: 'soul goals instructions memory references project' },
  { id: 'self-improvement', label: 'Self-improvement', group: 'agents', keywords: 'improve review memory skills edits approval' },
  { id: 'insights', label: 'Insights', group: 'agents', keywords: 'reports daily weekly monthly schedule html' },
  { id: 'skills', label: 'Skills', group: 'agents', keywords: 'bundled skill library install claude codex agy', gate: ADMIN },
  { id: 'skill-eval', label: 'Skills evaluator', group: 'agents', keywords: 'skill eval validate improve iterations lab', gate: ADMIN },
  { id: 'context-library', label: 'Context library', group: 'agents', keywords: 'skills souls snippets library materialize', gate: ADMIN },
  // ── System ──
  { id: 'daemon', label: 'Daemon', group: 'system', keywords: 'ottod server port listener network sandbox isolation', gate: ADMIN },
  { id: 'plugins', label: 'Plugins', group: 'system', keywords: 'custom plugin sidecar install git', gate: ADMIN },
  { id: 'trust-safety', label: 'Trust & safety', group: 'system', keywords: 'security posture audit log trust', gate: ADMIN },
  { id: 'logs', label: 'Logs', group: 'system', keywords: 'daemon log files debug errors', gate: ADMIN },
  { id: 'backup', label: 'Backup & restore', group: 'system', keywords: 'export import archive restore transfer connections git backup', gate: ADMIN },
  // ── People ──
  { id: 'users', label: 'Users', group: 'people', keywords: 'accounts members roles workspace permissions', gate: USERS_ADMIN },
  { id: 'access-groups', label: 'Groups & access', group: 'people', keywords: 'groups roles access rules rbac permissions', gate: 'root' },
  { id: 'sessions', label: 'Sessions', group: 'people', keywords: 'all users sessions admin overview', gate: USERS_ADMIN },
] as const satisfies readonly {
  id: string;
  label: string;
  group: SettingsGroupId;
  keywords?: string;
  gate?: SettingsGate;
}[];

export type SettingsSectionId = (typeof SETTINGS_SECTIONS)[number]['id'];

export interface SettingsSection {
  id: SettingsSectionId;
  label: string;
  group: SettingsGroupId;
  keywords?: string;
  gate?: SettingsGate;
}

const SECTIONS: readonly SettingsSection[] = SETTINGS_SECTIONS;

/** The section a route id names, or undefined for an unknown id. */
export function findSection(id: string): SettingsSection | undefined {
  return SECTIONS.find((s) => s.id === id);
}

/** The display label of a section — what its PageHeader title renders. */
export function sectionLabel(id: SettingsSectionId): string {
  return findSection(id)?.label ?? id;
}

export function groupLabel(id: SettingsGroupId): string {
  return SETTINGS_GROUPS.find((g) => g.id === id)?.label ?? id;
}

/** Whether `access` may open `section` (ungated sections always pass). */
export function canOpenSection(section: SettingsSection, access: SettingsAccess): boolean {
  const g = section.gate;
  if (g == null) return true;
  if (g === 'root') return access.isRoot;
  return access.can(g.feature, g.level);
}

/** The sections `access` may open, in nav order. */
export function availableSections(access: SettingsAccess): SettingsSection[] {
  return SECTIONS.filter((s) => canOpenSection(s, access));
}

/**
 * Filter sections by a free-text query: every whitespace-separated term must
 * appear (case-insensitive) in the label, keywords, group label or id. Results
 * keep nav order, except that sections whose LABEL starts with the query (then
 * contains it) rank first — so Enter on "jira" opens Jira accounts, not a
 * section that merely mentions Jira in its keywords. Empty query = all.
 */
export function filterSections(sections: readonly SettingsSection[], query: string): SettingsSection[] {
  const q = query.trim().toLowerCase();
  if (!q) return [...sections];
  const terms = q.split(/\s+/);
  const scored: { s: SettingsSection; score: number; i: number }[] = [];
  sections.forEach((s, i) => {
    const label = s.label.toLowerCase();
    const hay = `${label} ${s.keywords ?? ''} ${groupLabel(s.group)} ${s.id.replace(/-/g, ' ')}`.toLowerCase();
    if (!terms.every((t) => hay.includes(t))) return;
    const score = label.startsWith(q) ? 0 : label.includes(q) ? 1 : 2;
    scored.push({ s, score, i });
  });
  return scored.sort((a, b) => a.score - b.score || a.i - b.i).map((x) => x.s);
}
