// Session-list bucketing + the shown-list query — the PURE half (no runes) so
// `node --test` can exercise it; the workspace store derives every sidebar
// list from ONE `bucketSessions` pass instead of ~12 separate O(n) filters.

import type { Session } from '../api/types';

/** = `SCRATCH_WORKSPACE_ID` (sessionScope.ts) — inlined so this module has no
 *  runtime imports (`node --test` loads it directly; the unit test pins the
 *  two equal). */
export const SCRATCH_ID = 'scratch';

/** Background-spawned session sources that never surface in the sidebar's flat
 *  session lists (they live in their own panels/views). MUST stay byte-identical
 *  to the Rust source of truth: `BACKGROUND_SESSION_SOURCES` in
 *  `crates/otto-core/src/domain.rs` (which also drives server-side durability
 *  and the `foreground=true` list filter). Every derived list filters through
 *  `isForeground` — never re-inline a source blacklist. */
export const BACKGROUND_SOURCES = new Set([
  'channel',
  'review',
  'review_summarizer',
  'skilleval',
  'skillreview',
  'product-analysis',
  'product_refine',
  'swarm',
  'canvas_assist',
  'canvas_assist_preview',
  'mockup_assist',
  'db_assist',
  'workflow',
  'vault-docs',
  'vault-docs-review',
  'pr-draft',
  'commit-draft',
  'insights',
  'run_with_otto',
  'goal_loop',
  'discovery_chat',
  'scheduled_task',
  'finding',
  'assistant',
  'design_assist',
  'browser_summarize',
]);

/** A user-facing foreground session (sidebar-listable). */
export function isForeground(s: Session): boolean {
  const src = (s.meta as { source?: unknown } | null)?.source;
  return typeof src !== 'string' || !BACKGROUND_SOURCES.has(src);
}

/** Whether the sidebar lists `s` (when live): a connection, a foreground
 *  agent, or a Slack/Telegram channel ticket — the client twin of the
 *  daemon's `foreground=true&with_sources=channel` filter. */
export function isShownKind(s: Session): boolean {
  if (s.kind !== 'agent' || isForeground(s)) return true;
  return (s.meta as { source?: unknown } | null)?.source === 'channel';
}

/** The `GET /workspaces/{id}/sessions` query for the main list: live rows the
 *  sidebar shows (+ any extra background sources a mounted panel asked for). */
export function shownListQuery(extraSources: Iterable<string> = []): string {
  const srcs = ['channel', ...[...extraSources].filter((s) => s !== 'channel')].sort((a, b) =>
    a === 'channel' ? -1 : b === 'channel' ? 1 : a.localeCompare(b),
  );
  return `?archived=false&foreground=true&with_sources=${srcs.map(encodeURIComponent).join(',')}`;
}

/** Max ids per `?ids=` request (the daemon's cap). */
export const IDS_CHUNK = 64;

/** `ids` split into `?ids=` chunks the daemon accepts. */
export function idChunks(ids: Iterable<string>): string[][] {
  const out: string[][] = [];
  let cur: string[] = [];
  for (const id of ids) {
    cur.push(id);
    if (cur.length === IDS_CHUNK) {
      out.push(cur);
      cur = [];
    }
  }
  if (cur.length > 0) out.push(cur);
  return out;
}

const byRecent = (a: Session, b: Session): number => b.last_active_at.localeCompare(a.last_active_at);

export interface SessionBuckets {
  /** Non-archived rows. */
  active: Session[];
  /** Non-archived, foreground (sidebar Agents / counts scope). */
  foregroundActive: Session[];
  /** Non-archived agents of the current workspace (not scratch). */
  agent: Session[];
  /** Foreground agents of the scratch workspace ("No workspace" group). */
  scratch: Session[];
  /** Non-archived connection sessions. */
  connection: Session[];
  /** Foreground agents of the current workspace (sidebar "Agents"). */
  plainAgent: Session[];
  /** Telegram / Slack channel agents, newest first. */
  telegram: Session[];
  slack: Session[];
}

/** ONE pass over `sessions` into every sidebar list (each keeps the input
 *  order; the channel groups are sorted newest first). */
export function bucketSessions(sessions: readonly Session[]): SessionBuckets {
  const b: SessionBuckets = {
    active: [],
    foregroundActive: [],
    agent: [],
    scratch: [],
    connection: [],
    plainAgent: [],
    telegram: [],
    slack: [],
  };
  for (const s of sessions) {
    if (s.archived) continue;
    b.active.push(s);
    const fg = isForeground(s);
    if (fg) b.foregroundActive.push(s);
    if (s.kind === 'connection') {
      b.connection.push(s);
      continue;
    }
    if (s.kind !== 'agent') continue;
    if (s.workspace_id === SCRATCH_ID) {
      if (fg) b.scratch.push(s);
      continue;
    }
    b.agent.push(s);
    if (fg) b.plainAgent.push(s);
    const ch = (s.meta as { channel?: unknown } | null)?.channel;
    if (ch === 'telegram') b.telegram.push(s);
    else if (ch === 'slack') b.slack.push(s);
  }
  b.telegram.sort(byRecent);
  b.slack.sort(byRecent);
  return b;
}
