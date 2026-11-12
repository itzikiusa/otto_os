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

/**
 * Who a section's settings apply to — one small label per section, so a
 * change's reach is never a surprise: this device only (browser storage /
 * the desktop shell), the signed-in user, the CURRENT workspace (the page
 * names it — WorkspaceScope), or everyone on this Otto (daemon-wide). Read
 * off each page's own save calls; `scopeNote` says when a page mixes two.
 */
export type SettingsScope = 'device' | 'me' | 'workspace' | 'everyone';

export const SCOPE_LABEL: Record<SettingsScope, string> = {
  device: 'Device',
  me: 'You',
  workspace: 'Workspace',
  everyone: 'Everyone',
};

/** The tooltip / accessible description behind each label. */
export const SCOPE_HINT: Record<SettingsScope, string> = {
  device: 'Saved on this device only',
  me: 'Yours only; other users have their own',
  workspace: 'Applies to the current workspace only',
  everyone: 'Applies to everyone using this Otto',
};

/** Who may open a section: a feature at a capability, or the root account. */
export type SettingsGate = { feature: Feature; level: Capability } | 'root';

/** The caller's access, as `auth` exposes it (kept structural for tests). */
export interface SettingsAccess {
  can(feature: Feature, level: Capability): boolean;
  isRoot: boolean;
}

const ADMIN: SettingsGate = { feature: 'settings', level: 'admin' };
const USERS_ADMIN: SettingsGate = { feature: 'users', level: 'admin' };
// Providers, Daemon, Trust & safety and Logs read/write root-only handlers
// (`GET|PUT /settings`, `/audit-log`, `/security-posture`, `/logs/daemon` all
// `require_root`), so a non-root settings admin only ever saw "Couldn’t load".

/**
 * Every section, in nav order. `id` is the route (`#/settings/<id>`) and must
 * never change (links and E2E use it); `label` is the ONE name shown in the
 * nav, the page title and ⌘K — sentence case, proper nouns (Jira, MCP) kept.
 * `gate` omitted = any signed-in member.
 */
