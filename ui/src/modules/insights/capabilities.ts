// Capability & health registry API helpers (B3).
// Module-local types — not added to ui/src/lib/api/types.ts per task boundary.

import { api } from '../../lib/api/client';
import { sentenceCase } from '../../lib/labels';

// ---------------------------------------------------------------------------
// DTO types (mirror crates/otto-server/src/routes/capabilities.rs)
// ---------------------------------------------------------------------------

/** One dependency a module needs to function. */
export interface CapabilityDep {
  /** "provider" | "cli" | "lsp" | "mcp" | "channel" | "git" | "issue" | "db" | "broker" */
  kind: string;
  /** Human-readable name. */
  name: string;
  /** Whether the dep is usable. */
  ok: boolean;
  /** Optional detail — resolved path, account provider, connection kind, … */
  detail: string | null;
}

/** "ready" | "degraded" | "missing_setup" */
export type CapabilityStatus = 'ready' | 'degraded' | 'missing_setup';

/** Capability snapshot for one Otto module. */
export interface ModuleCapability {
  /** Feature slug, e.g. "sessions", "git", "channels". */
  feature: string;
  status: CapabilityStatus;
  reasons: string[];
  fixes: string[];
  deps: CapabilityDep[];
}

/** Support bundle returned by GET /support-bundle. */
export interface SupportBundle {
  version: string;
  /** Settings with secrets replaced by "[redacted]". */
  settings: Record<string, unknown>;
  capabilities: ModuleCapability[];
  recent_audit: Record<string, unknown>[];
  migration_level: number;
  redaction_hits: number;
}

// ---------------------------------------------------------------------------
// API helpers
// ---------------------------------------------------------------------------

export const capabilitiesApi = {
  /** GET /api/v1/capabilities — full health registry (root only). */
  list: (): Promise<ModuleCapability[]> => api.get<ModuleCapability[]>('/capabilities'),

  /** GET /api/v1/support-bundle — downloadable bundle (root only). */
  bundle: (): Promise<SupportBundle> => api.get<SupportBundle>('/support-bundle'),
};

// ---------------------------------------------------------------------------
// Display helpers
// ---------------------------------------------------------------------------

/** Label shown in the UI for a status value. */
export function statusLabel(s: CapabilityStatus): string {
  switch (s) {
    case 'ready': return 'Ready';
    case 'degraded': return 'Degraded';
    case 'missing_setup': return 'Not set up';
  }
}

/** CSS class suffix used for status badges and status-dot colours. */
export function statusClass(s: CapabilityStatus): string {
  switch (s) {
    case 'ready': return 'green';
    case 'degraded': return 'yellow';
    case 'missing_setup': return 'gray';
  }
}

/** Feature slug → human-readable label (sentence case). */
export function featureLabel(slug: string): string {
  const map: Record<string, string> = {
    sessions: 'Agent sessions',
    lsp: 'Language servers (LSP)',
    mcp: 'MCP servers',
    channels: 'Channels (Slack / Telegram)',
    git: 'Git accounts',
    issues: 'Issue trackers (Jira)',
    db: 'Database connections',
    brokers: 'Message brokers (Kafka)',
  };
  return map[slug] ?? sentenceCase(slug);
}

/** Dependency kind (`CapabilityDep.kind`) → the word a person reads. */
export function depKindLabel(kind: string): string {
  const map: Record<string, string> = {
    provider: 'Provider',
    cli: 'CLI',
    lsp: 'Language server',
    mcp: 'MCP server',
    channel: 'Channel',
    git: 'Git account',
    issue: 'Issue tracker',
    db: 'Database',
    broker: 'Broker',
  };
  return map[kind] ?? sentenceCase(kind);
}

/** Settings route the UI should navigate to for a feature slug. */
export function settingsRoute(slug: string): string {
  const map: Record<string, string> = {
    sessions: 'settings/providers',
    lsp: 'settings/language-servers',
    mcp: 'settings/mcp-servers',
    channels: 'settings/channels',
    git: 'settings/git-accounts',
    issues: 'settings/jira',
    db: 'connections',
    brokers: 'brokers',
  };
  return map[slug] ?? 'settings';
}
