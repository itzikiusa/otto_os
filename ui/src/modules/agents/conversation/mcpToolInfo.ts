// Recognizes Otto session-to-session and database MCP tool calls across
// Claude Code (`mcp__otto__otto_*`), Codex, outward `otto.*`, and Antigravity
// (`call_mcp_tool` with `{ ServerName: 'otto', ToolName, Arguments }`).
import type { ToolCallBlock } from './format';

export interface PeerToolCardData {
  kind: 'peer';
  op: 'open' | 'send' | 'broadcast' | 'wait' | 'get';
  actionLabel: string;
  targetId: string | null;
  targetFallback: string;
  snippet: string | null;
}

export interface DbToolCardData {
  kind: 'db';
  op: 'query' | 'schema' | 'object' | 'children';
  actionLabel: string;
  connectionId: string | null;
  sql: string | null;
  scope: string | null;
  rowCount: number | null;
  durationMs: number | null;
}

export type RichMcpCardData = PeerToolCardData | DbToolCardData;

function asRecord(v: unknown): Record<string, unknown> | null {
  return v !== null && typeof v === 'object' && !Array.isArray(v)
    ? (v as Record<string, unknown>)
    : null;
}

function pickStr(obj: Record<string, unknown> | null, ...keys: string[]): string | null {
  if (!obj) return null;
  for (const k of keys) {
    const v = obj[k];
    if (typeof v === 'string' && v.trim()) return v.trim();
  }
  return null;
}

function pickNum(obj: Record<string, unknown> | null, ...keys: string[]): number | null {
  if (!obj) return null;
  for (const k of keys) {
    const v = obj[k];
    if (typeof v === 'number' && Number.isFinite(v)) return v;
  }
  return null;
}

function parseResultJson(text: string | null | undefined): Record<string, unknown> | null {
  if (!text) return null;
  const trimmed = text.trim();
  if (!trimmed.startsWith('{')) return null;
  try {
    return asRecord(JSON.parse(trimmed));
  } catch {
    return null;
  }
}

/** Normalize MCP tool name + unwrap `call_mcp_tool` wrapper arguments. */
export function normalizeOttoMcpCall(
  block: ToolCallBlock,
): { canonical: string; args: Record<string, unknown> | null } | null {
  const rawInput = asRecord(block.input);
  let rawName = (block.name || block.title || '').trim();
  let args = rawInput;

  // Antigravity lazy MCP wrapper: `call_mcp_tool` / `default_api:call_mcp_tool`
  if (/(?:^|:)call_mcp_tool$/i.test(rawName) && rawInput) {
    const innerName = pickStr(rawInput, 'ToolName', 'tool_name', 'name');
    if (innerName) {
      rawName = innerName;
      args = asRecord(rawInput.Arguments ?? rawInput.arguments ?? rawInput.args) ?? rawInput;
    }
  }

  const cleaned = rawName
    .toLowerCase()
    .replace(/^default_api:/, '')
    .replace(/^mcp__[^_]+__/, '')
    .replace(/^otto[.:]/, '')
    .replace(/^otto_/, '');

  return cleaned ? { canonical: cleaned, args } : null;
}

function clipSnippet(s: string | null, max = 220): string | null {
  if (!s) return null;
  const oneLine = s.replace(/\s+/g, ' ').trim();
  if (!oneLine) return null;
  return oneLine.length > max ? `${oneLine.slice(0, max - 1)}…` : oneLine;
}

/** Extract structured Peer Session or Database MCP card metadata when applicable. */
export function extractRichMcpCard(
  block: ToolCallBlock,
  resultText?: string,
): RichMcpCardData | null {
  const norm = normalizeOttoMcpCall(block);
  if (!norm) return null;
  const { canonical, args } = norm;
  const parsed = parseResultJson(resultText ?? block.result?.text);
  const nestedSession = asRecord(parsed?.session);
  const nestedStats = asRecord(parsed?.stats);

  // 1. Peer-session MCP tools
  if (
    canonical === 'open_session' ||
    canonical === 'send_message' ||
    canonical === 'broadcast_message' ||
    canonical === 'wait_session' ||
    canonical === 'get_session'
  ) {
    const op =
      canonical === 'open_session'
        ? 'open'
        : canonical === 'send_message'
          ? 'send'
          : canonical === 'broadcast_message'
            ? 'broadcast'
            : canonical === 'wait_session'
              ? 'wait'
              : 'get';

    const actionLabel =
      op === 'open'
        ? 'Peer session opened'
        : op === 'send'
          ? 'Message to peer session'
          : op === 'broadcast'
            ? 'Broadcast to sessions'
            : op === 'wait'
              ? 'Wait for peer session'
              : 'Inspect peer session';

    const targetId =
      pickStr(args, 'session_id', 'target_session_id', 'recipient', 'id') ??
      pickStr(parsed, 'session_id', 'id') ??
      pickStr(nestedSession, 'id');

    const targetFallback =
      pickStr(args, 'title', 'name', 'provider') ??
      pickStr(parsed, 'title') ??
      pickStr(nestedSession, 'title') ??
      (op === 'broadcast' ? 'Workspace sessions' : targetId ?? 'Peer session');

    const snippet = clipSnippet(
      pickStr(args, 'message', 'text', 'prompt', 'initial_prompt') ??
        pickStr(parsed, 'last_message', 'summary', 'output'),
    );

    return {
      kind: 'peer',
      op,
      actionLabel,
      targetId,
      targetFallback,
      snippet,
    };
  }

  // 2. Database MCP tools
  if (
    canonical === 'db_query' ||
    canonical === 'query_db' ||
    canonical === 'ui_db_run_query' ||
    canonical === 'db_schema' ||
    canonical === 'db_object' ||
    canonical === 'db_children'
  ) {
    const op =
      canonical === 'db_schema'
        ? 'schema'
        : canonical === 'db_object'
          ? 'object'
          : canonical === 'db_children'
            ? 'children'
            : 'query';

    const actionLabel =
      op === 'query'
        ? 'Database query'
        : op === 'schema'
          ? 'Database schema'
          : op === 'object'
            ? 'Database object'
            : 'Database browse';

    const connectionId =
      pickStr(args, 'connection_id', 'conn_id', 'connectionId', 'id') ??
      pickStr(parsed, 'connection_id');

    const sql = pickStr(args, 'sql', 'query', 'statement') ?? pickStr(parsed, 'sql', 'statement');
    const scope = pickStr(args, 'node', 'database', 'schema', 'table', 'object', 'path');

    const rowsArr = Array.isArray(parsed?.rows) ? parsed.rows : null;
    const rowCount =
      pickNum(parsed, 'row_count') ??
      pickNum(nestedStats, 'row_count') ??
      (rowsArr ? rowsArr.length : null);

    const durationMs =
      pickNum(parsed, 'duration_ms') ?? pickNum(nestedStats, 'duration_ms');

    return {
      kind: 'db',
      op,
      actionLabel,
      connectionId,
      sql,
      scope,
      rowCount,
      durationMs,
    };
  }

  return null;
}