export const SETTINGS_SECTIONS = [
  // ── General ──
  { id: 'appearance', label: 'Appearance', group: 'general', keywords: 'theme scheme dark light accent colour color backdrop wallpaper transparency direction rtl right-to-left terminal font size copy select floating bar isolate device close tab archive delete database vertical columns sidebar customize hide reorder favorites favourites star pin sections order', scope: 'device', settings: ['Theme', 'Scheme', 'Accent color', 'Backdrop', 'Transparency', 'Direction', 'Right-to-left text', 'Terminal font', 'Terminal font size', 'Copy on select', 'Floating bar', 'Sessions on this device', 'Closing a session tab', 'Database Explorer', 'Sidebar'] }, // UK search synonyms (colour, favourites); ui-guards: allow
  { id: 'session-names', label: 'Session names', group: 'general', keywords: 'theme auto name rename agent session', scope: 'me', settings: ['Active theme', 'Custom themes', 'Theme name'] },
  { id: 'notifications', label: 'Notifications', group: 'general', keywords: 'alerts credential expiry warnings native macos session finished waiting banner', scope: 'everyone', settings: ['Alerts', 'Channel notifications', 'Credential expiry', 'Warn before a credential expires'] },
  { id: 'snipping', label: 'Snipping', group: 'general', keywords: 'snip screenshot capture annotate clipboard shortcut', scope: 'device', settings: ['Snip shortcut', 'Capture shortcut'] },
  { id: 'browser', label: 'Browser', group: 'general', keywords: 'live tabs engine lightpanda web view reader', gate: { feature: 'browser', level: 'view' }, scope: 'device', scopeNote: 'The renderer is per device; the browser engine and downloads are daemon-wide.', settings: ['Live tabs on this device', 'Browser beside a session', 'Browser engine', 'Downloads from web pages'] },
  { id: 'tokens', label: 'Personal access tokens', group: 'general', keywords: 'api token pat key cli script ci', scope: 'me', settings: ['New token', 'Your tokens', 'Using a token'] },
  // ── Integrations ──
  { id: 'git-accounts', label: 'Git accounts', group: 'integrations', keywords: 'github gitlab bitbucket token pr push https credentials', scope: 'me', settings: ['GitHub token', 'GitLab token', 'Bitbucket token'] },
  { id: 'jira', label: 'Jira & Linear accounts', group: 'integrations', keywords: 'atlassian confluence linear issues tickets token api key', scope: 'me', settings: ['Email', 'Label', 'Token expiry'] },
  { id: 'channels', label: 'Slack, Telegram & webhooks', group: 'integrations', keywords: 'slack telegram webhook bridge inbound bot', scope: 'workspace', scopeNote: 'Slack act-as-me at the bottom is per user.', settings: ['Bot token', 'App token', 'Default channel ID', 'Default chat ID', 'Allowed users', 'Open to everyone', 'Preferred CLI', 'Reply instructions', 'Webhook key', 'Inbound URL', 'Act as me'] },
  { id: 'mcp-servers', label: 'MCP servers', group: 'integrations', keywords: 'model context protocol tools workspace', scope: 'workspace', settings: ['Connections MCP', 'Your servers', 'Command', 'Environment'] },
  { id: 'language-servers', label: 'Language servers', group: 'integrations', keywords: 'lsp completion gopls rust-analyzer typescript install path', scope: 'everyone', settings: ['Install missing servers', 'Install command'] },
  { id: 'phone-access', label: 'Phone access', group: 'integrations', keywords: 'tailscale tailnet phone iphone android mobile pair qr device continue remote serve', scope: 'everyone', scopeNote: 'Paired phones are per user; serving over Tailscale is for this Mac.', settings: ['Tailscale', 'Serving', 'Paired phones', 'Devices in your tailnet'] },
  { id: 'sharing', label: 'Sharing', group: 'integrations', keywords: 'email sender share guest one-time code otp gmail smtp email remote', scope: 'me', scopeNote: 'The public link domain applies to everyone.', settings: ['Gmail sender', 'Gmail address', 'App password', 'Public link domain'] },
  // ── Agents ──
  // Settings admins reach Update all CLIs (ws editor) and the model catalog
  // (Agents); the daemon-wide settings on the page stay root-only in the page.
  { id: 'providers', label: 'Providers', group: 'agents', keywords: 'agent cli claude codex agy gemini custom model update', gate: ADMIN, scope: 'everyone', settings: ['Default agent', 'Automatic updates', 'Update all CLIs', 'Models catalog', 'PR & commit draft model', 'Permissions', 'Custom provider', 'Command', 'Resume arguments', 'Model flag template'] },
  { id: 'context-soul', label: 'Workspace context', group: 'agents', keywords: 'soul goals instructions memory references project', scope: 'workspace', settings: ['Goal', 'Shared instructions', 'Workspace memory', 'References', 'Project context', 'Decisions', 'Artifacts', 'Soul', 'Materialize now'] },
  { id: 'self-improvement', label: 'Self-improvement', group: 'agents', keywords: 'improve review memory skills edits approval', scope: 'workspace', settings: ['Schedule', 'Run every (minutes)', 'Look back (hours)', 'Autonomy', 'What may change', 'Skill allow-list', 'Pending approvals', 'Recent runs'] },
  { id: 'insights', label: 'Insights', group: 'agents', keywords: 'reports daily weekly monthly schedule html', scope: 'everyone', settings: ['Report agent', 'Schedule', 'Provider'] },
  { id: 'skills', label: 'Skills', group: 'agents', keywords: 'bundled skill library install claude codex agy', gate: ADMIN, scope: 'everyone', settings: ['Install bundled skills', 'Install all'] },
  { id: 'skill-eval', label: 'Skills evaluator', group: 'agents', keywords: 'skill eval validate improve iterations lab', gate: ADMIN, scope: 'everyone', settings: ['Improver agent', 'Iterations', 'Validation passes', 'Default validations'] },
  { id: 'context-library', label: 'Context library', group: 'agents', keywords: 'skills souls snippets library materialize', gate: ADMIN, scope: 'everyone', settings: ['Global default soul', 'Skills', 'Souls', 'Snippets'] },
  // ── System ──
  { id: 'daemon', label: 'Daemon', group: 'system', keywords: 'ottod server port listener network sandbox isolation sessions restart persist suspend idle timeout', gate: 'root', scope: 'everyone', settings: ['Network listener', 'Port', 'Process sandbox', 'Run unconfined', 'Keep sessions running', 'Suspend idle sessions', 'Archive idle sessions', 'Auto-archive', 'Log folder', 'Restart daemon'] },
  // Every /plugin-admin* handler is require_root (plugins.rs) — S17-303.
  { id: 'plugins', label: 'Plugins', group: 'system', keywords: 'custom plugin sidecar install git', gate: 'root', scope: 'everyone', settings: ['Install plugin', 'Enable plugin'] },
  { id: 'trust-safety', label: 'Trust & safety', group: 'system', keywords: 'security posture audit log trust secrets encrypt encryption keychain plaintext', gate: 'root', scope: 'everyone', settings: ['Secret storage', 'Secure secrets', 'Encrypt secrets', 'Security posture', 'Audit log', 'Active API tokens'] },
  { id: 'logs', label: 'Logs', group: 'system', keywords: 'daemon log files debug errors', gate: 'root', scope: 'everyone', settings: ['Log file', 'Lines', 'Filter', 'Wrap long lines', 'Live'] },
  // Every Backup action is root-only server-side: /settings/export|import,
  // /state/backup|restore|archive* (routes/backup.rs), /state/git/*
  // (backup_git.rs), /state/connections/export* (connection_export.rs) — S17-303.
  { id: 'backup', label: 'Backup & restore', group: 'system', keywords: 'export import archive restore transfer connections git backup storage design hall auto-tidy autosave prune', gate: 'root', scope: 'everyone', settings: ['Settings only', 'Full backup', 'Git backup', 'Export connections', 'Restore', 'Design storage', 'Compact database'] },
  // ── People ──
  { id: 'users', label: 'Users', group: 'people', keywords: 'accounts members roles workspace permissions', gate: USERS_ADMIN, scope: 'everyone', settings: ['Accounts', 'Username', 'Display name', 'Password', 'Workspace roles', 'Feature grants'] },
  { id: 'access-groups', label: 'Groups & access', group: 'people', keywords: 'groups roles access rules rbac permissions', gate: 'root', scope: 'everyone', settings: ['Group name', 'Add user', 'Resource type', 'Presets'] },
  { id: 'sessions', label: 'Sessions', group: 'people', keywords: 'all users sessions admin overview', gate: USERS_ADMIN, scope: 'everyone', settings: ['All sessions', 'Suspend session', 'Archive session'] },
] as const satisfies readonly {
  id: string;
  label: string;
  group: SettingsGroupId;
  keywords?: string;
  gate?: SettingsGate;
  scope: SettingsScope;
  scopeNote?: string;
  /** The page's visible setting labels, so search finds "idle timeout". */
  settings: readonly string[];
}[];

