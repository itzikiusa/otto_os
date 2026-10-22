// Contract mirror: integrations. Part of lib/api/types (see ../types.ts, the barrel).
// Mirrors docs/contracts/api.md + ws.md — change them together.

import type {
  Id,
} from './core';

// ---------------------------------------------------------------------------
// Issue tracking (Jira, Linear)
// ---------------------------------------------------------------------------

/** The tracker an issue account belongs to — it decides where searches, fetches
 *  and comments go (Jira REST or the Linear GraphQL API). */
export type IssueProviderKind = 'jira' | 'linear';

export interface IssueAccount {
  id: Id;
  user_id: Id;
  provider: IssueProviderKind;
  label: string;
  email: string;
  base_url: string;
  /** User-entered token expiry; null = unknown/none. */
  token_expires_at: string | null;
  created_at: string;
}

export interface IssueProject {
  key: string;
  name: string;
}

/** `GET /issue/projects?meta=1` / `/issue/confluence/spaces?meta=1`: the
 *  listing plus whether it stopped at its page cap (later rows not shown). */
export interface ListingPage<T> {
  items: T[];
  truncated: boolean;
}

export interface IssueSummary {
  key: string;
  summary: string;
  status: string;
  issue_type: string;
  url: string;
  /** Present only when Jira has a value (raw-JQL callers group by these). */
  parent_key?: string;
  assignee?: string;
  fix_versions?: string[];
  updated?: string;
}

/** `GET /issue/my-work?account_id=` — one of the caller's assigned issues
 *  (Focus view), with parent + project context for hierarchy rebuilding. */
export interface MyWorkIssue {
  key: string;
  summary: string;
  status: string;
  issue_type: string;
  url: string;
  project_key: string;
  project_name: string;
  parent_key: string | null;
  parent_summary: string | null;
  parent_type: string | null;
  priority: string | null;
  updated_at: string | null;
}

export interface IssueDetail {
  key: string;
  summary: string;
  status: string;
  issue_type: string;
  url: string;
  description: string | null;
  assignee: string | null;
}

/** `session.meta.issue` — client-owned (no Rust struct: the daemon stores the JSON verbatim).
 *  `url` / `status` are copied from the picked `IssueSummary`; sessions attached before that
 *  carry `''`, so renderers treat an empty `url` as "no link" and an empty `status` as "no chip". */
export interface AttachedIssue {
  provider: IssueProviderKind;
  account_id: Id;
  key: string;
  summary: string;
  url: string;
  status: string;
}

// ---------------------------------------------------------------------------
// Integrations (Slack / Telegram)
// ---------------------------------------------------------------------------

export type Channel = 'slack' | 'telegram' | 'webhook';

/** Public-by-key POST /webhooks/{workspace_id}. Overload returns a 429
 * `{code: 'busy', message}` problem before any work starts; retry later. */
export interface WebhookInboundReq {
  text: string;
  conversation?: string;
  thread?: string;
  user?: string;
  callback_url?: string;
}
export interface WebhookInboundResp {
  accepted: boolean;
  conversation: string;
}


/** `GET /workspaces/{id}/integrations/slack/directory` — one Slack member. */
export interface SlackDirUser {
  id: string;
  /** Display name, else real name, else the handle. */
  name: string;
  handle: string;
  is_bot: boolean;
}

/** One Slack channel the bot can see. */
export interface SlackDirChannel {
  id: string;
  name: string;
  is_private: boolean;
}

/** `GET /workspaces/{id}/integrations/slack/directory[?refresh=1]` — names for
 *  the Slack card's ids. A part that Slack refused carries its `*_error`
 *  (e.g. the missing scope) and comes back empty. */
export interface SlackDirectory {
  users: SlackDirUser[];
  channels: SlackDirChannel[];
  users_error?: string;
  channels_error?: string;
  truncated: boolean;
}

export interface Integration {
  workspace_id: string;
  channel: Channel;
  enabled: boolean;
  allowed_users: string;     // comma-separated
  /** Explicit opt-in: a BLANK `allowed_users` admits every sender. Off ⇒ a
   *  blank list admits nobody (fail closed). Webhooks ignore it. */
  open_to_all: boolean;
  agent_reply: boolean;
  reply_instructions: string;
  channel_id: string;
  preferred_cli: string;       // '' = use the default agent
  has_bot_token: boolean;
  has_app_token: boolean;
  updated_at: string;
}

/** Live state of one Slack / Telegram inbound listener
 *  (`GET /workspaces/{id}/integrations/status`). In-memory on the daemon:
 *  only enabled, listener-backed integrations have an entry. */
export type ListenerState =
  | 'waiting_for_token'
  | 'connecting'
  | 'connected'
  | 'reconnecting'
  | 'failing'
  | 'conflict';

export interface ListenerStatus {
  workspace_id: string;
  channel: Channel;
  state: ListenerState;
  /** User-facing reason for `state` (an error, what's missing). */
  detail?: string;
  /** When the listener entered `state`. */
  since: string;
  connected_at?: string;
  /** Last inbound event the listener received. */
  last_event_at?: string;
  last_error?: string;
  last_error_at?: string;
  /** Consecutive failed attempts since the last good connection. */
  failures: number;
  /** Last senders the allow-list dropped, newest first (omitted when none). */
  rejected_senders?: RejectedSender[];
}

/** A sender a channel's allow-list turned away (`ListenerStatus.rejected_senders`). */
export interface RejectedSender {
  /** Channel-native user id — what goes into `allowed_users`. */
  user: string;
  /** @handle / display name when the platform sent one (display only). */
  name?: string;
  at: string;
}

export interface UpsertIntegrationReq {
  enabled: boolean;
  bot_token?: string | null;   // omit/null to keep existing
  app_token?: string | null;   // slack only
  allowed_users: string;
  /** Omit to keep the stored value. */
  open_to_all?: boolean;
  agent_reply: boolean;
  reply_instructions: string;
  channel_id: string;
  preferred_cli: string;       // '' = use the default agent
}