export type SettingsSectionId = (typeof SETTINGS_SECTIONS)[number]['id'];

export interface SettingsSection {
  id: SettingsSectionId;
  label: string;
  group: SettingsGroupId;
  keywords?: string;
  gate?: SettingsGate;
  scope: SettingsScope;
  scopeNote?: string;
  settings: readonly string[];
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

/** The section's scope as one line: "Workspace — Applies to the current…". */
export function scopeDescription(section: SettingsSection): string {
  const base = `${SCOPE_LABEL[section.scope]}: ${SCOPE_HINT[section.scope]}.`;
  return section.scopeNote ? `${base} ${section.scopeNote}` : base;
}

/**
 * The individual setting a query found, when it isn't the section's own
 * name: the first setting label holding every term. Shown under the section
 * in the filtered list ("Daemon · Archive idle sessions").
 */
export function matchedSetting(section: SettingsSection, query: string): string | null {
  const q = query.trim().toLowerCase();
  if (!q || section.label.toLowerCase().includes(q)) return null;
  const terms = q.split(/\s+/);
  return section.settings.find((l) => terms.every((t) => l.toLowerCase().includes(t))) ?? null;
}

/**
 * Filter sections by a free-text query: every whitespace-separated term must
 * appear (case-insensitive) in the label, keywords, setting labels, group
 * label or id. Results keep nav order, except that sections whose LABEL starts
 * with the query (then contains it) rank first, then those with a SETTING
 * label holding the query — so Enter on "jira" opens Jira accounts, and
 * "idle timeout" opens the page that has that setting, not one that merely
 * mentions a word in its keywords. Empty query = all.
 */
export function filterSections(sections: readonly SettingsSection[], query: string): SettingsSection[] {
  const q = query.trim().toLowerCase();
  if (!q) return [...sections];
  const terms = q.split(/\s+/);
  const scored: { s: SettingsSection; score: number; i: number }[] = [];
  sections.forEach((s, i) => {
    const label = s.label.toLowerCase();
    const settings = s.settings.join(' ').toLowerCase();
    const hay = `${label} ${s.keywords ?? ''} ${settings} ${groupLabel(s.group)} ${s.id.replace(/-/g, ' ')}`.toLowerCase();
    if (!terms.every((t) => hay.includes(t))) return;
    const score = label.startsWith(q) ? 0 : label.includes(q) ? 1 : matchedSetting(s, q) ? 2 : 3;
    scored.push({ s, score, i });
  });
  return scored.sort((a, b) => a.score - b.score || a.i - b.i).map((x) => x.s);
}
